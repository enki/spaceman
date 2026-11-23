//! What the Workspace HUD shows: each display's row of workspaces, with their layouts and
//! the apps in them. Title-free: bundle IDs and preview rects only.

use super::*;

impl<B: WorkspaceBackend> WorkspaceRuntime<B> {
    /// Per display, its visible workspace and every other workspace holding windows.
    pub(super) fn workspace_menu(
        &self,
        visible: &[ResolvedWorkspace],
        outputs: &macos::DisplayOutputs,
    ) -> Vec<status_item::DisplayWorkspaces> {
        outputs
            .displays()
            .iter()
            .filter_map(|display| {
                let current = visible
                    .iter()
                    .find(|resolved| resolved.display_id == display.id)
                    .map(|resolved| resolved.workspace);
                let workspaces = self
                    .listed_workspaces(&display.id, current, outputs)
                    .into_iter()
                    .map(|workspace| {
                        let mut apps = Vec::new();
                        for window in self.workspaces.windows(workspace) {
                            if !apps.contains(&window.app.pid) {
                                apps.push(window.app.pid);
                            }
                        }
                        status_item::MenuWorkspace {
                            workspace,
                            current: current == Some(workspace),
                            apps,
                        }
                    })
                    .collect::<Vec<_>>();
                (!workspaces.is_empty()).then(|| status_item::DisplayWorkspaces {
                    display: display.name.clone(),
                    workspaces,
                })
            })
            .collect()
    }

    /// The HUD for an action on `workspace`. `from` is the workspace a switch left on that
    /// workspace's display, so the HUD can start there.
    pub(super) fn hud_for(
        &self,
        workspace: WorkspaceId,
        kind: UiHudKind,
        from: Option<WorkspaceId>,
        outputs: &macos::DisplayOutputs,
    ) -> UiHudEvent {
        let visible = self.visible_workspaces(outputs);
        let displays = outputs
            .displays()
            .iter()
            .take(HUD_MAX_DISPLAYS)
            .map(|display| {
                let current = visible
                    .iter()
                    .find(|resolved| resolved.display_id == display.id)
                    .map(|resolved| resolved.workspace);
                let from = from.filter(|from| {
                    *from != workspace
                        && current == Some(workspace)
                        && outputs.resolve(self.workspaces.assignment(*from)).id == display.id
                });
                UiHudDisplay {
                    id: display.id.clone(),
                    visible: current,
                    from,
                    workspaces: self
                        .listed_workspaces(&display.id, current, outputs)
                        .into_iter()
                        .map(|workspace| self.hud_workspace(workspace))
                        .collect(),
                }
            })
            .collect();
        UiHudEvent {
            workspace,
            kind,
            displays,
        }
    }

    fn hud_workspace(&self, workspace: WorkspaceId) -> UiHudWorkspace {
        let fullscreen = self.workspaces.uses_fullscreen_layout(workspace);
        let engine = self.workspaces.engine(workspace);
        let mut tiles: Vec<UiHudTile> = engine
            .preview_placements(HUD_PREVIEW_RECT)
            .into_iter()
            .take(HUD_MAX_TILES)
            .map(|placement| UiHudTile {
                key: placement.slot.0,
                rect: placement.frame,
                bundle: placement
                    .window
                    .as_ref()
                    .and_then(|window| self.bundle_id(&window.app))
                    .map(str::to_owned),
            })
            .collect();
        // Fullscreen Layout shows the window in front first.
        if fullscreen
            && let Some(front) = self
                .workspaces
                .preferred_focus(workspace)
                .and_then(|window| engine.slot_for_window(&window))
            && let Some(index) = tiles.iter().position(|tile| tile.key == front.0)
        {
            let tile = tiles.remove(index);
            tiles.insert(0, tile);
        }
        let mut owned: Vec<_> = self
            .workspaces
            .floating_entries()
            .filter(|(_, owner)| *owner == workspace)
            .map(|(window, _)| window)
            .collect();
        owned.sort_by_key(|window| (window.app.pid, window.window_id));
        let mut floating: Vec<String> = Vec::new();
        for window in owned {
            if floating.len() == HUD_MAX_FLOATING {
                break;
            }
            if let Some(bundle) = self.bundle_id(&window.app)
                && !floating.iter().any(|known| known == bundle)
            {
                floating.push(bundle.to_owned());
            }
        }
        UiHudWorkspace {
            workspace,
            fullscreen,
            tiles,
            floating,
        }
    }
}
