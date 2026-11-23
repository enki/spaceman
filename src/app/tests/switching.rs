use super::*;

#[test]
fn switching_leaves_self_hidden_windows_alone_instead_of_failing() {
    let tiled = window(10);
    let stashed = window(11);
    let mut backend = FakeBackend {
        observation: Some(observation(&tiled, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(tiled.clone(), frame(50));
    backend.frames.insert(stashed.clone(), frame(-4000));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &tiled);
    runtime
        .workspaces
        .own_floating(stashed.clone(), WorkspaceId::ONE);

    runtime
        .switch(
            WorkspaceId::new(2).unwrap(),
            outputs(),
            &AtomicBool::new(false),
        )
        .unwrap();

    assert!(runtime.parked.contains_key(&tiled));
    assert!(!runtime.parked.contains_key(&stashed));
    assert_eq!(runtime.backend.frames[&stashed], frame(-4000));
}

#[test]
fn moving_to_an_inactive_workspace_does_not_switch() {
    let window = window(10);
    let mut backend = FakeBackend {
        focused: Some(window.clone()),
        observation: Some(observation(&window, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), frame(50));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &window);
    let target = WorkspaceId::new(2).unwrap();

    runtime
        .move_focused(target, outputs(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.workspace_of(&window), Some(target));
    assert!(runtime.parked.contains_key(&window));
    assert!(
        !runtime
            .backend
            .calls
            .iter()
            .any(|call| matches!(call, BackendCall::Focus(_)))
    );

    runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
}

#[test]
fn focusing_a_parked_window_activates_its_workspace() {
    let window = window(10);
    let normal = frame(50);
    let outputs = outputs();
    let displays = outputs.frames().collect::<Vec<_>>();
    let parked_frame = crate::geometry::parked_frame(outputs.main().frame, &displays, normal);
    let mut backend = FakeBackend {
        focused: Some(window.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(window.clone(), parked_frame);
    let mut parked = HashMap::new();
    parked.insert(window.clone(), normal);
    let mut runtime = WorkspaceRuntime::with_backend(ManagementScope::Both, backend, parked);
    let workspace = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, workspace, &window);

    let expected = outputs.main().frame;
    let status = runtime.cycle(outputs, &AtomicBool::new(false)).unwrap();

    assert_eq!(status.active, vec![workspace, WorkspaceId::SIX]);
    assert_eq!(runtime.backend.frames[&window], expected);
}

#[test]
fn frontmost_app_activates_before_ax_focus_catches_up() {
    let old = window(10);
    let current = crate::PhysicalWindowId {
        app: AppInstanceId {
            pid: 2,
            launched_at: 2,
        },
        window_id: 20,
    };
    let normal = frame(50);
    let outputs = outputs();
    let displays = outputs.frames().collect::<Vec<_>>();
    let parked_frame = crate::geometry::parked_frame(outputs.main().frame, &displays, normal);
    let mut backend = FakeBackend {
        focused: Some(old.clone()),
        frontmost: Some(current.app.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(old.clone(), normal);
    backend.frames.insert(current.clone(), parked_frame);
    let mut parked = HashMap::new();
    parked.insert(current.clone(), normal);
    let mut runtime = WorkspaceRuntime::with_backend(ManagementScope::Both, backend, parked);
    bind(&mut runtime, WorkspaceId::ONE, &old);
    let target = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, target, &current);

    let status = runtime.cycle(outputs, &AtomicBool::new(false)).unwrap();

    assert_eq!(status.active, vec![target, WorkspaceId::SIX]);
}

#[test]
fn switching_reveals_and_lays_out_incoming_beneath_before_hiding_outgoing() {
    let outgoing = window(10);
    let incoming = window(20);
    let outgoing_normal = frame(50);
    let incoming_normal = frame(100);
    let outputs = outputs();
    let displays = outputs.frames().collect::<Vec<_>>();
    let incoming_parked =
        crate::geometry::parked_frame(outputs.main().frame, &displays, incoming_normal);
    let mut backend = FakeBackend::default();
    backend.frames.insert(outgoing.clone(), outgoing_normal);
    backend.frames.insert(incoming.clone(), incoming_parked);
    let mut parked = HashMap::new();
    parked.insert(incoming.clone(), incoming_normal);
    let mut runtime = WorkspaceRuntime::with_backend(ManagementScope::Both, backend, parked);
    bind(&mut runtime, WorkspaceId::ONE, &outgoing);
    let target = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, target, &incoming);

    runtime
        .switch(target, outputs.clone(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(target)
    );
    assert_eq!(runtime.backend.focused, Some(incoming.clone()));
    assert!(runtime.parked.contains_key(&outgoing));
    assert!(!runtime.parked.contains_key(&incoming));
    let reveal = runtime
        .backend
        .calls
        .iter()
        .position(|call| {
            matches!(call, BackendCall::SetPosition(window, frame) if window == &incoming && frame == &incoming_normal)
        })
        .unwrap();
    let layout = runtime
        .backend
        .calls
        .iter()
        .position(|call| matches!(call, BackendCall::Apply(window, _) if window == &incoming))
        .unwrap();
    let hide = runtime
        .backend
        .calls
        .iter()
        .position(|call| matches!(call, BackendCall::SetPosition(window, _) if window == &outgoing))
        .unwrap();
    // The incoming workspace is fully formed beneath the outgoing windows
    // before anything is hidden, so the desktop never shows.
    assert!(reveal < layout && layout < hide);
}

#[test]
fn switching_away_and_back_restores_last_focused_window() {
    let first = window(10);
    let focused = window(20);
    let incoming = window(30);
    let outputs = outputs();
    let displays = outputs.frames().collect::<Vec<_>>();
    let incoming_normal = frame(100);
    let incoming_parked =
        crate::geometry::parked_frame(outputs.main().frame, &displays, incoming_normal);
    let mut backend = FakeBackend {
        focused: Some(focused.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(50));
    backend.frames.insert(focused.clone(), frame(500));
    backend.frames.insert(incoming.clone(), incoming_parked);
    let mut parked = HashMap::new();
    parked.insert(incoming.clone(), incoming_normal);
    let mut runtime = WorkspaceRuntime::with_backend(ManagementScope::Both, backend, parked);
    bind(&mut runtime, WorkspaceId::ONE, &first);
    bind(&mut runtime, WorkspaceId::ONE, &focused);
    let target = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, target, &incoming);
    let cancelled = AtomicBool::new(false);

    runtime.switch(target, outputs.clone(), &cancelled).unwrap();
    runtime
        .switch(WorkspaceId::ONE, outputs.clone(), &cancelled)
        .unwrap();

    assert_eq!(runtime.backend.focused, Some(focused));
}

#[test]
fn selecting_an_active_workspace_focuses_its_window() {
    let target = window(10);
    let other_display = window(20);
    let mut backend = FakeBackend {
        focused: Some(other_display.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(target.clone(), frame(50));
    backend.frames.insert(other_display.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &target);
    bind(&mut runtime, WorkspaceId::SIX, &other_display);

    runtime
        .switch(WorkspaceId::ONE, outputs(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(runtime.backend.focused, Some(target.clone()));
    assert!(matches!(
        runtime.backend.calls.last(),
        Some(BackendCall::Focus(window)) if window == &target
    ));
}

#[test]
fn selecting_an_active_split_workspace_restores_its_remembered_focus() {
    let first = window(10);
    let remembered = window(20);
    let mut backend = FakeBackend {
        focused: Some(first.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(50));
    backend.frames.insert(remembered.clone(), frame(500));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &first);
    bind(&mut runtime, WorkspaceId::ONE, &remembered);
    runtime
        .workspaces
        .remember_focus(WorkspaceId::ONE, &remembered);

    runtime
        .switch(WorkspaceId::ONE, outputs(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(runtime.backend.focused, Some(remembered));
}

#[test]
fn switching_to_an_unconfigured_named_workspace_is_rejected() {
    let mut runtime = runtime(FakeBackend::default());

    let Err(error) = runtime.switch(
        WorkspaceId::named('Z').unwrap(),
        outputs(),
        &AtomicBool::new(false),
    ) else {
        panic!("unconfigured workspace unexpectedly switched");
    };

    assert_eq!(error, "Workspace Z is not configured");
}

#[test]
fn workspace_back_and_forth_tracks_each_display_lane_independently() {
    let main = window(10);
    let secondary = window(20);
    let mut backend = FakeBackend {
        focused: Some(main.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(main.clone(), frame(50));
    backend.frames.insert(secondary.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &main);
    bind(&mut runtime, WorkspaceId::SIX, &secondary);
    let main_target = WorkspaceId::new(2).unwrap();
    let secondary_target = WorkspaceId::new(7).unwrap();
    let cancelled = AtomicBool::new(false);

    runtime.switch(main_target, outputs(), &cancelled).unwrap();
    runtime.backend.focused = Some(secondary.clone());
    runtime
        .switch(secondary_target, outputs(), &cancelled)
        .unwrap();

    runtime.backend.focused = Some(main);
    runtime
        .workspace_back_and_forth(outputs(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(secondary_target)
    );

    runtime.backend.focused = Some(secondary);
    runtime
        .workspace_back_and_forth(outputs(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(WorkspaceId::SIX)
    );
    assert_eq!(
        runtime.workspaces.previous_for(&DisplaySelector::Main),
        Some(main_target)
    );
    assert_eq!(
        runtime.workspaces.previous_for(&DisplaySelector::Secondary),
        Some(secondary_target)
    );
}

#[test]
fn the_hud_gives_each_display_its_own_row_and_only_the_switched_one_moves() {
    let main = window(10);
    let secondary = window(20);
    let mut backend = FakeBackend {
        focused: Some(secondary.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(main.clone(), frame(50));
    backend.frames.insert(secondary.clone(), frame(1050));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &main);
    bind(&mut runtime, WorkspaceId::SIX, &secondary);
    let cancelled = AtomicBool::new(false);

    let target = WorkspaceId::new(7).unwrap();
    let hud = runtime
        .switch(target, outputs(), &cancelled)
        .unwrap()
        .hud
        .unwrap();
    let rows: Vec<_> = hud
        .displays
        .iter()
        .map(|display| {
            (
                display.id.as_str(),
                display.visible,
                display.from,
                display
                    .workspaces
                    .iter()
                    .map(|entry| entry.workspace)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "main-id",
                Some(WorkspaceId::ONE),
                None,
                vec![WorkspaceId::ONE]
            ),
            (
                "secondary-id",
                Some(target),
                Some(WorkspaceId::SIX),
                vec![WorkspaceId::SIX, target]
            ),
        ]
    );
    assert_eq!(
        hud.display_of(target).map(|display| display.id.as_str()),
        Some("secondary-id")
    );
}

#[test]
fn switching_aborts_when_the_incoming_workspace_cannot_be_laid_out() {
    let outgoing = window(10);
    let incoming = window(20);
    let outgoing_normal = frame(50);
    let incoming_normal = frame(100);
    let outputs = outputs();
    let displays = outputs.frames().collect::<Vec<_>>();
    let incoming_parked =
        crate::geometry::parked_frame(outputs.main().frame, &displays, incoming_normal);
    let mut backend = FakeBackend {
        managed_outcome: Some(macos::ApplyOutcome::Unavailable),
        ..FakeBackend::default()
    };
    backend.frames.insert(outgoing.clone(), outgoing_normal);
    backend.frames.insert(incoming.clone(), incoming_parked);
    let mut parked = HashMap::new();
    parked.insert(incoming.clone(), incoming_normal);
    let mut runtime = WorkspaceRuntime::with_backend(ManagementScope::Both, backend, parked);
    bind(&mut runtime, WorkspaceId::ONE, &outgoing);
    let target = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, target, &incoming);

    assert!(
        runtime
            .switch(target, outputs.clone(), &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.backend.frames[&outgoing], outgoing_normal);
    assert!(!runtime.parked.contains_key(&outgoing));
    assert!(runtime.parked.contains_key(&incoming));
}

#[test]
fn moving_a_floating_window_to_another_workspace_parks_and_transfers_ownership() {
    let floating = window(10);
    let mut backend = FakeBackend {
        focused: Some(floating.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(floating.clone(), frame(100));
    let mut runtime = runtime(backend);
    runtime
        .workspaces
        .own_floating(floating.clone(), WorkspaceId::ONE);
    let target = WorkspaceId::new(2).unwrap();

    runtime
        .move_focused(target, outputs(), &AtomicBool::new(false))
        .unwrap();

    assert_eq!(
        runtime.workspaces.floating_workspace_of(&floating),
        Some(target)
    );
    assert!(runtime.parked.contains_key(&floating));
    assert!(
        !outputs()
            .main()
            .frame
            .contains_center(runtime.backend.frames[&floating])
    );
}

#[test]
fn focusing_an_unmanaged_window_of_a_multi_window_app_does_not_switch_workspaces() {
    let main_window = window(10);
    let secondary_window = window(11);
    let parked_frame = main_parked_frame(frame(50));
    let mut backend = FakeBackend {
        focused: Some(secondary_window.clone()),
        frontmost: Some(secondary_window.app.clone()),
        observation: Some(observation(&secondary_window, frame(200))),
        ..FakeBackend::default()
    };
    backend.frames.insert(main_window.clone(), parked_frame);
    backend.frames.insert(secondary_window.clone(), frame(200));
    let mut runtime = runtime_with_parked(backend, [(main_window.clone(), frame(50))]);
    let hidden = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, hidden, &main_window);

    let status = runtime.cycle(outputs(), &AtomicBool::new(false)).unwrap();

    assert_eq!(status.active, vec![WorkspaceId::ONE, WorkspaceId::SIX]);
    assert_eq!(
        runtime.workspaces.workspace_of(&secondary_window),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(runtime.workspaces.workspace_of(&main_window), Some(hidden));
}

#[test]
fn switch_rollback_preserves_original_recovery_frames_and_suppresses_focus_loop() {
    let outgoing = window(10);
    let incoming_first = window(20);
    let incoming_second = window(30);
    let outputs = outputs();
    let first_original = frame(100);
    let second_original = frame(200);
    let first_parked = main_parked_frame(first_original);
    let second_parked = main_parked_frame(second_original);
    let mut backend = FakeBackend {
        focused: Some(incoming_first.clone()),
        frontmost: Some(incoming_first.app.clone()),
        observation: Some(observation(&outgoing, frame(50))),
        managed_outcome: Some(macos::ApplyOutcome::Unavailable),
        ..FakeBackend::default()
    };
    backend.frames.insert(outgoing.clone(), frame(50));
    backend.frames.insert(incoming_first.clone(), first_parked);
    backend
        .frames
        .insert(incoming_second.clone(), second_parked);
    let mut runtime = runtime_with_parked(
        backend,
        [
            (incoming_first.clone(), first_original),
            (incoming_second.clone(), second_original),
        ],
    );
    bind(&mut runtime, WorkspaceId::ONE, &outgoing);
    let target = WorkspaceId::new(2).unwrap();
    bind(&mut runtime, target, &incoming_first);
    bind(&mut runtime, target, &incoming_second);

    // First cycle attempts focus-follow -> switch to workspace 2, which rolls back
    // when layout_workspace returns Unavailable.
    assert!(
        runtime
            .cycle(outputs.clone(), &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(runtime.parked.get(&incoming_first), Some(&first_original));
    assert_eq!(runtime.parked.get(&incoming_second), Some(&second_original));
    assert_eq!(runtime.suppressed_focus, Some(incoming_first));

    // Next cycle does not loop into another focus-follow switch while focus is unchanged.
    runtime.backend.managed_outcome = None;
    let status = runtime.cycle(outputs, &AtomicBool::new(false)).unwrap();
    assert_eq!(status.active, vec![WorkspaceId::ONE, WorkspaceId::SIX]);
}

#[test]
fn status_menu_lists_each_displays_visible_and_occupied_workspaces() {
    let app_window = |pid, window_id| crate::PhysicalWindowId {
        app: AppInstanceId {
            pid,
            launched_at: 1,
        },
        window_id,
    };
    let mut runtime = runtime(FakeBackend::default());
    bind(&mut runtime, WorkspaceId::ONE, &app_window(7, 10));
    bind(&mut runtime, WorkspaceId::ONE, &app_window(8, 11));
    bind(&mut runtime, WorkspaceId::ONE, &app_window(7, 12));
    let three = WorkspaceId::THREE;
    runtime.workspaces.own_floating(app_window(9, 13), three);

    let menu = runtime.status(0, &outputs()).menu;

    let entry = |workspace, current, apps: &[i32]| status_item::MenuWorkspace {
        workspace,
        current,
        apps: apps.to_vec(),
    };
    assert_eq!(
        menu,
        vec![
            status_item::DisplayWorkspaces {
                display: "Main".into(),
                workspaces: vec![
                    entry(WorkspaceId::ONE, true, &[7, 8]),
                    entry(three, false, &[9]),
                ],
            },
            status_item::DisplayWorkspaces {
                display: "Secondary".into(),
                workspaces: vec![entry(WorkspaceId::SIX, true, &[])],
            },
        ]
    );

    runtime.workspaces.switch(three);
    let menu = runtime.status(0, &outputs()).menu;

    assert_eq!(
        menu[0].workspaces,
        vec![
            entry(WorkspaceId::ONE, false, &[7, 8]),
            entry(three, true, &[9]),
        ]
    );
}

#[test]
fn swipes_step_through_the_occupied_workspaces_of_the_display_under_the_pointer() {
    let tiled = window(10);
    let floating = window(11);
    let mut backend = FakeBackend {
        observation: Some(observation(&tiled, frame(50))),
        ..FakeBackend::default()
    };
    backend.frames.insert(tiled.clone(), frame(50));
    backend.frames.insert(floating.clone(), frame(60));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &tiled);
    runtime
        .workspaces
        .own_floating(floating, WorkspaceId::THREE);
    let main_display = (500, 400);
    let swipe = |runtime: &mut WorkspaceRuntime<FakeBackend>, swipe, point| {
        runtime
            .swipe(swipe, point, outputs(), &AtomicBool::new(false))
            .unwrap()
            .active
    };

    assert_eq!(
        swipe(&mut runtime, Swipe::Left, main_display),
        [WorkspaceId::THREE, WorkspaceId::SIX]
    );
    assert_eq!(
        swipe(&mut runtime, Swipe::Left, main_display),
        [WorkspaceId::THREE, WorkspaceId::SIX]
    );
    assert_eq!(
        swipe(&mut runtime, Swipe::Left, (1500, 400)),
        [WorkspaceId::THREE, WorkspaceId::SIX]
    );
    assert_eq!(
        swipe(&mut runtime, Swipe::Right, main_display),
        [WorkspaceId::ONE, WorkspaceId::SIX]
    );
}

#[test]
fn move_workspace_to_occupied_display_promotes_source_previous_and_parks_displaced() {
    let w1 = window(10);
    let w3 = window(20);
    let w6 = window(30);
    let w3_normal = frame(100);
    let mut backend = FakeBackend {
        focused: Some(w1.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(w1.clone(), frame(50));
    backend
        .frames
        .insert(w3.clone(), main_parked_frame(w3_normal));
    backend.frames.insert(w6.clone(), frame(1050));
    let mut runtime = runtime_with_parked(backend, [(w3.clone(), w3_normal)]);
    bind(&mut runtime, WorkspaceId::ONE, &w1);
    bind(&mut runtime, WorkspaceId::THREE, &w3);
    bind(&mut runtime, WorkspaceId::SIX, &w6);
    // Record workspace 3 as Main's previous workspace while workspace 1 stays active.
    runtime.workspaces.switch(WorkspaceId::THREE);
    runtime.workspaces.switch(WorkspaceId::ONE);
    let cancelled = AtomicBool::new(false);

    let status = runtime
        .move_workspace_to_display(DisplayTarget::Next, outputs(), &cancelled)
        .unwrap();

    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Secondary
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.previous_for(&DisplaySelector::Secondary),
        Some(WorkspaceId::SIX)
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::THREE)
    );
    assert_eq!(runtime.backend.focused, Some(w1.clone()));
    assert_eq!(
        runtime.backend.frames[&w1],
        outputs().secondary().unwrap().frame
    );
    assert_eq!(runtime.backend.frames[&w3], outputs().main().frame);
    assert!(runtime.parked.contains_key(&w6));
    assert!(!runtime.parked.contains_key(&w3));
    assert_eq!(
        runtime.backend.persisted_monitors,
        vec![(WorkspaceId::ONE, DisplaySelector::Secondary)]
    );

    // Reveal-before-park ordering across both displays.
    let calls = &runtime.backend.calls;
    let pos = |p: &dyn Fn(&BackendCall) -> bool| calls.iter().position(p).unwrap();
    let reveal_source =
        pos(&|c| matches!(c, BackendCall::SetPosition(w, f) if w == &w3 && f == &w3_normal));
    let layout_source = pos(&|c| matches!(c, BackendCall::Apply(w, _) if w == &w3));
    let layout_moved = pos(&|c| matches!(c, BackendCall::Apply(w, _) if w == &w1));
    let park_displaced = pos(&|c| matches!(c, BackendCall::SetPosition(w, _) if w == &w6));
    assert!(
        reveal_source < layout_source
            && layout_source < layout_moved
            && layout_moved < park_displaced
    );

    // HUD reflects both displays, with Secondary sliding from displaced workspace 6.
    let hud = status.hud.unwrap();
    assert_eq!(hud.workspace, WorkspaceId::ONE);
    assert_eq!(hud.displays.len(), 2);
    assert_eq!(hud.displays[0].visible, Some(WorkspaceId::THREE));
    assert_eq!(hud.displays[1].visible, Some(WorkspaceId::ONE));
    assert_eq!(hud.displays[1].from, Some(WorkspaceId::SIX));

    // Option+Tab on the target display brings back the displaced workspace 6.
    runtime
        .workspace_back_and_forth(outputs(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(WorkspaceId::SIX)
    );
}

#[test]
fn move_workspace_to_empty_display_and_three_display_wrap_around() {
    let w1 = window(10);
    let mut backend = FakeBackend {
        focused: Some(w1.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(w1.clone(), frame(50));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &w1);
    let cancelled = AtomicBool::new(false);
    let three = three_outputs();

    // Main -> Secondary (empty workspace 6 displaced).
    runtime
        .move_workspace_to_display(DisplayTarget::Next, three.clone(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Secondary
    );
    assert_eq!(runtime.backend.frames[&w1], three.displays()[1].frame);

    // Secondary -> Third (no workspace homed there yet; UUID selector).
    runtime
        .move_workspace_to_display(DisplayTarget::Next, three.clone(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Preferred("third-id".into())
    );
    assert_eq!(runtime.backend.frames[&w1], three.displays()[2].frame);

    // Third -> wraps around to Main.
    runtime
        .move_workspace_to_display(DisplayTarget::Next, three.clone(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Main
    );
    assert_eq!(runtime.backend.frames[&w1], three.displays()[0].frame);

    // Prev wraps Main -> Third; explicit selector moves Third -> Secondary; same display is no-op.
    runtime
        .move_workspace_to_display(DisplayTarget::Prev, three.clone(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Preferred("third-id".into())
    );
    runtime
        .move_workspace_to_display(
            DisplayTarget::Selector(DisplaySelector::Secondary),
            three.clone(),
            &cancelled,
        )
        .unwrap();
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Secondary
    );
    let count_before = runtime.backend.persisted_monitors.len();
    runtime
        .move_workspace_to_display(
            DisplayTarget::Selector(DisplaySelector::Secondary),
            three,
            &cancelled,
        )
        .unwrap();
    assert_eq!(runtime.backend.persisted_monitors.len(), count_before);
}

#[test]
fn move_workspace_single_display_is_a_noop() {
    let w1 = window(10);
    let mut backend = FakeBackend {
        focused: Some(w1.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(w1.clone(), frame(50));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &w1);
    let single = macos::DisplayOutputs::new(vec![outputs().main().clone()]).unwrap();

    let error = runtime
        .move_workspace_to_display(DisplayTarget::Next, single, &AtomicBool::new(false))
        .err()
        .unwrap();

    assert_eq!(error, "Only one display is connected");
    assert!(runtime.backend.persisted_monitors.is_empty());
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Main
    );
}

#[test]
fn move_workspace_preserves_fullscreen_layout_and_focused_tile() {
    let first = window(10);
    let second = window(20);
    let mut backend = FakeBackend {
        focused: Some(second.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(first.clone(), frame(50));
    backend.frames.insert(second.clone(), frame(50));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &first);
    bind(&mut runtime, WorkspaceId::ONE, &second);
    runtime
        .workspaces
        .set_fullscreen_layout(WorkspaceId::ONE, true);

    runtime
        .move_workspace_to_display(DisplayTarget::Next, outputs(), &AtomicBool::new(false))
        .unwrap();

    let secondary_frame = outputs().secondary().unwrap().frame;
    assert!(runtime.workspaces.uses_fullscreen_layout(WorkspaceId::ONE));
    assert_eq!(runtime.backend.frames[&first], secondary_frame);
    assert_eq!(runtime.backend.frames[&second], secondary_frame);
    assert_eq!(runtime.backend.focused, Some(second));
}

#[test]
fn move_workspace_persisted_assignment_survives_config_reload() {
    let dir = std::env::temp_dir().join(format!(
        "spaceman-move-ws-test-{}-{}",
        std::process::id(),
        crate::config::now().as_nanos()
    ));
    let config_path = dir.join("config.toml");
    let w1 = window(10);
    let mut backend = FakeBackend {
        focused: Some(w1.clone()),
        config_path: Some(config_path.clone()),
        ..FakeBackend::default()
    };
    backend.frames.insert(w1.clone(), frame(50));
    let mut runtime = runtime(backend);
    bind(&mut runtime, WorkspaceId::ONE, &w1);
    let cancelled = AtomicBool::new(false);

    runtime
        .move_workspace_to_display(DisplayTarget::Next, outputs(), &cancelled)
        .unwrap();
    runtime.replace_config(crate::config::load_path(&config_path).unwrap());
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Secondary
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(WorkspaceId::ONE)
    );

    // Moving back to Main clears the override and survives reload cleanly.
    runtime
        .move_workspace_to_display(DisplayTarget::Next, outputs(), &cancelled)
        .unwrap();
    runtime.replace_config(crate::config::load_path(&config_path).unwrap());
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Main
    );
    assert!(
        !std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("[[workspaces]]")
    );
    drop(std::fs::remove_dir_all(dir));
}

#[test]
fn move_workspace_replaces_unplugged_assignment_and_rolls_back_on_error() {
    // Part 1: unplugged preferred display showing on main moves to secondary cleanly.
    let w1 = window(10);
    let w9 = window(20);
    let w1_normal = frame(50);
    let config =
        Config::parse("version = 1\n[[workspaces]]\nworkspace = 9\nmonitor = \"unplugged-id\"\n")
            .unwrap();
    let mut backend = FakeBackend {
        focused: Some(w9.clone()),
        ..FakeBackend::default()
    };
    backend
        .frames
        .insert(w1.clone(), main_parked_frame(w1_normal));
    backend.frames.insert(w9.clone(), frame(100));
    let mut parked = HashMap::new();
    parked.insert(w1.clone(), w1_normal);
    let mut runtime =
        WorkspaceRuntime::with_backend_and_config(ManagementScope::Both, backend, parked, config);
    let nine = WorkspaceId::new(9).unwrap();
    bind(&mut runtime, WorkspaceId::ONE, &w1);
    bind(&mut runtime, nine, &w9);
    runtime.foreground_lanes.insert(
        "main-id".into(),
        DisplaySelector::Preferred("unplugged-id".into()),
    );
    let cancelled = AtomicBool::new(false);

    runtime
        .move_workspace_to_display(DisplayTarget::Next, outputs(), &cancelled)
        .unwrap();
    assert_eq!(
        runtime.workspaces.assignment(nine),
        &DisplaySelector::Secondary
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(nine)
    );
    assert_eq!(runtime.backend.frames[&w1], outputs().main().frame);

    // Part 2: config save failure rolls back both displays completely.
    let w6 = window(30);
    let w2 = window(40);
    let w2_normal = frame(80);
    let mut backend = FakeBackend {
        focused: Some(w1.clone()),
        fail_persist_config: true,
        ..FakeBackend::default()
    };
    backend.frames.insert(w1.clone(), outputs().main().frame);
    backend
        .frames
        .insert(w2.clone(), main_parked_frame(w2_normal));
    backend
        .frames
        .insert(w6.clone(), outputs().secondary().unwrap().frame);
    let mut runtime = runtime_with_parked(backend, [(w2.clone(), w2_normal)]);
    bind(&mut runtime, WorkspaceId::ONE, &w1);
    bind(&mut runtime, WorkspaceId::TWO, &w2);
    bind(&mut runtime, WorkspaceId::SIX, &w6);

    assert!(
        runtime
            .move_workspace_to_display(DisplayTarget::Next, outputs(), &cancelled)
            .is_err()
    );
    assert_eq!(
        runtime.workspaces.assignment(WorkspaceId::ONE),
        &DisplaySelector::Main
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Main),
        Some(WorkspaceId::ONE)
    );
    assert_eq!(
        runtime.workspaces.active_for(&DisplaySelector::Secondary),
        Some(WorkspaceId::SIX)
    );
    assert_eq!(runtime.backend.frames[&w1], outputs().main().frame);
    assert_eq!(
        runtime.backend.frames[&w6],
        outputs().secondary().unwrap().frame
    );
    assert!(!runtime.parked.contains_key(&w6));
    assert_eq!(runtime.parked.get(&w2), Some(&w2_normal));
}
