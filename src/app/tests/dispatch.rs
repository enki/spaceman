use super::*;

type DirectAction = fn(
    &mut WorkspaceRuntime<FakeBackend>,
    macos::DisplayOutputs,
    &AtomicBool,
) -> Result<CycleStatus, String>;

fn actions() -> Vec<(WorkerAction, DirectAction)> {
    vec![
        (WorkerAction::Retile, |r, o, c| r.cycle(o, c)),
        (
            WorkerAction::SwitchWorkspace(WorkspaceId::new(2).unwrap()),
            |r, o, c| r.switch(WorkspaceId::new(2).unwrap(), o, c),
        ),
        (WorkerAction::WorkspaceBackAndForth, |r, o, c| {
            r.workspace_back_and_forth(o, c)
        }),
        (
            WorkerAction::MoveWorkspaceToDisplay(DisplayTarget::Next),
            |r, o, c| r.move_workspace_to_display(DisplayTarget::Next, o, c),
        ),
        (WorkerAction::MoveFocused(WorkspaceId::SIX), |r, o, c| {
            r.move_focused(WorkspaceId::SIX, o, c)
        }),
        (
            WorkerAction::MoveApp("com.example.Test".into(), WorkspaceId::SIX),
            |r, o, c| r.move_app("com.example.Test", WorkspaceId::SIX, o, c),
        ),
        (WorkerAction::CycleFocus, |r, o, c| r.cycle_focus(o, c)),
        (WorkerAction::Focus(Direction::Right), |r, o, c| {
            r.focus_relative(Direction::Right, o, c)
        }),
        (WorkerAction::Reorder(Direction::Down), |r, o, c| {
            r.reorder_focused(Direction::Down, o, c)
        }),
        (WorkerAction::ToggleLayout, |r, o, c| r.toggle_layout(o, c)),
        (WorkerAction::Resize(ResizeDirection::Grow), |r, o, c| {
            r.resize_focused(ResizeDirection::Grow, o, c)
        }),
        (WorkerAction::Balance, |r, o, c| r.balance_focused(o, c)),
        (WorkerAction::ToggleFullscreenLayout, |r, o, c| {
            r.toggle_fullscreen_layout(o, c)
        }),
    ]
}

fn fixture() -> WorkspaceRuntime<FakeBackend> {
    let windows = [
        (window(10), frame(50)),
        (window(20), frame(500)),
        (window(30), frame(1050)),
    ];
    let backend = FakeBackend {
        frames: windows.iter().cloned().collect(),
        focused: Some(window(10)),
        frontmost: Some(window(10).app),
        observation: Some(observation_for_windows(
            "com.example.Test",
            &window(10),
            &windows,
        )),
        ..FakeBackend::default()
    };
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &window(10));
    bind(&mut runtime, WorkspaceId::ONE, &window(20));
    bind(&mut runtime, WorkspaceId::SIX, &window(30));
    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();
    runtime.backend.calls.clear();
    runtime
}

fn compare_dispatch(
    command: WorkerCommand,
    direct: impl FnOnce(&mut WorkspaceRuntime<FakeBackend>) -> Result<CycleStatus, String>,
) {
    let mut dispatched = fixture();
    let mut expected = fixture();
    let expected_result = direct(&mut expected);
    let Some(WorkerResult::Status { result, debug }) =
        dispatched.execute(command, &AtomicBool::new(false))
    else {
        panic!("an action or gesture must return status, including on failure")
    };
    assert_eq!(result.as_ref().err(), expected_result.as_ref().err());
    if let (Ok(actual), Ok(expected)) = (result, expected_result) {
        assert_eq!(actual.failed, expected.failed);
        assert_eq!(actual.active, expected.active);
    }
    assert_eq!(dispatched.workspaces.state(), expected.workspaces.state());
    assert_eq!(dispatched.backend.frames, expected.backend.frames);
    assert_eq!(
        dispatched.backend.workspace_state,
        expected.backend.workspace_state
    );
    assert_eq!(dispatched.backend.calls, expected.backend.calls);
    assert_eq!(
        serde_json::to_value(debug).unwrap(),
        serde_json::to_value(expected.debug_state(&outputs())).unwrap()
    );
}

#[test]
fn action_dispatch_preserves_parameters_results_and_post_operation_debug() {
    for (action, direct) in actions() {
        compare_dispatch(WorkerCommand::Action(action, outputs()), |r| {
            direct(r, outputs(), &AtomicBool::new(false))
        });
    }
}

#[test]
fn gesture_dispatch_preserves_preview_and_final_semantics() {
    for preview in [true, false] {
        compare_dispatch(
            WorkerCommand::Gesture(
                FinalGesture::Drop(window(10), (1500, 100)),
                preview,
                outputs(),
            ),
            |r| {
                r.drop_window(
                    window(10),
                    (1500, 100),
                    preview,
                    outputs(),
                    &AtomicBool::new(false),
                )
            },
        );
        let resized = crate::Rect {
            width: 600,
            ..outputs().main().frame
        };
        compare_dispatch(
            WorkerCommand::Gesture(
                FinalGesture::Resize(window(10), resized),
                preview,
                outputs(),
            ),
            |r| {
                r.resize_window(
                    window(10),
                    resized,
                    preview,
                    outputs(),
                    &AtomicBool::new(false),
                )
            },
        );
    }
}

#[test]
fn dispatch_classifies_cancellation_before_worker_receipt() {
    let (sender, receiver) = channel();
    let mut dispatch = WorkerDispatch::new(sender);
    let mut commands = actions()
        .into_iter()
        .map(|(action, _)| {
            let expected = action == WorkerAction::Retile;
            (WorkerCommand::Action(action, outputs()), expected)
        })
        .collect::<Vec<_>>();
    for preview in [true, false] {
        commands.push((
            WorkerCommand::Gesture(FinalGesture::Drop(window(10), (50, 50)), preview, outputs()),
            preview,
        ));
        commands.push((
            WorkerCommand::Gesture(
                FinalGesture::Resize(window(10), frame(50)),
                preview,
                outputs(),
            ),
            preview,
        ));
    }
    commands.push((WorkerCommand::RestoreAll, false));
    commands.push((WorkerCommand::Action(WorkerAction::Retile, outputs()), true));
    commands.push((WorkerCommand::Stop, false));
    for (command, expected) in commands {
        assert!(dispatch.send(command));
        assert!(dispatch.is_busy());
        assert_eq!(dispatch.is_interruptible_busy(), expected);
        assert!(receiver.try_recv().is_ok());
        dispatch.finish_command();
    }
    drop(receiver);
    assert!(!dispatch.send(WorkerCommand::Action(WorkerAction::Retile, outputs())));
    assert!(!dispatch.is_busy());
    assert!(!dispatch.is_interruptible_busy());
}

#[test]
fn control_acknowledgements_name_each_queued_action() {
    let acknowledgements = actions()
        .into_iter()
        .map(|(action, _)| action.queued_acknowledgement())
        .collect::<Vec<_>>();
    assert_eq!(
        acknowledgements,
        [
            "retile queued",
            "switch to workspace 2 queued",
            "workspace back-and-forth queued",
            "move workspace to display queued",
            "move to workspace 6 queued",
            "move app to workspace 6 queued",
            "focus cycle queued",
            "focus queued",
            "directional move queued",
            "layout toggle queued",
            "resize queued",
            "balance queued",
            "fullscreen layout toggle queued",
        ]
    );
}

#[test]
fn restoration_request_sends_now_or_waits_for_the_current_worker() {
    for was_busy in [false, true] {
        let (sender, receiver) = channel();
        let mut dispatch = WorkerDispatch::new(sender);
        if was_busy {
            assert!(dispatch.send(WorkerCommand::Action(WorkerAction::Retile, outputs())));
            assert!(receiver.try_recv().is_ok());
        }
        dispatch.request_restore();
        assert!(dispatch.is_busy());
        assert_eq!(dispatch.is_restoring(), !was_busy);
        assert!(dispatch.is_restoring_or_pending());
        assert_eq!(dispatch.is_interruptible_busy(), was_busy);
        if was_busy {
            assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
            dispatch.finish_command();
            assert_eq!(dispatch.start_pending_restore(), Some(true));
            assert!(dispatch.is_restoring());
        }
        assert!(matches!(receiver.try_recv(), Ok(WorkerCommand::RestoreAll)));
    }
}

#[test]
fn disconnected_worker_does_not_claim_restoration_is_running() {
    let (sender, receiver) = channel();
    drop(receiver);
    let mut dispatch = WorkerDispatch::new(sender);
    dispatch.request_restore();
    assert!(!dispatch.is_busy() && !dispatch.is_restoring_or_pending());
}

#[test]
fn restoration_and_stop_do_not_require_display_outputs() {
    let mut runtime = fixture();
    assert!(matches!(
        runtime.execute(WorkerCommand::RestoreAll, &AtomicBool::new(false)),
        Some(WorkerResult::Restored { failed: 0 })
    ));
    assert!(
        runtime
            .execute(WorkerCommand::Stop, &AtomicBool::new(false))
            .is_none()
    );
}

#[test]
fn failure_count_keeps_all_non_applied_outcomes() {
    use macos::ApplyOutcome::*;
    for outcome in [Applied, Constrained, Gone, Unavailable, Deferred] {
        let results = [macos::ApplyResult {
            window: window(10),
            target: frame(50),
            actual: None,
            outcome,
        }];
        assert_eq!(failed_results(&results), usize::from(outcome != Applied));
    }
}

#[test]
fn hud_emits_only_for_workspace_move_and_layout_actions() {
    let cancelled = AtomicBool::new(false);
    let mut runtime = fixture();

    assert_eq!(runtime.cycle(outputs(), &cancelled).unwrap().hud, None);
    assert_eq!(
        runtime.cycle_focus(outputs(), &cancelled).unwrap().hud,
        None
    );

    let toggled = runtime
        .toggle_layout(outputs(), &cancelled)
        .unwrap()
        .hud
        .unwrap();
    assert_eq!(toggled.workspace, WorkspaceId::ONE);
    assert_eq!(
        toggled.kind,
        UiHudKind::Layout {
            axis: crate::Axis::Vertical
        }
    );
    let row = &toggled.displays[0];
    assert_eq!(row.visible, Some(WorkspaceId::ONE));
    assert_eq!(row.from, None);
    assert_eq!(row.workspaces[0].workspace, WorkspaceId::ONE);
    assert_eq!(row.workspaces[0].tiles.len(), 2);
    assert!(!row.workspaces[0].fullscreen);

    let fullscreen = runtime
        .toggle_fullscreen_layout(outputs(), &cancelled)
        .unwrap()
        .hud
        .unwrap();
    assert_eq!(fullscreen.workspace, WorkspaceId::ONE);
    assert_eq!(fullscreen.kind, UiHudKind::Fullscreen { enabled: true });
    assert!(fullscreen.displays[0].workspaces[0].fullscreen);

    let moved = runtime
        .move_focused(WorkspaceId::TWO, outputs(), &cancelled)
        .unwrap()
        .hud
        .unwrap();
    assert_eq!(moved.workspace, WorkspaceId::TWO);
    assert_eq!(moved.kind, UiHudKind::Moved);
    let row = &moved.displays[0];
    // The marker stays with the user: moving a window never changes the visible workspace.
    assert_eq!(row.visible, Some(WorkspaceId::ONE));
    let tiles = |workspace| {
        row.workspaces
            .iter()
            .find(|entry| entry.workspace == workspace)
            .map_or(0, |entry| entry.tiles.len())
    };
    assert_eq!((tiles(WorkspaceId::ONE), tiles(WorkspaceId::TWO)), (1, 1));

    let switched = runtime
        .switch(WorkspaceId::TWO, outputs(), &cancelled)
        .unwrap()
        .hud
        .unwrap();
    assert_eq!(switched.workspace, WorkspaceId::TWO);
    assert_eq!(switched.kind, UiHudKind::Workspace);
    let row = &switched.displays[0];
    assert_eq!(row.visible, Some(WorkspaceId::TWO));
    assert_eq!(row.from, Some(WorkspaceId::ONE));
    assert_eq!(
        row.workspaces
            .iter()
            .map(|entry| entry.workspace)
            .collect::<Vec<_>>(),
        vec![WorkspaceId::ONE, WorkspaceId::TWO]
    );
}
