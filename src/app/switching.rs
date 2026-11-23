use super::parking::RESTORE_CONFIRM_TOLERANCE;
use super::*;

const REVEAL_SETTLE_LIMIT: Duration = Duration::from_millis(300);
/// Poll interval while waiting for incoming windows to arrive.
const REVEAL_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// How long a pending reveal waits before the restore write is re-issued.
const REVEAL_REWRITE_INTERVAL: Duration = Duration::from_millis(100);

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    pub(super) fn switch(
        &mut self,
        workspace: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.synchronize_foreground(&outputs);
        let selector = self.ensure_managed_workspace(workspace)?;
        let output = output_for(&self.workspaces, workspace, &outputs);
        let display_id = outputs.resolve(&selector).id.clone();
        let previous = self
            .visible_workspaces(&outputs)
            .into_iter()
            .find(|resolved| resolved.display_id == display_id)
            .map_or(workspace, |resolved| resolved.workspace);
        if previous == workspace {
            let focused = self.backend.focused_window();
            let windows = self.workspaces.windows(workspace);
            if self.workspaces.uses_fullscreen_layout(workspace)
                && let Some(focused) = focused
                    .as_ref()
                    .filter(|focused| self.workspaces.workspace_of(focused) == Some(workspace))
            {
                self.workspaces.remember_focus(workspace, focused);
                self.pending_fullscreen_focus_restore.remove(&workspace);
                self.persist_workspace_state_if_changed()?;
                let mut status = self.status(0, &outputs);
                status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, None, &outputs));
                return Ok(status);
            }
            self.focus_workspace_entry(workspace, &windows, focused);
            let mut status = self.status(0, &outputs);
            status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, None, &outputs));
            return Ok(status);
        }
        let switch_started = Instant::now();
        let mut outgoing = self.workspaces.windows(previous);
        self.order_back_to_front(&mut outgoing, |window| window.window_id);
        let incoming = self.workspaces.windows(workspace);
        log_event!(
            "switch: workspace {} -> {} (outgoing {}, incoming {})",
            previous,
            workspace,
            outgoing.len(),
            incoming.len()
        );
        let outgoing_focus = self
            .backend
            .focused_window()
            .filter(|focused| outgoing.contains(focused));
        if let Some(focused) = outgoing_focus.as_ref() {
            self.workspaces.remember_focus(previous, focused);
        }
        // Reveal incoming windows first: position writes do not raise, so they
        // arrive and get laid out beneath the outgoing windows. Hiding the
        // outgoing workspace afterwards exposes a fully formed workspace and
        // the bare desktop never shows. Failures before parking leave the
        // screen untouched.
        let incoming_originals: HashMap<_, _> = incoming
            .iter()
            .filter_map(|window| {
                self.parked
                    .get(window)
                    .copied()
                    .map(|frame| (window.clone(), frame))
            })
            .collect();
        let mut restored = Vec::new();
        for window in &incoming {
            if cancelled.load(Ordering::Acquire) {
                log_event!("switch cancelled while revealing incoming windows");
                self.rollback_switch(Vec::new(), &restored, &incoming_originals, output, &outputs);
                return Err("workspace switch cancelled".into());
            }
            if !self.parked.contains_key(window) {
                continue;
            }
            restored.push(window.clone());
            match self.restore_window(window) {
                macos::ApplyOutcome::Applied => {}
                macos::ApplyOutcome::Gone => {
                    self.forget_window(workspace, window);
                }
                outcome => {
                    log_event!(
                        "switch rollback: could not restore {:?} ({:?})",
                        window,
                        outcome
                    );
                    self.rollback_switch(
                        Vec::new(),
                        &restored,
                        &incoming_originals,
                        output,
                        &outputs,
                    );
                    return Err(format!(
                        "Workspace {} could not restore all of its windows",
                        workspace
                    ));
                }
            }
        }
        if !self.wait_for_reveal(&incoming, cancelled) {
            self.rollback_switch(Vec::new(), &restored, &incoming_originals, output, &outputs);
            if cancelled.load(Ordering::Acquire) {
                return Err("workspace switch cancelled".into());
            }
            return Err(format!(
                "Workspace {workspace} stayed inactive because its windows did not finish restoring"
            ));
        }
        let layout = self.layout_workspace(workspace, output, cancelled);
        if cancelled.load(Ordering::Acquire) {
            log_event!("switch cancelled while laying out incoming windows");
            self.rollback_switch(Vec::new(), &incoming, &incoming_originals, output, &outputs);
            return Err("workspace switch cancelled".into());
        }
        if layout.blocked {
            log_event!("switch rollback: workspace {} layout blocked", workspace);
            self.rollback_switch(Vec::new(), &incoming, &incoming_originals, output, &outputs);
            return Err(format!(
                "Workspace {} stayed inactive because its windows could not be laid out",
                workspace
            ));
        }
        let mut parked_outgoing = Vec::new();
        for window in &outgoing {
            if cancelled.load(Ordering::Acquire) {
                log_event!("switch cancelled while parking outgoing windows");
                self.rollback_switch(
                    parked_outgoing,
                    &incoming,
                    &incoming_originals,
                    output,
                    &outputs,
                );
                return Err("workspace switch cancelled".into());
            }
            match self.park_window(window, output, &outputs) {
                macos::ApplyOutcome::Applied => parked_outgoing.push(window.clone()),
                macos::ApplyOutcome::Gone => {
                    self.forget_window(previous, window);
                }
                outcome => {
                    log_event!(
                        "switch rollback: could not park {:?} ({:?})",
                        window,
                        outcome
                    );
                    self.rollback_switch(
                        parked_outgoing,
                        &incoming,
                        &incoming_originals,
                        output,
                        &outputs,
                    );
                    return Err(format!(
                        "Workspace {} stayed active because a window could not be parked",
                        previous
                    ));
                }
            }
        }
        self.workspaces.switch(workspace);
        self.foreground_lanes.insert(display_id, selector);
        self.focus_workspace_entry(workspace, &incoming, outgoing_focus);
        self.persist_workspace_state_if_changed()?;
        log_event!(
            "switch complete: workspace {} in {}ms",
            workspace,
            switch_started.elapsed().as_millis()
        );
        let mut status = self.status(layout.failed, &outputs);
        status.hud = Some(self.hud_for(workspace, UiHudKind::Workspace, Some(previous), &outputs));
        Ok(status)
    }

    pub(super) fn focus_workspace_entry(
        &mut self,
        workspace: WorkspaceId,
        windows: &[crate::PhysicalWindowId],
        fallback: Option<crate::PhysicalWindowId>,
    ) {
        let workspace_focus = self
            .workspaces
            .preferred_focus(workspace)
            .filter(|window| windows.contains(window))
            .or_else(|| windows.first().cloned());
        let Some(window) = workspace_focus else {
            self.suppressed_focus = fallback;
            return;
        };
        if self.backend.focus_window(&window) {
            self.pending_fullscreen_focus_restore.remove(&workspace);
            self.suppressed_focus = None;
        } else {
            self.suppressed_focus = fallback;
        }
    }

    /// Bounded wait until every incoming window's restore is confirmed by the
    /// window server (pending restores keep their recovery frames). Slow
    /// applications apply moves asynchronously; hiding the outgoing workspace
    /// before they arrive would flash the bare desktop. Polling is
    /// confirmation-only; the restore write is re-issued only for windows
    /// that still have not arrived after [`REVEAL_REWRITE_INTERVAL`].
    pub(super) fn wait_for_reveal(
        &mut self,
        incoming: &[crate::PhysicalWindowId],
        cancelled: &AtomicBool,
    ) -> bool {
        let started = Instant::now();
        let mut last_rewrite = started;
        loop {
            let pending: Vec<_> = incoming
                .iter()
                .filter(|window| self.parked.contains_key(window))
                .cloned()
                .collect();
            if pending.is_empty() {
                return !cancelled.load(Ordering::Acquire);
            }
            if started.elapsed() >= REVEAL_SETTLE_LIMIT || cancelled.load(Ordering::Acquire) {
                log_event!(
                    "reveal interrupted: {} window(s) still arriving; keeping outgoing workspace",
                    pending.len()
                );
                return false;
            }
            thread::sleep(REVEAL_POLL_INTERVAL);
            let Some(frames) = self.backend.window_server_frames() else {
                continue;
            };
            let mut removed = Vec::new();
            for window in &pending {
                let Some(original) = self.parked.get(window).copied() else {
                    continue;
                };
                if frames.get(&window.window_id).is_some_and(|actual| {
                    actual.position_approximately_equals(original, RESTORE_CONFIRM_TOLERANCE)
                }) {
                    self.commit_unparked(window);
                    removed.push((window.clone(), original));
                }
            }
            if last_rewrite.elapsed() >= REVEAL_REWRITE_INTERVAL {
                last_rewrite = Instant::now();
                for window in &pending {
                    let Some(frame) = self.parked.get(window).copied() else {
                        continue;
                    };
                    if matches!(
                        self.write_restore_position(window, frame),
                        macos::ApplyOutcome::Gone
                    ) {
                        removed.push((window.clone(), frame));
                    }
                }
            }
            self.persist_removals_or_rollback(removed);
        }
    }

    /// Park the window furthest back first. Hiding the front window first
    /// uncovers the windows stacked beneath it (every Fullscreen Layout tile
    /// shares one frame) until they are parked too, which reads as a flash.
    pub(super) fn order_back_to_front<T>(
        &mut self,
        items: &mut [T],
        window_id: impl Fn(&T) -> u32,
    ) {
        if items.len() < 2 {
            return;
        }
        let Some(order) = self.backend.onscreen_window_order() else {
            return;
        };
        let rank: HashMap<u32, usize> = order
            .iter()
            .enumerate()
            .map(|(rank, id)| (*id, rank))
            .collect();
        // Off-screen windows uncover nothing; they sort first.
        items.sort_by_key(|item| {
            std::cmp::Reverse(rank.get(&window_id(item)).copied().unwrap_or(usize::MAX))
        });
    }

    /// Undo a partially applied switch: bring the outgoing windows back and
    /// hide any incoming windows that were already revealed.
    fn rollback_switch(
        &mut self,
        parked_outgoing: Vec<crate::PhysicalWindowId>,
        revealed: &[crate::PhysicalWindowId],
        incoming_originals: &HashMap<crate::PhysicalWindowId, crate::Rect>,
        output: crate::Rect,
        outputs: &macos::DisplayOutputs,
    ) {
        self.suppressed_focus = self.backend.focused_window();
        for window in parked_outgoing {
            self.restore_window(&window);
        }
        for window in revealed {
            if let Some(&original) = incoming_originals.get(window)
                && !self.mark_parked(window.clone(), original)
            {
                log_event!("rollback mark_parked failed for {:?}", window);
            }
            // A pending restore may land after rollback. Supersede that write
            // even when its recovery entry has not yet been retired.
            let _: macos::ApplyOutcome = self.park_window(window, output, outputs);
        }
    }

    pub(super) fn workspace_back_and_forth(
        &mut self,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let focused = self
            .backend
            .focused_window()
            .ok_or("Focused window is unavailable")?;
        let workspace = self
            .workspaces
            .workspace_of_any(&focused)
            .ok_or("Focused window is not managed")?;
        let selector = self.workspaces.assignment(workspace);
        let previous = self
            .workspaces
            .previous_for(selector)
            .ok_or("Focused display has no previous workspace")?;
        self.switch(previous, outputs, cancelled)
    }

    /// Left goes to the next workspace in the display's menu section, right to
    /// the previous one. Nothing happens past either end.
    pub(super) fn swipe(
        &mut self,
        swipe: Swipe,
        point: (i32, i32),
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.synchronize_foreground(&outputs);
        let display_id = outputs
            .displays()
            .iter()
            .find(|display| display.frame.contains(point.0, point.1))
            .unwrap_or_else(|| outputs.main())
            .id
            .clone();
        let current = self
            .visible_workspaces(&outputs)
            .into_iter()
            .find(|resolved| resolved.display_id == display_id)
            .map(|resolved| resolved.workspace);
        let listed = self.listed_workspaces(&display_id, current, &outputs);
        let target = listed
            .iter()
            .position(|workspace| Some(*workspace) == current)
            .and_then(|index| match swipe {
                Swipe::Left => listed.get(index + 1),
                Swipe::Right => index.checked_sub(1).and_then(|index| listed.get(index)),
            });
        match target {
            Some(&target) => self.switch(target, outputs, cancelled),
            None => Ok(self.status(0, &outputs)),
        }
    }

    pub(super) fn move_focused(
        &mut self,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let window = self
            .backend
            .focused_window()
            .ok_or("Focused window is unavailable")?;
        self.move_window(window, target, outputs, cancelled)
    }

    pub(super) fn move_app(
        &mut self,
        bundle_id: &str,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        let mut windows = self.windows_for_bundle(bundle_id);
        if windows.is_empty() {
            self.cycle(outputs.clone(), cancelled)?;
            windows = self.windows_for_bundle(bundle_id);
        }
        match windows.as_slice() {
            [] => Err(format!("No managed window found for {bundle_id}")),
            [window] => self.move_window(window.clone(), target, outputs, cancelled),
            _ => Err(format!(
                "{bundle_id} has multiple managed windows; use move on the focused window"
            )),
        }
    }

    fn windows_for_bundle(&self, bundle_id: &str) -> Vec<crate::PhysicalWindowId> {
        WorkspaceId::all()
            .flat_map(|workspace| self.workspaces.engine(workspace).bound_windows())
            .filter(|window| self.bundle_id(&window.app) == Some(bundle_id))
            .collect()
    }

    pub(super) fn move_window(
        &mut self,
        window: crate::PhysicalWindowId,
        target: WorkspaceId,
        outputs: macos::DisplayOutputs,
        cancelled: &AtomicBool,
    ) -> Result<CycleStatus, String> {
        self.ensure_managed_workspace(target)?;
        let source = if let Some(source) = self.workspaces.workspace_of_any(&window) {
            source
        } else {
            self.cycle(outputs.clone(), cancelled)?;
            self.workspaces
                .workspace_of_any(&window)
                .ok_or("Window is not managed by Spaceman")?
        };
        if source == target {
            return Ok(self.status(0, &outputs));
        }
        let is_floating = self.workspaces.floating_workspace_of(&window).is_some();
        let source_output = output_for(&self.workspaces, source, &outputs);
        let target_output = output_for(&self.workspaces, target, &outputs);
        let target_is_active = self.is_visible(target, &outputs);
        if !target_is_active
            && !matches!(
                self.park_window(&window, target_output, &outputs),
                macos::ApplyOutcome::Applied
            )
        {
            return Err("Window could not be parked in the target workspace".into());
        }
        if is_floating {
            self.workspaces.own_floating(window.clone(), target);
        } else {
            self.workspaces
                .move_window(&window, target)
                .ok_or("Window could not be moved")?;
            self.exit_fullscreen_if_split(target);
        }
        self.clear_fullscreen_restore_if_inactive(source);
        self.clear_relocation_evidence(source, target, &window);
        self.note_hand_move(&window, source, target);
        if !target_is_active {
            self.suppressed_focus = Some(window.clone());
        }
        let mut failed = 0;
        if !is_floating && self.is_visible(source, &outputs) {
            let layout = self.layout_workspace(source, source_output, cancelled);
            failed += layout.failed;
        }
        if target_is_active {
            if self.parked.contains_key(&window)
                && !matches!(
                    self.restore_window(&window),
                    macos::ApplyOutcome::Applied | macos::ApplyOutcome::Gone
                )
            {
                failed += 1;
            }
            if !is_floating {
                let layout = self.layout_workspace(target, target_output, cancelled);
                failed += layout.failed;
            }
            if !self.backend.focus_window(&window) {
                log_event!("move focus follow failed for {:?}", window);
            }
        }
        self.persist_workspace_state_if_changed()?;
        let mut status = self.status(failed, &outputs);
        status.hud = Some(self.hud_for(target, UiHudKind::Moved, None, &outputs));
        Ok(status)
    }
}
