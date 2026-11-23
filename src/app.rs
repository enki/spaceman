use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, TryRecvError, channel},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    AppInstanceId, BindingState, Decision, Direction, DisplaySelector, DisplayTarget,
    NavigationTile, ResizeDirection, Settings, WorkspaceId, Workspaces,
    config::{Config, WindowDisposition},
    control::{self, Command as ControlCommand, Request as ControlRequest},
    hotkeys::{DeliveryWatch, HotkeyPlan, Hotkeys},
    live, login_item, macos, permission_guide, status_item,
    swipe::Swipe,
    termination,
};

/// Timestamped diagnostic line on stderr (redirected to `agent.log` in bundle
/// launches). Never include window titles or other user content.
///
/// Unlike `eprintln!`, a failed write (such as a full disk) is ignored rather
/// than panicking the calling thread, and each line is written in one call.
macro_rules! log_event {
    ($($arg:tt)*) => {{
        let now = $crate::config::now();
        let line = format!(
            "[{}.{:03}] {}\n",
            now.as_secs(),
            now.subsec_millis(),
            format_args!($($arg)*)
        );
        // Writing to stderr itself has no fallback sink when the pipe or disk is full.
        drop(std::io::Write::write_all(&mut std::io::stderr().lock(), line.as_bytes()));
    }};
}

mod app_loop;
mod backend;
mod bridge;
mod commands;
mod cycle;
mod hud_event;
mod input;
mod journal;
mod memory;
mod minimums;
mod move_workspace;
mod parking;
mod runtime;
mod state_writer;
mod switching;
mod worker;

pub use app_loop::{MainThreadAgent, TickOutcome};
pub use bridge::{
    HUD_MAX_DISPLAYS, HUD_MAX_FLOATING, HUD_MAX_TILES, HUD_PREVIEW_RECT, UiAction, UiAgentSnapshot,
    UiBridge, UiHudDisplay, UiHudEvent, UiHudKind, UiHudLayout, UiHudTile, UiHudWorkspace,
    UiLaunchAtLogin, UiNews, UiUpdateStatus,
};

pub struct HostCallbacks {
    pub open_settings: Box<dyn FnMut() -> Result<(), String>>,
    pub open_setup: Box<dyn FnMut() -> Result<(), String>>,
    pub save_hud: Box<dyn FnMut(bool) -> Result<(), String>>,
}

use app_loop::AppLoopInit;
use backend::*;
use input::{GestureTracker, InputQueue, QueuedInput};
use journal::JournalState;
use memory::MemoryState;
use runtime::*;
use worker::spawn_worker;

pub const TICK_INTERVAL: Duration = Duration::from_millis(25);
const FALLBACK_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const APPLICATION_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const ACCESSIBILITY_POLL_INTERVAL: Duration = Duration::from_millis(120);
const RESTORE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const APPLICATION_ACTIVATION_SETTLE: Duration = Duration::from_millis(50);

const fn refresh_due(
    window_created: bool,
    event_dirty: bool,
    fallback_due: bool,
    activation_ready: bool,
) -> bool {
    window_created || (activation_ready && (event_dirty || fallback_due))
}

#[derive(Clone, Copy)]
enum ManagementScope {
    Both,
    Main,
    Secondary,
}

impl ManagementScope {
    fn initial_workspaces(self) -> &'static [WorkspaceId] {
        match self {
            Self::Both => &[WorkspaceId::ONE, WorkspaceId::SIX],
            Self::Main => &[WorkspaceId::ONE],
            Self::Secondary => &[WorkspaceId::SIX],
        }
    }

    fn automatic_status(self) -> &'static str {
        match self {
            Self::Both => "Automatic tiling enabled · Workspaces 1 / 6",
            Self::Main => "Automatic tiling enabled · Workspace 1",
            Self::Secondary => "Automatic tiling enabled · Workspace 6",
        }
    }

    fn from_environment() -> Result<Self, String> {
        match crate::config::managed_display_env()? {
            Some(value) => Self::parse(&value),
            None => Ok(Self::Both),
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "main" => Ok(Self::Main),
            "secondary" => Ok(Self::Secondary),
            "both" => Ok(Self::Both),
            _ => Err(format!(
                "SPACEMAN_MANAGED_DISPLAY must be main, secondary, or both; got {value:?}"
            )),
        }
    }

    fn manages_selector(self, display: &DisplaySelector) -> bool {
        matches!(
            (self, display),
            (Self::Both, _)
                | (Self::Main, DisplaySelector::Main)
                | (Self::Secondary, DisplaySelector::Secondary)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Worker(WorkerAction),
    /// Carbon delivered the hotkey with this ID.
    Hotkey(u32),
    /// The keyboard tap saw an Option key press (virtual key, Carbon modifiers).
    OptionKeyDown(u32, u32),
    ToggleAutomatic,
    /// The first-run setup tour finished, so its hold on tiling ends.
    FinishSetup,
    OpenSettings,
    OpenSetup,
    OpenAccessibility,
    ToggleHud,
    SetHud(bool),
    CheckForUpdates,
    InstallUpdate,
    Terminate,
    ToggleLaunchAtLogin,
    Restart,
    Quit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkerAction {
    Retile,
    SwitchWorkspace(WorkspaceId),
    WorkspaceBackAndForth,
    MoveWorkspaceToDisplay(DisplayTarget),
    /// A trackpad swipe, stepping through the occupied workspaces of the display
    /// under the pointer.
    Swipe(Swipe, (i32, i32)),
    MoveFocused(WorkspaceId),
    MoveApp(String, WorkspaceId),
    CycleFocus,
    Focus(Direction),
    Reorder(Direction),
    ToggleLayout,
    Resize(ResizeDirection),
    Balance,
    ToggleFullscreenLayout,
    /// Kit noticed something wrong: mark the journal and snapshot it.
    MarkWrong,
}

impl WorkerAction {
    fn queued_acknowledgement(&self) -> String {
        match self {
            Self::Retile => "retile queued".into(),
            Self::SwitchWorkspace(workspace) => format!("switch to workspace {workspace} queued"),
            Self::WorkspaceBackAndForth => "workspace back-and-forth queued".into(),
            Self::MoveWorkspaceToDisplay(_) => "move workspace to display queued".into(),
            Self::Swipe(..) => "swipe queued".into(),
            Self::MoveFocused(workspace) => format!("move to workspace {workspace} queued"),
            Self::MoveApp(_, workspace) => format!("move app to workspace {workspace} queued"),
            Self::CycleFocus => "focus cycle queued".into(),
            Self::Focus(_) => "focus queued".into(),
            Self::Reorder(_) => "directional move queued".into(),
            Self::ToggleLayout => "layout toggle queued".into(),
            Self::Resize(_) => "resize queued".into(),
            Self::Balance => "balance queued".into(),
            Self::ToggleFullscreenLayout => "fullscreen layout toggle queued".into(),
            Self::MarkWrong => "marking the journal".into(),
        }
    }
}

enum WorkerCommand {
    Action(WorkerAction, macos::DisplayOutputs),
    Gesture(FinalGesture, bool, macos::DisplayOutputs),
    RestoreAll,
    Stop,
}

/// Worker commands slower than this are logged; long Accessibility stalls
/// otherwise look like Spaceman silently doing nothing.
const SLOW_WORKER_COMMAND: Duration = Duration::from_secs(2);

impl WorkerCommand {
    const fn label(&self) -> &'static str {
        match self {
            Self::Action(WorkerAction::Retile, _) => "retile",
            Self::Action(..) => "action",
            Self::Gesture(..) => "gesture",
            Self::RestoreAll => "restore all",
            Self::Stop => "stop",
        }
    }

    fn interruptible(&self) -> bool {
        matches!(
            self,
            Self::Action(WorkerAction::Retile, _) | Self::Gesture(_, true, _)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FinalGesture {
    Drop(crate::PhysicalWindowId, (i32, i32)),
    Resize(crate::PhysicalWindowId, crate::Rect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExitIntent {
    Quit,
    Restart,
    Update,
    Terminate,
}

impl ExitIntent {
    fn mode_label(self) -> &'static str {
        match self {
            Self::Quit => "quitting",
            Self::Restart => "restarting",
            Self::Update => "updating",
            Self::Terminate => "terminating",
        }
    }
}

enum WorkerResult {
    Status {
        result: Result<CycleStatus, String>,
        debug: Box<RuntimeDebug>,
    },
    Restored {
        failed: usize,
    },
}

/// Startup state initialized before the Rune event loop starts: single-agent
/// lock, control socket, signal handler, and the window-management worker.
pub struct PreparedAgent {
    actions: Receiver<Action>,
    actions_sender: Sender<Action>,
    commands: Sender<WorkerCommand>,
    results: Receiver<WorkerResult>,
    cancelled: Arc<AtomicBool>,
    scope: ManagementScope,
    hotkey_workspaces: Vec<WorkspaceId>,
    hud: bool,
    controls: Receiver<ControlRequest>,
    control_server: control::Server,
    bridge: UiBridge,
}

pub fn prepare() -> Result<(PreparedAgent, UiBridge), String> {
    macos::redirect_stderr_to_log();
    log_event!(
        "agent started (pid {}, version {})",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    );
    let scope = ManagementScope::from_environment()?;
    let parked = load_parked()?;
    let (config, workspace_state) =
        recover_startup_metadata(Config::load(), load_workspace_state());
    let hud = config.hud();
    let mut configured_workspaces = Workspaces::default();
    config.configure_workspaces(&mut configured_workspaces);
    let mut hotkey_workspaces = WorkspaceId::numbered()
        .chain(config.named_workspaces())
        .chain(
            workspace_state
                .as_ref()
                .into_iter()
                .flat_map(crate::workspace::WorkspaceState::named_workspaces_in_use),
        )
        .filter(|workspace| scope.manages_selector(configured_workspaces.assignment(*workspace)))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    hotkey_workspaces.sort();
    let (commands, worker_commands) = channel();
    let (worker_results, results) = channel();
    let (actions_sender, actions) = channel();
    let (control_requests, controls) = channel();
    let control_server = control::listen(control_requests)?;
    let signal_actions = actions_sender.clone();
    ctrlc::set_handler(move || {
        // Receiver disconnection means the main loop has already exited.
        drop(signal_actions.send(Action::Quit));
    })
    .map_err(|error| format!("failed to install app signal handler: {error}"))?;
    let cancelled = Arc::new(AtomicBool::new(false));
    spawn_worker(
        worker_commands,
        worker_results,
        Arc::clone(&cancelled),
        scope,
        parked,
        config,
        workspace_state,
    );
    let bridge = UiBridge::new(actions_sender.clone());
    Ok((
        PreparedAgent {
            actions,
            actions_sender,
            commands,
            results,
            cancelled,
            scope,
            hotkey_workspaces,
            hud,
            controls,
            control_server,
            bridge: bridge.clone(),
        },
        bridge,
    ))
}

impl PreparedAgent {
    /// Installs main-thread AppKit hooks (termination gate, status item, updater,
    /// AX event monitor) once winit's `NSApplication` delegate is active.
    /// `setup_tour_open` holds automatic tiling until the first-run setup tour
    /// finishes, so a new install is never retiled behind the setup window.
    pub fn start_main_thread(
        self,
        mut callbacks: HostCallbacks,
        setup_tour_open: bool,
    ) -> Result<MainThreadAgent, String> {
        if let Err(error) = termination::install(self.actions_sender.clone()) {
            log_event!("failed to install application termination gate: {error}");
            return Err(format!(
                "failed to install application termination gate: {error}"
            ));
        }
        if let Err(error) = status_item::install(self.actions_sender.clone()) {
            log_event!("failed to install Spaceman status item: {error}");
            termination::allow_next();
            return Err(format!("failed to install Spaceman status item: {error}"));
        }
        status_item::set_hud(self.hud);
        let launch_status = login_item::status();
        match &launch_status {
            Ok(status) => status_item::set_launch_at_login(*status),
            Err(error) => {
                status_item::set_status(&format!("Launch at Login unavailable: {error}"));
            }
        }
        let workspace_numbers = self
            .scope
            .initial_workspaces()
            .iter()
            .filter_map(|workspace| workspace.number())
            .collect::<Vec<_>>();
        status_item::set_workspace_numbers(&workspace_numbers);
        let trusted = macos::accessibility_trusted();
        status_item::set_accessibility(trusted);
        let events = macos::WindowEventMonitor::new();
        let mut agent = MainThreadAgent::new(
            AppLoopInit {
                actions: self.actions,
                actions_sender: self.actions_sender,
                commands: self.commands,
                results: self.results,
                cancelled: self.cancelled,
                scope: self.scope,
                events,
                hotkey_workspaces: self.hotkey_workspaces,
                hud: self.hud,
                trusted,
                setup_tour_open,
                controls: self.controls,
                control_server: self.control_server,
                bridge: self.bridge,
                callbacks: HostCallbacks {
                    open_settings: Box::new(|| Ok(())),
                    open_setup: Box::new(|| Ok(())),
                    save_hud: Box::new(|_| Ok(())),
                },
            },
            launch_status,
        );
        if !trusted {
            status_item::set_status("Paused · Accessibility required");
            if let Err(error) = (callbacks.open_settings)() {
                status_item::set_status(&format!("Could not open Settings: {error}"));
            }
        }
        agent.set_callbacks(callbacks);
        Ok(agent)
    }
}

pub fn allow_immediate_termination() {
    termination::allow_next();
}

pub fn finish_appkit_termination() {
    termination::reply(true);
}

#[cfg(test)]
mod tests;
