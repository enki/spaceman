# Workspaces

Keep windows grouped on numbered or named virtual workspaces per display, switch between them
instantly, and send windows or whole apps to a workspace, without using macOS Spaces.

## Sub-features

```ts
Switch workspace                                           // ws.switch
├── Option+1..0 / Option+A..Z / spaceman ctl switch <ws> / a workspace in the menu bar menu
├── Incoming parked windows revealed -> laid out -> outgoing windows parked off-screen
│   └── Reveal not confirmed by the window server in time -> switch rolled back, old workspace stays
├── Focus -> last-focused tile of the incoming workspace
└── Already active -> stays, re-focuses its remembered tile (keeps current focus if it is there)

Swipe                                                      // ws.swipe
└── Three-finger swipe left/right -> next/previous workspace in the menu section of the display
    under the pointer (visible + occupied), stops at either end

Back-and-forth                                             // ws.back-and-forth
└── Option+Tab / ctl back-and-forth -> previous workspace of the focused display's lane

Move workspace to another display                          // ws.move-workspace
├── Option+Shift+Tab / ctl move-workspace [next|prev|<display>]
├── Orders displays left-to-right; wraps around for next/prev; persists assignment to config.toml
├── Reveals & lays out source display's previous workspace -> lays out moved workspace on target
│   -> parks target's displaced workspace (which becomes target's Option+Tab previous)
└── Preserves Fullscreen Layout and focus on the moved workspace; shows HUD on both displays

Move window without switching                              // ws.move-window
├── Option+Shift+<ws> / ctl move <ws> -> window parked on target, current workspace reflows
└── ctl move-app <bundle-id> <ws>     -> every current window of that app

Focus follows                                              // ws.focus-follow
└── Focusing a window that lives on a hidden workspace (Dock, Cmd+Tab, notification)
    -> that workspace becomes active

Where a new window goes (first match wins)                 // ws.adoption
├── ignore rule        -> never managed
├── route rule         -> that workspace (parked if hidden); bundle-only route rules apply to the
│                         app's initial window while secondary windows stay on the active workspace
├── float rule / dialog / non-resizable -> floating on the workspace where first seen
├── remembered placement (startup window, first 60 s, or the app relaunched within 30 s of
│   quitting) -> remembered workspace
└── otherwise          -> active workspace of the display under the window's center

Displays                                                    // ws.displays
├── Default: 1-5 main, 6-10 secondary; [[workspaces]] monitor = main | secondary | UUID | name
└── Preferred display unplugged -> its workspaces fall back to main, return on reconnect

Workspace HUD ([settings] hud, on by default)               // ws.hud
├── A switch, a window moved to another workspace, a split-axis or Fullscreen Layout toggle
│   -> a capsule near the bottom of every display, each with its own row: every workspace on that
│      display holding windows, plus the visible one, in order; a marker on the visible one
├── hud_style = "icons" (default) -> each workspace as its number and its apps' icons (one per app,
│   │  tile order); the visible group bright, the others dimmed; nothing changes size
│   └── "layouts" -> each workspace as its layout with an app icon per tile, the visible one larger
│      (others .62×, dimmed); an empty one is a dot; Fullscreen Layout is one tile with the front
│      app's icon and the others smaller beside it
├── No text but workspace numbers, and never a window title; icons are looked up by bundle ID
├── Appears: fades in, scales from .96 and rises 6 pt; stays 1.15 s after the last event; leaves
│   the same way back in about 0.17 s
├── Interactive without stealing focus (OverlayInput::Regions, KeyPolicy::None):
│   ├── Hover over the capsule -> pauses auto-dismiss (and rescues a mid-exit capsule); resumes
│   │   0.6 s after the pointer leaves
│   ├── Click a workspace slot -> switches that display to that workspace without activating
│   ├── Trailing Settings gear -> opens Spaceman Settings and glides the capsule out
│   └── Right-click (or Ctrl+click) an app icon -> native Float/Tile and Pin/Unpin menu writing
│       through config.toml rules
├── Another switch while it's up, or on its way out -> the same capsule carries on from where it
│   is: the marker's leading edge sets off first and its trailing edge catches up; in Layouts the
│   cells grow and shrink and their neighbours slide. Only the display that switched moves its
│   marker; every capsule holds and leaves together
├── Shown again after hiding -> the marker (and in Layouts the large cell) starts on the
│   workspace the switch left and travels as the capsule fades in
├── A workspace joins or leaves the row -> it fades and scales in place; a moved window's icon
│   leaves one workspace and appears in the other; the marker stays with you
├── The same workspace's layout changes -> its tiles move to their new frames
├── More workspaces than fit -> the row scrolls to keep the visible one in view, its cut edge fades
└── Reduce Motion -> the same changes as a short fade, with nothing travelling or scaling
```

Hidden windows are **parked**, not minimized: moved so a one-pixel strip remains at a display edge,
with the original frame persisted to `parked-windows.json` first.

## How To Get To It (User POV)

```ts
Keyboard (while tiling is enabled)
├── Option+<1..0 | A..Z>        -> switch
├── Option+Shift+<1..0 | A..Z>  -> move focused window
├── Option+Tab                  -> back-and-forth
└── Option+Shift+Tab            -> move focused workspace to next display
Trackpad: three-finger swipe left/right -> next/previous workspace (turn macOS's three-finger
          "Swipe between full-screen applications" off or move it to four fingers)
CLI: spaceman ctl switch | back-and-forth | move-workspace | move | move-app
Config: ~/Library/Application Support/Spaceman/config.toml -> [[rules]], [[workspaces]] (edits apply when the file changes; new named-workspace shortcuts register on the next app start)
Menu bar: shows the active workspace glyphs; its menu lists each display's visible workspace (checked)
          and occupied ones with their apps' icons and Option shortcut -> choose one to switch
Mouse: drop a tile on another display -> moves it to that display's active workspace (see tiling.md)
```

Named-workspace keys exist only for letters referenced in config; they override built-in shortcuts
on the same letter.

## Existing Checks

- Unit (FakeBackend), `src/app/tests/switching.rs`:
  `switching_reveals_and_lays_out_incoming_beneath_before_hiding_outgoing` (ordering of reveal,
  layout, hide), `switch_keeps_outgoing_visible_when_incoming_reveal_never_arrives`
  (`reliability.rs`, rollback), `moving_to_an_inactive_workspace_does_not_switch`,
  `focusing_a_parked_window_activates_its_workspace`,
  `workspace_back_and_forth_tracks_each_display_lane_independently`,
  `switching_to_an_unconfigured_named_workspace_is_rejected`,
  `move_workspace_to_occupied_display_promotes_source_previous_and_parks_displaced`,
  `move_workspace_to_empty_display_and_three_display_wrap_around`,
  `move_workspace_single_display_is_a_noop`,
  `move_workspace_preserves_fullscreen_layout_and_focused_tile`,
  `move_workspace_persisted_assignment_survives_config_reload`,
  `move_workspace_replaces_unplugged_assignment_and_rolls_back_on_error`.
- Unit (FakeBackend), `src/app/tests/switching.rs`:
  `status_menu_lists_each_displays_visible_and_occupied_workspaces` (menu rows per display, checks,
  app order). The AppKit rows themselves (icons, tooltip of app names) are not covered.
- Unit: `swipes_step_through_the_occupied_workspaces_of_the_display_under_the_pointer`
  (`src/app/tests/switching.rs`) and the recognizer in `src/swipe.rs` (fires once per swipe; ignores
  short, vertical and four-finger swipes). The event tap itself (`src/macos/swipes.rs`) is not
  covered.
- Unit (FakeBackend), `src/app/tests/reconciliation.rs`: adoption order
  (`bundle_route_applies_to_initial_window_while_secondary_windows_stay_on_active_workspace`,
  `title_route_rule_routes_matching_secondary_window`,
  `ignore_rule_releases_the_window_and_never_adopts_it`,
  `floating_rule_owns_window_without_adding_it_to_layout`,
  `new_window_opens_on_active_workspace_even_when_app_has_window_on_inactive_workspace`), display fallback
  (`missing_preferred_display_falls_back_without_double_visibility`,
  `active_preferred_workspace_falls_back_and_returns_after_hotplug`).
- Unit (FakeBackend), `src/app/tests/parking.rs`: parking without minimizing, re-parking windows that
  drift back, honoring windows that keep un-parking (parking exemption, working tree), and keeping
  slots when Accessibility briefly cannot find a hidden window.
- Unit (FakeBackend), `src/app/tests/reconciliation.rs`:
  `an_app_that_quits_while_hidden_releases_its_tiles_and_recovery_metadata` (an app that quits
  while its windows are parked frees their tiles without the workspace being shown).
- Unit: `src/hotkeys/delivery.rs` pairs Option presses seen by the key tap with Carbon deliveries
  in either order and reports a press still undelivered after 750 ms. The tap
  (`src/macos/keys.rs`) and the re-registration in `src/app/app_loop/hotkey_health.rs` are not
  covered.
- Live harness (not run): `scripts/live-workspaces.sh` switches 6/7/8 by real hotkeys, moves without
  switching, rapid-switch coalescing, and hard-kill recovery. Needs two displays and AeroSpace.

- Unit, `src/app/tests/dispatch.rs` and `src/app/tests/switching.rs`: the HUD event's rows per
  display, the visible workspace on each, and `from` only on the display that switched.
- Unit, `agent/src/hud/tests.rs`: when the HUD shows, holds, leaves and comes back, a fresh show
  starting on the workspace the switch left, the Layouts row's width, the Icons apps, and which
  panel each display keeps as displays come and go. `agent/src/hud/motion_tests.rs` steps the
  overlay's own view at 120 Hz on a virtual clock, for both styles, and reads back each frame's
  presented rects, opacity and scale: a burst of switches, a hide and a show pass Rune's motion
  gate (no pop, jump, cut fade or overshoot); the marker goes one way per switch, its leading
  edge first, and a switch mid-flight carries it on without slowing; shown again, it starts where
  the switch left; the other display's row stays at rest; a moved window's icon changes groups;
  the widest row fits the panel with the marker in view; Reduce Motion moves nothing. Not
  covered: the panels themselves (ordering in and out, the first show's fade, moving between
  displays, a display unplugged while one is up) and how the motion looks on a display.
  Multi-display has not been seen on real hardware.

## Gotchas

- Unit tests model frames, not z-order or paint timing: "outgoing parked after incoming laid out"
  does not prove nothing flashes. See the stacked-window flash in
  [Fullscreen Layout](fullscreen-layout.md#gotchas).
- Some apps (Electron, Ghostty) move their own windows back on screen; Spaceman re-parks, then after
  repeated undoing honors the reappearance and re-homes the window to the visible workspace. In the
  working tree, a window that does so twice in five minutes stays tiled but is no longer hidden
  across switches (`parking_exempt`).
- Focus-follow switches workspaces as a side effect of any app activation, including ones the user
  did not intend (an app raising itself). Each `focus follow` line in `agent.log` records recent
  input timing and whether Command was held, so Command-Tab can be told apart from self-activation.
- Panels and non-standard windows are never parked, so they show on every workspace; an empty
  workspace exposes whatever unmanaged windows sit behind managed ones.
- macOS can stop delivering Carbon hotkeys to a running agent (seen twice on Nov 9, 2026, cause
  unknown). A listen-only key tap sees an Option shortcut Carbon didn't take; after 750 ms the agent
  logs `hotkey: <name> pressed but macOS did not deliver it`, registers every shortcut again (at
  most every 5 s) and runs the missed action. Each delivered press logs `hotkey: <name>`.
- Config edits (rules, display assignments, and HUD settings) reload automatically before the next
  command; new named-workspace hotkeys register on the next app start, and `ctl restart` exits
  without relaunching.
- The HUD's first appearance after launch only fades in: its panel has drawn nothing yet for the
  capsule to rise from. Every later one enters with the full motion.
- Try the HUD's motion without switching workspaces in the Playground (`⇧⌘D`, HUD > hud-switch):
  both styles at once. `N`/`P` step a workspace, `R` and `B` switch several quickly, `M` moves
  a window, `L` changes the layout, `H` hides it and shows the next workspace.
