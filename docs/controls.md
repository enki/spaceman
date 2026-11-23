# Controls

Spaceman registers global hotkeys while automatic tiling is enabled. Character keys follow the
current macOS input source; Spaceman refreshes registrations after an input-source change. An invalid
layout prevents startup registration and leaves tiling paused. During a layout change, Spaceman keeps
the existing registrations if it cannot construct a replacement plan; if replacement registration
fails after the old registrations are released, it pauses and reports `Hotkeys unavailable`.

| Keys | Action |
| --- | --- |
| `Option+1` ... `Option+9`, `Option+0` | Switch workspace 1 ... 10 |
| Add `Shift` to a workspace key | Move the focused window there without switching |
| `Option+A` ... `Option+Z` | Switch to that configured named workspace |
| Add `Shift` to a named workspace key | Move the focused window there without switching |
| `Option+Tab` | Toggle the focused display's current/previous workspace |
| `Option+Shift+Tab` | Move the focused workspace to the next display (wrapping left-to-right) |
| Option-backtick | Focus the next tiled window, wrapping in stable workspace order |
| `Option+H/J/K/L` | Focus the nearest tile left/down/up/right, including across displays |
| `Option+Shift+H/J/K/L` | Move the focused tile in that direction |
| `Option+/` | Toggle the active workspace layout's root horizontal/vertical axis |
| `Option+-` / `Option+=` | Shrink/grow the focused tile by 50 pixels at its immediate split |
| `Option+Shift+F` | Toggle the workspace-wide Fullscreen Layout without entering native fullscreen |
| Three-finger swipe left/right | Next/previous workspace on the display under the pointer |

A three-finger horizontal swipe steps through the workspaces that the display's menu section lists
(the visible one and every one holding windows), stopping at either end. Swiping left goes to the
next one, as with macOS Spaces. Spaceman only listens to the trackpad, so macOS still acts on the
same swipe: set "Swipe between full-screen applications" to four fingers or off in Trackpad
settings. Four-finger swipes and vertical swipes are left to macOS.

Named workspace shortcuts are registered only when referenced by config. If a configured letter
overlaps a built-in shortcut, the workspace shortcut takes precedence.

Spaceman remembers the last-focused logical tile in each workspace and restores that focus when the
workspace becomes active again. Pressing an already-active workspace shortcut preserves the current
focus when one of that workspace's windows is already focused; otherwise it focuses the remembered
tile.

Fullscreen Layout is a workspace mode: every tiled window receives the full display frame, and the
focused window is raised above the others. Its menu-bar workspace glyph is filled while the mode is
active. Option-backtick focuses the next tiled window and wraps
through the workspace's stable tile order; it provides the same ordering in split layouts. In
Fullscreen Layout, `Option+H`/`Option+K` step to the previous tile and `Option+L`/`Option+J` to the
next one in that order without wrapping; past either end of the stack they continue to the adjacent
display, as in a split layout. Leaving the workspace hides its windows back to front, so stacked
windows are never uncovered mid-switch.
Command-backtick continues to use macOS's same-application window cycle, and Spaceman remembers the
resulting focus. Press `Option+Shift+F` again to restore the unchanged split layout. Fullscreen Layout
and its active tile survive workspace switches, native-tab physical-window replacement, and Spaceman
restarts. `Option+/` exits Fullscreen Layout, toggles the preserved split axis, and immediately applies
the resulting normal layout. Keyboard resize, balance, and tile reordering require leaving Fullscreen
Layout first. Pointer dragging and resizing are ignored while the mode is active. Floating windows are
not expanded. Removing or moving away the workspace's final tiled window exits Fullscreen Layout.

Dragging a managed tile over a sibling swaps it. Dropping near a tile edge can create a nested split;
dropping onto another display can move it to that display's active workspace. Resizing a managed
window with the pointer updates the relevant split weights. Spaceman previews the rest of the layout
during the gesture and applies the final result on mouse-up.

While the worker is busy, keyboard commands and completed pointer gestures are queued in arrival
order. Repeated relative commands (resize, focus, move, and toggles) are not discarded. The queue holds
up to 64 pending inputs; excess input is rejected with `Input queue full; command was not queued` in
the menu status and log. Pausing, quitting, or losing hotkeys clears pending input so it cannot replay
after resuming. A gesture is classified from its starting and final geometry even if the worker was
too busy to show a preview. Pointer tracking reads window-server geometry rather than making
synchronous Accessibility requests on the main thread.

A workspace switch reveals incoming parked windows before hiding outgoing windows. If restoration
cannot be confirmed by the window server within the bounded reveal wait, the switch is rolled back:
the outgoing workspace stays active and pending incoming reveals are re-parked. Retry once the slow
application is responsive. Successful Accessibility writes alone do not confirm arrival.

## Menu

The menu-bar control shows active workspace/layout glyphs and runtime errors. Its actions are:

- `Run Setup...`, shown while Accessibility permission is missing;
- while tiling, the workspaces of each display (under the display's name when there are several):
  the visible one checked, plus every workspace holding windows, each with its apps' icons (names on
  hover) and its `Option` shortcut. Choosing one switches to it;
- `Disable Tiling`, or `Enable Tiling` while paused;
- `Workspace HUD`, checked while the non-activating workspace capsule (`[settings] hud = true`) is enabled;
  `Settings > General > HUD style` picks how it draws each workspace (`Icons` or `Layouts`, `[settings] hud_style`, default `Icons`);
- `Launch at Login`, shown checked when macOS has enabled the signed app bundle;
- `Settings...` (`⌘,`), opening the Rune Settings window (`General`, `Rules`, `Workspaces`, `Shortcuts`, and `Activity`, plus `⇧⌘D` Playground);
- `Check for Updates...`, which checks and opens Settings (its sidebar and `General > Updates` show
  the result), or `Install Spaceman <version> and Restart` once an update is downloaded and verified;
- `Quit Spaceman` (`⌘Q`).

If macOS reports that the login item requires approval, the menu label says
`Launch at Login (Approval Required...)`; choosing it opens Login Items settings. Registration does
not bypass macOS consent and does not affect the currently running process.

Pausing, quitting, and external macOS termination requests restore parked windows before completing.
Restoration retries are visible in the menu status. Installing an update uses the same restoration
gate: Spaceman swaps in the new bundle and relaunches only after every parked window is restored and
its recovery metadata is durably removed. If the install then fails, Spaceman keeps running and
resumes tiling. `Settings > General > Updates` shows the running version and the updater's state,
and picks the release channel (`Stable` or `Dev`, `[settings] update_channel`, default `Stable`).

## CLI

The installed CLI is `~/.local/bin/spaceman`. Add `~/.local/bin` to `PATH`, or invoke that absolute
path. Commands that alter layouts require automatic tiling to be enabled.

```sh
spaceman ctl status
spaceman ctl debug
spaceman ctl switch 2
spaceman ctl back-and-forth
spaceman ctl move-workspace
spaceman ctl move-workspace prev
spaceman ctl move-workspace secondary
spaceman ctl move 7
spaceman ctl move-app com.tinyspeck.slackmacgap 6
spaceman ctl focus left
spaceman ctl focus next
spaceman ctl move-direction right
spaceman ctl layout
spaceman ctl fullscreen
spaceman ctl resize shrink
spaceman ctl resize grow
spaceman ctl balance
spaceman ctl pause
spaceman ctl resume
spaceman ctl retile
spaceman ctl wrong
spaceman ctl restart
spaceman ctl quit
spaceman hud
spaceman doctor
spaceman doctor 24
```

Directions are `left`, `down`, `up`, and `right`. `focus next` is Option-backtick and `fullscreen`
toggles the focused workspace's Fullscreen Layout, like `Option+Shift+F`. `balance` equalizes the focused tile's immediate
split while respecting nested minimum sizes. `hud` repeatedly renders sanitized runtime geometry in
the terminal until interrupted. `ctl wrong` marks the journal the moment something looks wrong and
writes a full arrangement snapshot, so `doctor` shows what the screen held then.

`ctl debug` excludes window titles. It reports each display's Fullscreen Layout state and tiled
geometry plus floating-window workspace, parked state, and original recovery frame so workspace leaks
can be diagnosed safely. Its `parking_exempt` list names managed windows that repeatedly rejected
parking positions and are therefore left visible across workspace switches for the current session.

`doctor` summarizes the arrangement journal (all of it, or the last N hours): agent starts and their
loaded state, exit restorations, remembered placements recalled or left unclaimed, wrong placements
(windows Kit moved to another workspace by hand within 120 seconds of Spaceman placing them, by
placement source: active workspace, recall, or rule), the times `ctl wrong` was run, and every
model/screen disagreement that lasted at least three seconds, grouped by kind with counts, total and
longest durations, and the applications involved. It ends with the last known per-workspace
arrangement. It reads files directly and works while the agent is stopped. The journaled kinds are:

| Kind | Meaning |
| --- | --- |
| `leaked` | owned by a hidden workspace, not parked, yet on a display |
| `stranded` | owned by a visible workspace and not parked, yet off every display |
| `parked_visible` | recorded as parked, yet on a display |
| `unadopted` | managed by configuration and on a display, but owned by no workspace |
| `misplaced` | tiled on a visible workspace more than 16 pixels from its layout target |
| `missing` | owned, but the window server no longer lists it |
| `parking_exempt` | left visible across switches after repeatedly un-parking itself |

The agent listens at `~/Library/Application Support/Spaceman/control.sock`; both socket and lock file
are mode `0600`. Requests are newline-delimited, limited to 4 KiB, and served locally over a Unix
socket. The lock prevents two agents from running at once.
