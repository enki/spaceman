use super::*;

pub(super) struct CycleStatus {
    pub(super) active: Vec<WorkspaceId>,
    pub(super) layouts: Vec<status_item::WorkspaceLayout>,
    pub(super) menu: Vec<status_item::DisplayWorkspaces>,
    pub(super) failed: usize,
    pub(super) hud: Option<UiHudEvent>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct RuntimeDebug {
    frontmost: Option<AppInstanceId>,
    focused: Option<crate::PhysicalWindowId>,
    active_workspaces: Vec<WorkspaceId>,
    displays: Vec<DebugDisplay>,
    tiles: Vec<NavigationTile>,
    floating: Vec<DebugFloatingWindow>,
    /// Managed windows left visible across workspace switches because they
    /// repeatedly reject parking positions.
    parking_exempt: Vec<crate::PhysicalWindowId>,
    targets: DebugTargets,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DebugFloatingWindow {
    window: crate::PhysicalWindowId,
    workspace: WorkspaceId,
    /// Present exactly when the window is parked off-screen.
    parked_original_frame: Option<crate::Rect>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DebugDisplay {
    id: String,
    name: String,
    frame: crate::Rect,
    workspace: WorkspaceId,
    axis: crate::Axis,
    fullscreen_layout: bool,
    layout: Vec<crate::Rect>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct DebugTargets {
    left: Option<crate::PhysicalWindowId>,
    down: Option<crate::PhysicalWindowId>,
    up: Option<crate::PhysicalWindowId>,
    right: Option<crate::PhysicalWindowId>,
}

pub(super) struct LayoutStatus {
    pub(super) failed: usize,
    pub(super) blocked: bool,
}

#[derive(Clone)]
pub(super) struct ResolvedWorkspace {
    pub(super) workspace: WorkspaceId,
    pub(super) selector: DisplaySelector,
    pub(super) display_id: String,
    pub(super) output: crate::Rect,
}

pub(super) struct WorkspaceRuntime<B = MacWorkspaceBackend> {
    pub(super) backend: B,
    pub(super) scope: ManagementScope,
    pub(super) workspaces: Workspaces,
    pub(super) known_apps: [HashMap<AppInstanceId, String>; WorkspaceId::COUNT],
    pub(super) constrained:
        [HashMap<crate::PhysicalWindowId, (crate::Rect, crate::Rect)>; WorkspaceId::COUNT],
    /// Target frames reported as `Applied` on the latest layout pass, awaiting
    /// observation readback. Electron apps accept `AXSize` writes synchronously
    /// and clamp to their aspect-ratio or size limits asynchronously afterward.
    pub(super) applied: [HashMap<crate::PhysicalWindowId, crate::Rect>; WorkspaceId::COUNT],
    /// Consecutive bounced or rejected applications per window for the same
    /// target frame; past `APPLY_ATTEMPT_LIMIT` the target is frozen.
    pub(super) apply_attempts:
        [HashMap<crate::PhysicalWindowId, (crate::Rect, u8)>; WorkspaceId::COUNT],
    pub(super) parked: HashMap<crate::PhysicalWindowId, crate::Rect>,
    /// When each window was last parked. Apps apply position writes
    /// asynchronously, so a fresh park must settle before an on-screen
    /// observation may be trusted as a genuine reappearance.
    pub(super) recently_parked: HashMap<crate::PhysicalWindowId, Instant>,
    /// When each window last un-parked itself (confirmed by the window server).
    pub(super) recent_heals: HashMap<crate::PhysicalWindowId, Instant>,
    /// Consecutive re-parks undone by the app; past the limit the window's
    /// reappearance is honored instead of fought.
    pub(super) repark_attempts: HashMap<crate::PhysicalWindowId, u8>,
    /// Windows that repeatedly un-park themselves. They remain managed and
    /// tiled, but are left visible across workspace switches for the session.
    pub(super) parking_exempt: HashSet<crate::PhysicalWindowId>,
    /// Bound windows the window server shows on screen while the app's
    /// Accessibility snapshot omits them, and when the omission began.
    pub(super) omitted_windows: HashMap<crate::PhysicalWindowId, Instant>,
    /// Owned windows Accessibility reports gone while the window server still
    /// lists them, and when that began.
    pub(super) inaccessible_since: HashMap<crate::PhysicalWindowId, Instant>,
    /// Smallest width and height each window has refused to shrink below.
    pub(super) minimum_sizes: HashMap<crate::PhysicalWindowId, (i32, i32)>,
    pub(super) config: Config,
    pub(super) foreground_lanes: HashMap<String, DisplaySelector>,
    pub(super) suppressed_focus: Option<crate::PhysicalWindowId>,
    /// The window a focus command just focused, until Accessibility reports it.
    pub(super) pending_focus: Option<super::commands::PendingFocus>,
    pub(super) pending_fullscreen_focus_restore: HashSet<WorkspaceId>,
    pub(super) persisted_workspace_state: crate::workspace::WorkspaceState,
    /// Title-free placements that survive application relaunches and reboots.
    pub(super) memory: MemoryState,
    pub(super) journal: JournalState,
}

impl WorkspaceRuntime<MacWorkspaceBackend> {
    pub(super) fn new(
        scope: ManagementScope,
        parked: HashMap<crate::PhysicalWindowId, crate::Rect>,
        config: Config,
        workspace_state: Option<crate::workspace::WorkspaceState>,
    ) -> Self {
        Self::with_backend_config_state(
            scope,
            MacWorkspaceBackend::new(),
            parked,
            config,
            workspace_state,
        )
    }
}

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    /// Rules apply to windows from now on; named workspaces and display assignments update.
    /// New named workspaces get their hotkeys at the next restart.
    pub(super) fn replace_config(&mut self, config: Config) {
        config.configure_workspaces(&mut self.workspaces);
        self.config = config;
    }

    pub(super) fn execute(
        &mut self,
        command: WorkerCommand,
        cancelled: &AtomicBool,
    ) -> Option<WorkerResult> {
        if let WorkerCommand::Action(_, outputs) | WorkerCommand::Gesture(_, _, outputs) = &command
        {
            self.synchronize_foreground(outputs);
        }
        let (result, outputs) = match command {
            WorkerCommand::Action(action, outputs) => {
                let result = self.execute_action(action, outputs.clone(), cancelled);
                (result, outputs)
            }
            WorkerCommand::Gesture(gesture, preview, outputs) => {
                let window = match &gesture {
                    FinalGesture::Drop(window, _) | FinalGesture::Resize(window, _) => window,
                };
                for attempts in &mut self.apply_attempts {
                    attempts.remove(window);
                }
                let result = match gesture {
                    FinalGesture::Drop(window, point) => {
                        self.drop_window(window, point, preview, outputs.clone(), cancelled)
                    }
                    FinalGesture::Resize(window, frame) => {
                        self.resize_window(window, frame, preview, outputs.clone(), cancelled)
                    }
                };
                (result, outputs)
            }
            WorkerCommand::RestoreAll => {
                let requested = self.parked.len();
                let failed = self.restore_all();
                self.journal_restore("restore all", requested, failed);
                // Pause, quit and restart all restore first; the layout they
                // resume from must be on disk before the process can exit.
                self.backend.flush_state();
                return Some(WorkerResult::Restored { failed });
            }
            WorkerCommand::Stop => {
                self.journal_event(crate::journal::JournalEvent::Stop);
                self.backend.flush_state();
                return None;
            }
        };
        self.check_arrangement(&outputs);
        Some(WorkerResult::Status {
            result,
            debug: Box::new(self.debug_state(&outputs)),
        })
    }

    fn execute_action(
        &mut self,
        action: WorkerAction,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        if matches!(
            action,
            WorkerAction::SwitchWorkspace(_)
                | WorkerAction::WorkspaceBackAndForth
                | WorkerAction::MoveWorkspaceToDisplay(_)
                | WorkerAction::Swipe(..)
                | WorkerAction::MoveFocused(_)
                | WorkerAction::MoveApp(..)
        ) {
            self.pending_focus = None;
        }
        match action {
            WorkerAction::Retile => self.cycle(outputs, cancelled),
            WorkerAction::SwitchWorkspace(workspace) => self.switch(workspace, outputs, cancelled),
            WorkerAction::WorkspaceBackAndForth => {
                self.workspace_back_and_forth(outputs, cancelled)
            }
            WorkerAction::MoveWorkspaceToDisplay(target) => {
                self.move_workspace_to_display(target, outputs, cancelled)
            }
            WorkerAction::Swipe(swipe, point) => self.swipe(swipe, point, outputs, cancelled),
            WorkerAction::MoveFocused(workspace) => {
                self.move_focused(workspace, outputs, cancelled)
            }
            WorkerAction::MoveApp(bundle_id, workspace) => {
                self.move_app(&bundle_id, workspace, outputs, cancelled)
            }
            WorkerAction::CycleFocus => self.cycle_focus(outputs, cancelled),
            WorkerAction::Focus(direction) => self.focus_relative(direction, outputs, cancelled),
            WorkerAction::Reorder(direction) => self.reorder_focused(direction, outputs, cancelled),
            WorkerAction::ToggleLayout => self.toggle_layout(outputs, cancelled),
            WorkerAction::Resize(direction) => self.resize_focused(direction, outputs, cancelled),
            WorkerAction::Balance => self.balance_focused(outputs, cancelled),
            WorkerAction::ToggleFullscreenLayout => {
                self.toggle_fullscreen_layout(outputs, cancelled)
            }
            WorkerAction::MarkWrong => {
                self.mark_wrong();
                Ok(self.status(0, &outputs))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn with_backend(
        scope: ManagementScope,
        backend: B,
        parked: HashMap<crate::PhysicalWindowId, crate::Rect>,
    ) -> Self {
        Self::with_backend_and_config(scope, backend, parked, Config::default())
    }

    #[cfg(test)]
    pub(super) fn with_backend_and_config(
        scope: ManagementScope,
        backend: B,
        parked: HashMap<crate::PhysicalWindowId, crate::Rect>,
        config: Config,
    ) -> Self {
        Self::with_backend_config_state(scope, backend, parked, config, None)
    }

    pub(super) fn with_backend_config_state(
        scope: ManagementScope,
        backend: B,
        parked: HashMap<crate::PhysicalWindowId, crate::Rect>,
        config: Config,
        workspace_state: Option<crate::workspace::WorkspaceState>,
    ) -> Self {
        let restored_workspace_state = workspace_state.is_some();
        let mut workspaces = Workspaces::default();
        config.configure_workspaces(&mut workspaces);
        if let Some(state) = workspace_state {
            workspaces.restore(state);
        }
        let pending_fullscreen_focus_restore = if restored_workspace_state {
            WorkspaceId::all()
                .filter(|workspace| {
                    workspaces.uses_fullscreen_layout(*workspace)
                        && scope.manages_selector(workspaces.assignment(*workspace))
                })
                .collect()
        } else {
            HashSet::new()
        };
        let known_apps = std::array::from_fn(|index| {
            workspaces
                .app_instances(WorkspaceId::from_index(index))
                .map(|app| (app.clone(), String::new()))
                .collect()
        });
        let persisted_workspace_state = workspaces.state();
        Self {
            backend,
            scope,
            workspaces,
            known_apps,
            constrained: std::array::from_fn(|_| HashMap::new()),
            applied: std::array::from_fn(|_| HashMap::new()),
            apply_attempts: std::array::from_fn(|_| HashMap::new()),
            parked,
            recently_parked: HashMap::new(),
            recent_heals: HashMap::new(),
            repark_attempts: HashMap::new(),
            parking_exempt: HashSet::new(),
            omitted_windows: HashMap::new(),
            inaccessible_since: HashMap::new(),
            minimum_sizes: HashMap::new(),
            config,
            foreground_lanes: HashMap::new(),
            suppressed_focus: None,
            pending_focus: None,
            pending_fullscreen_focus_restore,
            persisted_workspace_state,
            memory: MemoryState::default(),
            journal: JournalState::default(),
        }
    }

    pub(super) fn restore_window_memory(&mut self, memory: crate::window_memory::WindowMemory) {
        self.memory = MemoryState {
            memory,
            started_at: Some(Instant::now()),
            ..MemoryState::default()
        };
    }

    pub(super) fn visible_workspaces(
        &self,
        outputs: &macos::DisplayOutputs,
    ) -> Vec<ResolvedWorkspace> {
        outputs
            .displays()
            .iter()
            .filter_map(|display| {
                let candidates = || {
                    self.workspaces
                        .active_lanes()
                        .filter(|(selector, _)| outputs.resolve(selector).id == display.id)
                };
                let selected = self
                    .foreground_lanes
                    .get(&display.id)
                    .and_then(|foreground| {
                        candidates().find(|(selector, _)| *selector == foreground)
                    })
                    .or_else(|| {
                        candidates()
                            .filter(|(selector, _)| selector_available(selector, outputs))
                            .min_by_key(|(_, workspace)| *workspace)
                    })
                    .or_else(|| candidates().min_by_key(|(_, workspace)| *workspace))?;
                Some(ResolvedWorkspace {
                    workspace: selected.1,
                    selector: selected.0.clone(),
                    display_id: display.id.clone(),
                    output: display.frame,
                })
            })
            .collect()
    }

    pub(super) fn is_visible(
        &self,
        workspace: WorkspaceId,
        outputs: &macos::DisplayOutputs,
    ) -> bool {
        self.visible_workspaces(outputs)
            .iter()
            .any(|resolved| resolved.workspace == workspace)
    }

    pub(super) fn synchronize_foreground(&mut self, outputs: &macos::DisplayOutputs) {
        let disconnected: Vec<_> = self
            .foreground_lanes
            .iter()
            .filter(|(display_id, _)| {
                !outputs
                    .displays()
                    .iter()
                    .any(|display| &display.id == *display_id)
            })
            .map(|(display_id, selector)| (display_id.clone(), selector.clone()))
            .collect();
        for (display_id, selector) in disconnected {
            self.foreground_lanes.remove(&display_id);
            self.foreground_lanes
                .insert(outputs.resolve(&selector).id.clone(), selector);
        }
    }

    pub(super) fn bundle_id(&self, app: &AppInstanceId) -> Option<&str> {
        self.memory
            .bundles
            .get(app)
            .or_else(|| {
                self.known_apps
                    .iter()
                    .find_map(|apps| apps.get(app))
                    .filter(|bundle| !bundle.is_empty())
            })
            .map(String::as_str)
    }

    pub(super) fn ensure_managed_workspace(
        &self,
        workspace: WorkspaceId,
    ) -> Result<DisplaySelector, String> {
        if !self.workspaces.is_enabled(workspace) {
            return Err(format!("Workspace {workspace} is not configured"));
        }
        let selector = self.workspaces.assignment(workspace).clone();
        if !self.scope.manages_selector(&selector) {
            return Err(format!(
                "Workspace {} is outside the managed display scope",
                workspace
            ));
        }
        Ok(selector)
    }

    pub(super) fn exit_fullscreen_if_split(&mut self, workspace: WorkspaceId) {
        if !self.workspaces.uses_fullscreen_layout(workspace) {
            return;
        }
        let bound_count = self
            .workspaces
            .engine(workspace)
            .model()
            .workspace
            .slots
            .iter()
            .filter(|slot| slot.bound_window().is_some())
            .count();
        if bound_count > 1 {
            log_event!("workspace {workspace}: exited Fullscreen Layout ({bound_count} tiles)");
            self.workspaces.set_fullscreen_layout(workspace, false);
            self.pending_fullscreen_focus_restore.remove(&workspace);
        }
    }

    pub(super) fn spatial_target(
        &self,
        focused: &crate::PhysicalWindowId,
        direction: Direction,
        outputs: &macos::DisplayOutputs,
    ) -> Option<(WorkspaceId, crate::PhysicalWindowId)> {
        let tiles = self.navigation_tiles(outputs);
        crate::navigation::target(&tiles, focused, direction)
            .map(|target| (target.workspace, target.window.clone()))
    }

    pub(super) fn navigation_tiles(&self, outputs: &macos::DisplayOutputs) -> Vec<NavigationTile> {
        self.navigation_tiles_for(&self.visible_workspaces(outputs))
    }

    pub(super) fn navigation_tiles_for(
        &self,
        visible: &[ResolvedWorkspace],
    ) -> Vec<NavigationTile> {
        let mut tiles = Vec::new();
        for resolved in visible {
            let mut placements = self
                .workspaces
                .engine(resolved.workspace)
                .placements(&Settings::for_output(resolved.output));
            if self.workspaces.uses_fullscreen_layout(resolved.workspace) {
                for placement in &mut placements {
                    placement.frame = resolved.output;
                }
            }
            tiles.extend(placements.into_iter().filter_map(|placement| {
                placement.window.map(|window| NavigationTile {
                    workspace: resolved.workspace,
                    window,
                    frame: placement.frame,
                })
            }));
        }
        tiles
    }

    pub(super) fn debug_state(&mut self, outputs: &macos::DisplayOutputs) -> RuntimeDebug {
        let visible = self.visible_workspaces(outputs);
        let tiles = self.navigation_tiles_for(&visible);
        let focused = self.backend.focused_window();
        let target = |direction| {
            focused.as_ref().and_then(|focused| {
                crate::navigation::target(&tiles, focused, direction)
                    .map(|tile| tile.window.clone())
            })
        };
        let targets = DebugTargets {
            left: target(Direction::Left),
            down: target(Direction::Down),
            up: target(Direction::Up),
            right: target(Direction::Right),
        };
        let floating = self
            .workspaces
            .floating_entries()
            .map(|(window, workspace)| DebugFloatingWindow {
                parked_original_frame: self.parked.get(window).copied(),
                window: window.clone(),
                workspace,
            })
            .collect();
        RuntimeDebug {
            frontmost: self.backend.frontmost_application(),
            focused,
            active_workspaces: visible.iter().map(|resolved| resolved.workspace).collect(),
            displays: visible
                .into_iter()
                .filter_map(|resolved| {
                    let display = outputs
                        .displays()
                        .iter()
                        .find(|display| display.id == resolved.display_id)?;
                    Some(DebugDisplay {
                        id: display.id.clone(),
                        name: display.name.clone(),
                        frame: display.frame,
                        workspace: resolved.workspace,
                        axis: self.workspaces.engine(resolved.workspace).root_axis(),
                        fullscreen_layout: self
                            .workspaces
                            .uses_fullscreen_layout(resolved.workspace),
                        layout: self.preview_layout(resolved.workspace, HUD_PREVIEW_RECT),
                    })
                })
                .collect(),
            tiles,
            floating,
            parking_exempt: {
                let mut windows: Vec<_> = self.parking_exempt.iter().cloned().collect();
                windows.sort_by_key(|window| (window.app.pid, window.window_id));
                windows
            },
            targets,
        }
    }

    pub(super) fn layout_workspace(
        &mut self,
        workspace: WorkspaceId,
        output: crate::Rect,
        cancelled: &AtomicBool,
    ) -> LayoutStatus {
        self.layout_workspace_during_drag(workspace, output, None, cancelled)
    }

    pub(super) fn layout_workspace_during_drag(
        &mut self,
        workspace: WorkspaceId,
        output: crate::Rect,
        dragged: Option<&crate::PhysicalWindowId>,
        cancelled: &AtomicBool,
    ) -> LayoutStatus {
        let (status, results) = self.apply_layout(workspace, output, dragged, cancelled);
        if dragged.is_some() {
            return status;
        }
        self.refit_minimum_sizes(workspace, output, results, cancelled)
            .unwrap_or(status)
    }

    pub(super) fn apply_layout(
        &mut self,
        workspace: WorkspaceId,
        output: crate::Rect,
        dragged: Option<&crate::PhysicalWindowId>,
        cancelled: &AtomicBool,
    ) -> (LayoutStatus, Vec<macos::ApplyResult>) {
        let settings = Settings::for_output(output);
        let mut placements = self.workspaces.engine(workspace).placements(&settings);
        self.apply_fullscreen_layout(workspace, &mut placements, output);
        let attempts = &mut self.apply_attempts[workspace.index()];
        for placement in &placements {
            if let Some(window) = placement.window.as_ref() {
                attempts.remove(window);
            }
        }
        let results = match dragged {
            Some(dragged) => {
                self.backend
                    .apply_managed_during_drag(&placements, &settings, dragged, cancelled)
            }
            None => self
                .backend
                .apply_managed(&placements, &settings, cancelled),
        };
        // Managed layout runs without a fresh observation; the next cycle validates readback.
        self.record_results(workspace, &results);
        let failed = failed_results(&results);
        let expected = placements
            .iter()
            .filter(|placement| placement.window.is_some())
            .count();
        // Deferred means a drag is in progress and layout retries next cycle;
        // it must not be treated as a failure that rolls back a switch.
        let blocked = results.len() != expected
            || results.iter().any(|result| {
                matches!(
                    result.outcome,
                    macos::ApplyOutcome::Gone | macos::ApplyOutcome::Unavailable
                )
            });
        (LayoutStatus { failed, blocked }, results)
    }

    pub(super) fn forget_window(
        &mut self,
        workspace: WorkspaceId,
        window: &crate::PhysicalWindowId,
    ) {
        self.workspaces.forget_window(workspace, window);
        self.clear_fullscreen_restore_if_inactive(workspace);
        self.workspaces.forget_floating(window);
        self.constrained[workspace.index()].remove(window);
        self.applied[workspace.index()].remove(window);
        self.apply_attempts[workspace.index()].remove(window);
        self.omitted_windows.remove(window);
        self.recently_parked.remove(window);
        self.recent_heals.remove(window);
        self.repark_attempts.remove(window);
        self.inaccessible_since.remove(window);
        self.minimum_sizes.remove(window);
    }

    pub(super) fn clear_fullscreen_restore_if_inactive(&mut self, workspace: WorkspaceId) {
        if !self.workspaces.uses_fullscreen_layout(workspace) {
            self.pending_fullscreen_focus_restore.remove(&workspace);
        }
    }

    pub(super) fn apply_fullscreen_layout(
        &mut self,
        workspace: WorkspaceId,
        placements: &mut [crate::Placement],
        output: crate::Rect,
    ) {
        if !self.workspaces.uses_fullscreen_layout(workspace) {
            return;
        }
        for placement in placements {
            placement.frame = output;
            self.workspaces
                .engine_mut(workspace)
                .record_placement_target(placement.slot, output);
        }
    }

    pub(super) fn record_results(
        &mut self,
        workspace: WorkspaceId,
        results: &[macos::ApplyResult],
    ) {
        let constrained = &mut self.constrained[workspace.index()];
        let applied = &mut self.applied[workspace.index()];
        let attempts = &mut self.apply_attempts[workspace.index()];
        for result in results {
            match (result.outcome, result.actual) {
                (macos::ApplyOutcome::Constrained, Some(actual)) => {
                    applied.remove(&result.window);
                    constrained.insert(result.window.clone(), (result.target, actual));
                    cycle::record_apply_attempt(attempts, workspace, &result.window, result.target);
                }
                (macos::ApplyOutcome::Applied, _) => {
                    constrained.remove(&result.window);
                    if attempts
                        .get(&result.window)
                        .is_some_and(|(target, _)| *target != result.target)
                    {
                        attempts.remove(&result.window);
                    }
                    applied.insert(result.window.clone(), result.target);
                }
                (macos::ApplyOutcome::Gone, _) => {
                    applied.remove(&result.window);
                    constrained.remove(&result.window);
                    attempts.remove(&result.window);
                }
                (macos::ApplyOutcome::Deferred, _) => {
                    applied.remove(&result.window);
                }
                _ => {
                    applied.remove(&result.window);
                    cycle::record_apply_attempt(attempts, workspace, &result.window, result.target);
                }
            }
        }
    }

    pub(super) fn status(&self, failed: usize, outputs: &macos::DisplayOutputs) -> CycleStatus {
        self.status_for_visible(failed, &self.visible_workspaces(outputs), outputs)
    }

    pub(super) fn persist_workspace_state_if_changed(&mut self) -> Result<(), String> {
        let state = self.workspaces.state();
        if state == self.persisted_workspace_state {
            return Ok(());
        }
        self.backend.persist_workspace_state(&state)?;
        self.persisted_workspace_state = state;
        Ok(())
    }

    fn preview_layout(&self, workspace: WorkspaceId, output: crate::Rect) -> Vec<crate::Rect> {
        let mut layout = self.workspaces.engine(workspace).preview(output);
        if self.workspaces.uses_fullscreen_layout(workspace) {
            layout.fill(output);
        }
        layout
    }

    pub(super) fn status_for_visible(
        &self,
        failed: usize,
        visible: &[ResolvedWorkspace],
        outputs: &macos::DisplayOutputs,
    ) -> CycleStatus {
        let active: Vec<_> = visible
            .iter()
            .filter(|resolved| self.scope.manages_selector(&resolved.selector))
            .map(|resolved| resolved.workspace)
            .collect();
        CycleStatus {
            layouts: active
                .iter()
                .map(|workspace| status_item::WorkspaceLayout {
                    workspace: workspace.to_string(),
                    mode: self.workspace_preview(*workspace),
                })
                .collect(),
            menu: self.workspace_menu(visible, outputs),
            active,
            failed,
            hud: None,
        }
    }

    /// A display's visible workspace and every other one holding windows, in order:
    /// the rows of its menu section and the stops of a swipe.
    pub(super) fn listed_workspaces(
        &self,
        display_id: &str,
        current: Option<WorkspaceId>,
        outputs: &macos::DisplayOutputs,
    ) -> Vec<WorkspaceId> {
        WorkspaceId::all()
            .filter(|workspace| {
                let selector = self.workspaces.assignment(*workspace);
                self.workspaces.is_enabled(*workspace)
                    && self.scope.manages_selector(selector)
                    && outputs.resolve(selector).id == display_id
                    && (current == Some(*workspace)
                        || !self.workspaces.windows(*workspace).is_empty())
            })
            .collect()
    }

    fn workspace_preview(&self, workspace: WorkspaceId) -> UiHudLayout {
        if self.workspaces.uses_fullscreen_layout(workspace) {
            UiHudLayout::Fullscreen
        } else {
            UiHudLayout::Split(self.preview_layout(workspace, HUD_PREVIEW_RECT))
        }
    }
}

pub(super) fn output_for(
    workspaces: &Workspaces,
    workspace: WorkspaceId,
    outputs: &macos::DisplayOutputs,
) -> crate::Rect {
    outputs.resolve(workspaces.assignment(workspace)).frame
}

fn selector_available(selector: &DisplaySelector, outputs: &macos::DisplayOutputs) -> bool {
    outputs.resolve_exact(selector).is_some()
}

pub(super) fn failed_results(results: &[macos::ApplyResult]) -> usize {
    results
        .iter()
        .filter(|result| result.outcome != macos::ApplyOutcome::Applied)
        .count()
}
