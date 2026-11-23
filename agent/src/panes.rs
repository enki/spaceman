//! Settings pane registry plus the General, Workspaces, and Shortcuts panes.

use rune::{El, Icon, Tx, col};
use rune_kit::SegmentedControl;
use spaceman::{
    WorkspaceId,
    app::{UiLaunchAtLogin, UiUpdateStatus},
    config::HudStyle,
};

use crate::{
    Ui, activity,
    controls::{self, ButtonKind, Paint},
    map,
    playground::{Scene, reset_fixture},
    view::scroll_body,
};

static GENERAL_ICON: Icon = Icon {
    name: "spaceman-general",
    view: 16.0,
    body: include_str!("../assets/general.svg"),
};
static RULES_ICON: Icon = Icon {
    name: "spaceman-rules",
    view: 16.0,
    body: include_str!("../assets/rules.svg"),
};
static WORKSPACES_ICON: Icon = Icon {
    name: "spaceman-workspaces",
    view: 16.0,
    body: include_str!("../assets/workspaces.svg"),
};
static ACTIVITY_ICON: Icon = Icon {
    name: "spaceman-activity",
    view: 24.0,
    body: include_str!("../assets/activity.svg"),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    General,
    Rules,
    Workspaces,
    Shortcuts,
    Activity,
}

type PaneRender = fn(&Ui, Paint, &mut Vec<El>) -> (Option<El>, El);

struct PaneSpec {
    key: &'static str,
    title: &'static str,
    icon: &'static Icon,
    render: PaneRender,
}

const fn pane_spec(
    key: &'static str,
    title: &'static str,
    icon: &'static Icon,
    render: PaneRender,
) -> PaneSpec {
    PaneSpec {
        key,
        title,
        icon,
        render,
    }
}

impl Pane {
    pub const ALL: [Pane; 5] = [
        Pane::General,
        Pane::Rules,
        Pane::Workspaces,
        Pane::Shortcuts,
        Pane::Activity,
    ];

    const fn spec(self) -> PaneSpec {
        match self {
            Pane::General => pane_spec("general", "General", &GENERAL_ICON, |ui, p, _| {
                (None, general(ui, p))
            }),
            Pane::Rules => pane_spec("rules", "Rules", &RULES_ICON, |ui, p, layer| {
                (None, map::pane(ui, p, layer))
            }),
            Pane::Workspaces => {
                pane_spec("workspaces", "Workspaces", &WORKSPACES_ICON, |ui, p, _| {
                    (None, workspaces(ui, p))
                })
            }
            Pane::Shortcuts => pane_spec(
                "shortcuts",
                "Shortcuts",
                &rune_theme_opencode::icons::KEYBOARD,
                |_, p, _| (None, shortcuts(p)),
            ),
            Pane::Activity => pane_spec("activity", "Activity", &ACTIVITY_ICON, |ui, p, _| {
                (Some(activity::header_actions(ui, p)), activity::pane(ui, p))
            }),
        }
    }

    pub const fn key(self) -> &'static str {
        self.spec().key
    }

    pub const fn title(self) -> &'static str {
        self.spec().title
    }

    pub const fn icon(self) -> &'static Icon {
        self.spec().icon
    }

    pub fn render(self, ui: &Ui, p: Paint, layer: &mut Vec<El>) -> (Option<El>, El) {
        (self.spec().render)(ui, p, layer)
    }

    /// Maps `⌘1`..`⌘N` in sidebar order.
    pub fn from_shortcut(digit: char) -> Option<Pane> {
        let index = digit.to_digit(10)?.checked_sub(1)? as usize;
        Self::ALL.get(index).copied()
    }
}

pub(crate) const SCENES: &[Scene] = &[
    ("general", |ui| reset_fixture(ui, Pane::General)),
    ("general-setup", |ui| {
        reset_fixture(ui, Pane::General);
        if let Some(agent) = &mut ui.agent {
            agent.accessibility = false;
            agent.automatic = false;
            agent.launch_at_login = UiLaunchAtLogin::Off;
        }
    }),
    ("general-update-ready", |ui| {
        reset_fixture(ui, Pane::General);
        if let Some(agent) = &mut ui.agent {
            agent.update = UiUpdateStatus::Ready {
                version: "0.3.3".into(),
            };
        }
    }),
    ("workspaces", |ui| reset_fixture(ui, Pane::Workspaces)),
    ("shortcuts", |ui| reset_fixture(ui, Pane::Shortcuts)),
];

fn general(ui: &Ui, p: Paint) -> El {
    let c = &p.k.c;
    let mut sections = Vec::new();
    if let Some(agent) = &ui.agent {
        let access_name = spaceman::permission_guide::list_name();
        let access = if agent.accessibility {
            controls::setting_row(
                p,
                "accessibility",
                access_name,
                Some(("Granted. Spaceman can move windows.".into(), p.muted())),
                controls::status_dot(c.success.fg),
            )
        } else {
            controls::setting_row(
                p,
                "accessibility",
                access_name,
                Some((
                    "Spaceman needs it to move and resize windows.".into(),
                    c.warning.fg,
                )),
                controls::button(
                    p,
                    "grant",
                    "Open System Settings",
                    ButtonKind::Primary,
                    |ui, _| ui.open_accessibility(),
                ),
            )
        };
        let tiling = if agent.accessibility {
            controls::toggle_row(
                p,
                "tiling",
                "Automatic tiling",
                Some((
                    if agent.automatic {
                        "On. New windows tile as they open.".into()
                    } else {
                        "Paused. Parked windows are restored.".into()
                    },
                    p.muted(),
                )),
                agent.automatic,
                |ui| ui.toggle_automatic(),
            )
        } else {
            controls::setting_row(
                p,
                "tiling",
                "Automatic tiling",
                Some((
                    "Starts once Accessibility is granted and the hotkeys register.".into(),
                    p.muted(),
                )),
                controls::status_dot(c.border_strong),
            )
        };
        let (on, description) = match &agent.launch_at_login {
            UiLaunchAtLogin::On => (true, None),
            UiLaunchAtLogin::Off => (false, None),
            UiLaunchAtLogin::NeedsApproval => (
                true,
                Some((
                    "Waiting for approval in System Settings › Login Items.".to_owned(),
                    c.warning.fg,
                )),
            ),
            UiLaunchAtLogin::Failed(error) => (false, Some((error.clone(), c.danger.fg))),
        };
        let login = controls::toggle_row(p, "login", "Launch at Login", description, on, |ui| {
            ui.toggle_launch_at_login();
        });
        let hud = controls::toggle_row(
            p,
            "hud",
            "Workspace HUD",
            Some((
                "Show a non-activating capsule when switching workspaces or layouts.".into(),
                p.muted(),
            )),
            ui.doc.hud(),
            |ui| ui.toggle_hud(),
        );
        let hud_style = controls::setting_row(
            p,
            "hud-style",
            "HUD style",
            Some((
                "Each workspace as its number and app icons, or as its layout.".into(),
                p.muted(),
            )),
            SegmentedControl::new(
                &HUD_STYLES.map(|(label, _)| label.to_owned()),
                HUD_STYLES
                    .iter()
                    .position(|(_, style)| *style == ui.doc.hud_style()),
            )
            .fit()
            .on_pick(|ui: &mut Ui, index, _: &mut Tx| {
                if let Some((_, style)) = HUD_STYLES.get(index) {
                    ui.set_hud_style(*style);
                }
            })
            .build(p.k)
            .key("hud-style-picker"),
        );
        let setup_tour = controls::setting_row(
            p,
            "setup-tour",
            "Setup tour",
            Some(("Walk through permissions and shortcuts.".into(), p.muted())),
            controls::button(
                p,
                "run-tour",
                "Run Setup Tour…",
                ButtonKind::Secondary,
                |ui, _| ui.start_tour(),
            ),
        );
        sections.push(controls::section(
            p,
            "agent",
            "Spaceman",
            vec![access, tiling, hud, hud_style, login, setup_tour],
        ));
    }
    sections.push(crate::updates::section(ui, p));
    sections.push(controls::section(
        p,
        "config",
        "Configuration",
        vec![controls::setting_row(
            p,
            "file",
            "config.toml",
            Some((
                "Rules and workspaces save here. Edits from an editor show up too.".into(),
                p.muted(),
            )),
            controls::button(
                p,
                "reveal",
                "Show in Finder",
                ButtonKind::Secondary,
                |_: &mut Ui, _: &mut Tx| {
                    if let Some(path) = spaceman::config::config_path() {
                        controls::reveal_in_finder(&path);
                    }
                },
            ),
        )],
    ));
    scroll_body(
        "general-body",
        col().key("sections").gap(24.0).children(sections),
    )
}

const MONITORS: [&str; 2] = ["Main", "Secondary"];
const HUD_STYLES: [(&str, HudStyle); 2] =
    [("Icons", HudStyle::Icons), ("Layouts", HudStyle::Layouts)];

/// Built-in default display for `workspace` when `config.toml` sets no explicit `monitor`.
const fn default_monitor(workspace: WorkspaceId) -> &'static str {
    match workspace.number() {
        Some(n) if n >= 6 => "secondary",
        _ => "main",
    }
}

/// Which display `workspace` currently targets: explicit override from `config.toml`, or its
/// built-in default (1–5 and named workspaces on Main, 6–10 on Secondary).
fn effective_monitor(doc: &crate::rules::Doc, workspace: WorkspaceId) -> String {
    doc.monitor(workspace)
        .unwrap_or_else(|| default_monitor(workspace).to_owned())
}

fn workspaces(ui: &Ui, p: Paint) -> El {
    let all = ui.doc.workspaces_with_referenced(WorkspaceId::numbered());
    let mut main_rows = Vec::new();
    let mut secondary_rows = Vec::new();
    let mut custom_rows = Vec::new();
    for workspace in all {
        let target = effective_monitor(&ui.doc, workspace);
        let row = monitor_row(p, workspace, &target);
        match target.as_str() {
            "main" => main_rows.push(row),
            "secondary" => secondary_rows.push(row),
            _ => custom_rows.push(row),
        }
    }
    let mut sections = Vec::new();
    if !main_rows.is_empty() {
        sections.push(
            col()
                .key("main-display")
                .gap(8.0)
                .child(controls::heading(
                    p,
                    "main-header",
                    "Main display",
                    Some("Default for 1–5"),
                ))
                .child(controls::card(p, "main-card", main_rows)),
        );
    }
    if !secondary_rows.is_empty() {
        sections.push(
            col()
                .key("secondary-display")
                .gap(8.0)
                .child(controls::heading(
                    p,
                    "secondary-header",
                    "Secondary display",
                    Some("Default for 6–10 · main when unplugged"),
                ))
                .child(controls::card(p, "secondary-card", secondary_rows)),
        );
    }
    if !custom_rows.is_empty() {
        sections.push(
            col()
                .key("custom-display")
                .gap(8.0)
                .child(controls::heading(
                    p,
                    "custom-header",
                    "Named displays",
                    Some("Configured in config.toml"),
                ))
                .child(controls::card(p, "custom-card", custom_rows)),
        );
    }
    scroll_body(
        "workspaces-body",
        col().key("sections").gap(24.0).children(sections),
    )
}

fn monitor_row(p: Paint, workspace: WorkspaceId, effective: &str) -> El {
    let mut labels: Vec<String> = MONITORS.iter().map(|label| (*label).to_owned()).collect();
    let selected = match effective {
        "main" => 0,
        "secondary" => 1,
        name => {
            labels.push(name.to_owned());
            2
        }
    };
    let picker = SegmentedControl::new(&labels, Some(selected))
        .fit()
        .on_pick(move |ui: &mut Ui, index, _: &mut Tx| {
            let chosen = match index {
                0 => "main",
                1 => "secondary",
                _ => return,
            };
            // Picking a workspace's built-in default clears any redundant override in config.toml.
            let override_value = (chosen != default_monitor(workspace)).then_some(chosen);
            ui.edit(|doc| doc.set_monitor(workspace, override_value));
        })
        .build(p.k)
        .key("monitor");
    let title = match workspace.number() {
        Some(10) => "Workspace 10 (⌥0)".to_owned(),
        Some(_) => format!("Workspace {workspace}"),
        None => format!("Workspace {workspace} (named)"),
    };
    let description = (effective != default_monitor(workspace)).then(|| {
        let default_label = if default_monitor(workspace) == "secondary" {
            "Secondary"
        } else {
            "Main"
        };
        (format!("Default: {default_label}"), p.faint())
    });
    controls::setting_row(
        p,
        &format!("workspace-{workspace}"),
        &title,
        description,
        picker,
    )
}

/// The README's hotkeys.
const SHORTCUTS: &[(&str, &[&[&str]])] = &[
    ("Switch to workspace 1–10", &[&["⌥", "1…9, 0"]]),
    ("Move window to workspace 1–10", &[&["⌥", "⇧", "1…9, 0"]]),
    ("Switch to a named workspace", &[&["⌥", "A…Z"]]),
    ("Move window to a named workspace", &[&["⌥", "⇧", "A…Z"]]),
    ("Previous workspace on this display", &[&["⌥", "Tab"]]),
    ("Move workspace to next display", &[&["⌥", "⇧", "Tab"]]),
    ("Next window in the workspace", &[&["⌥", "`"]]),
    ("Focus left, down, up, right", &[&["⌥", "H J K L"]]),
    ("Move tile left, down, up, right", &[&["⌥", "⇧", "H J K L"]]),
    ("Flip the split", &[&["⌥", "/"]]),
    ("Shrink or grow the tile", &[&["⌥", "-"], &["⌥", "="]]),
    ("Fullscreen layout", &[&["⌥", "⇧", "F"]]),
];

fn shortcuts(p: Paint) -> El {
    let rows = controls::shortcut_rows(p, "shortcut", SHORTCUTS);
    scroll_body(
        "shortcuts-body",
        col()
            .key("sections")
            .gap(8.0)
            .child(controls::heading(
                p,
                "header",
                "Hotkeys",
                Some("Follow your keyboard layout"),
            ))
            .child(controls::card(p, "card", rows)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Doc;

    #[test]
    fn workspace_monitor_defaults_split_main_and_secondary() {
        let doc = Doc::parse("version = 1\n");
        for number in 1..=5 {
            let ws = WorkspaceId::new(number).unwrap();
            assert_eq!(default_monitor(ws), "main");
            assert_eq!(effective_monitor(&doc, ws), "main");
        }
        for number in 6..=10 {
            let ws = WorkspaceId::new(number).unwrap();
            assert_eq!(default_monitor(ws), "secondary");
            assert_eq!(effective_monitor(&doc, ws), "secondary");
        }
        let named = WorkspaceId::named('M').unwrap();
        assert_eq!(default_monitor(named), "main");
        assert_eq!(effective_monitor(&doc, named), "main");

        let mut overridden = doc;
        overridden.set_monitor(WorkspaceId::new(2).unwrap(), Some("secondary"));
        overridden.set_monitor(WorkspaceId::new(7).unwrap(), Some("main"));
        assert_eq!(
            effective_monitor(&overridden, WorkspaceId::new(2).unwrap()),
            "secondary"
        );
        assert_eq!(
            effective_monitor(&overridden, WorkspaceId::new(7).unwrap()),
            "main"
        );
    }
}
