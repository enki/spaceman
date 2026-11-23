mod delivery;

use std::{collections::BTreeMap, ffi::c_void, ptr, sync::mpsc::Sender};

pub(crate) use delivery::DeliveryWatch;

use crate::{
    Direction, DisplayTarget, ResizeDirection, WorkspaceId,
    app::{Action, WorkerAction},
    keyboard_layout::{LayoutFingerprint, LayoutSnapshot, LogicalKey, SemanticKey},
};

const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const EVENT_HOTKEY_PRESSED: u32 = 5;
pub(crate) const COMMAND_KEY: u32 = 1 << 8;
pub(crate) const SHIFT_KEY: u32 = 1 << 9;
pub(crate) const OPTION_KEY: u32 = 1 << 11;
pub(crate) const CONTROL_KEY: u32 = 1 << 12;
const SIGNATURE: u32 = u32::from_be_bytes(*b"SPMN");

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type EventHandler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn InstallEventHandler(
        target: *mut c_void,
        handler: EventHandler,
        event_type_count: u32,
        event_types: *const EventTypeSpec,
        user_data: *mut c_void,
        handler_ref: *mut *mut c_void,
    ) -> i32;
    fn RemoveEventHandler(handler_ref: *mut c_void) -> i32;
    fn GetApplicationEventTarget() -> *mut c_void;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        hotkey_id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        hotkey_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey_ref: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: u32,
        actual_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}

pub struct Hotkeys {
    registrations: Vec<*mut c_void>,
    handler: *mut c_void,
    context: Option<Box<HandlerContext>>,
    layout_fingerprint: LayoutFingerprint,
    /// A hotkey's ID is its index here.
    bindings: Vec<Registration>,
}

/// Read by the Carbon handler.
struct HandlerContext {
    sender: Sender<Action>,
}

#[derive(Debug)]
pub(crate) struct HotkeyPlan {
    registrations: Vec<Registration>,
    layout_fingerprint: LayoutFingerprint,
}

#[derive(Clone, Debug)]
struct Registration {
    key_code: u32,
    modifiers: u32,
    action: WorkerAction,
    description: String,
}

impl HotkeyPlan {
    pub(crate) fn current(enabled: &[WorkspaceId]) -> Result<Self, String> {
        let layout = LayoutSnapshot::current().map_err(|error| error.to_string())?;
        Self::from_layout(enabled, &layout)
    }

    fn from_layout(enabled: &[WorkspaceId], layout: &LayoutSnapshot) -> Result<Self, String> {
        let mut registrations = Vec::with_capacity(15 + enabled.len() * 2);
        let claimed_letters = enabled
            .iter()
            .filter_map(|workspace| workspace.letter().map(|letter| letter.to_ascii_lowercase()))
            .collect::<Vec<_>>();
        for workspace in WorkspaceId::all().filter(|workspace| enabled.contains(workspace)) {
            let key = match (workspace.number(), workspace.letter()) {
                (Some(number), _) => LogicalKey::Semantic(SemanticKey::Workspace(number)),
                (_, Some(letter)) => LogicalKey::Character(letter.to_ascii_lowercase()),
                _ => unreachable!("workspace ID is always valid"),
            };
            let key = layout.resolve(key).map_err(|error| error.to_string())?;
            registrations.push(registration(
                key,
                OPTION_KEY,
                WorkerAction::SwitchWorkspace(workspace),
                format!("switch workspace {workspace}"),
            ));
            registrations.push(registration(
                key,
                OPTION_KEY | SHIFT_KEY,
                WorkerAction::MoveFocused(workspace),
                format!("move window to workspace {workspace}"),
            ));
        }
        for (character, direction, name) in [
            ('h', Direction::Left, "left"),
            ('j', Direction::Down, "down"),
            ('k', Direction::Up, "up"),
            ('l', Direction::Right, "right"),
        ] {
            if claimed_letters.contains(&character) {
                continue;
            }
            let key = layout
                .resolve(LogicalKey::Character(character))
                .map_err(|error| error.to_string())?;
            registrations.push(registration(
                key,
                OPTION_KEY,
                WorkerAction::Focus(direction),
                format!("focus {name}"),
            ));
            registrations.push(registration(
                key,
                OPTION_KEY | SHIFT_KEY,
                WorkerAction::Reorder(direction),
                format!("move window {name}"),
            ));
        }
        for (key, modifiers, action, description) in [
            (
                LogicalKey::Character('f'),
                OPTION_KEY | SHIFT_KEY,
                WorkerAction::ToggleFullscreenLayout,
                "toggle fullscreen layout",
            ),
            (
                LogicalKey::Character('/'),
                OPTION_KEY,
                WorkerAction::ToggleLayout,
                "toggle layout",
            ),
            (
                LogicalKey::Character('`'),
                OPTION_KEY,
                WorkerAction::CycleFocus,
                "cycle workspace windows",
            ),
            (
                LogicalKey::Semantic(SemanticKey::Tab),
                OPTION_KEY,
                WorkerAction::WorkspaceBackAndForth,
                "workspace back-and-forth",
            ),
            (
                LogicalKey::Semantic(SemanticKey::Tab),
                OPTION_KEY | SHIFT_KEY,
                WorkerAction::MoveWorkspaceToDisplay(DisplayTarget::Next),
                "move workspace to next display",
            ),
            (
                LogicalKey::Character('-'),
                OPTION_KEY,
                WorkerAction::Resize(ResizeDirection::Shrink),
                "shrink layout",
            ),
            (
                LogicalKey::Character('='),
                OPTION_KEY,
                WorkerAction::Resize(ResizeDirection::Grow),
                "grow layout",
            ),
        ] {
            if matches!(key, LogicalKey::Character(character) if claimed_letters.contains(&character))
            {
                continue;
            }
            let key = layout.resolve(key).map_err(|error| error.to_string())?;
            registrations.push(registration(key, modifiers, action, description.into()));
        }

        reject_collisions(&registrations)?;
        Ok(Self {
            registrations,
            layout_fingerprint: layout.fingerprint().clone(),
        })
    }

    pub(crate) fn matches(&self, hotkeys: &Hotkeys) -> bool {
        self.layout_fingerprint == hotkeys.layout_fingerprint
    }
}

fn registration(
    key: crate::keyboard_layout::ResolvedKey,
    semantic_modifiers: u32,
    action: WorkerAction,
    description: String,
) -> Registration {
    Registration {
        key_code: key.key_code,
        modifiers: semantic_modifiers | if key.shift { SHIFT_KEY } else { 0 },
        action,
        description,
    }
}

fn reject_collisions(registrations: &[Registration]) -> Result<(), String> {
    let mut occupied = BTreeMap::new();
    for registration in registrations {
        if let Some(previous) = occupied.insert(
            (registration.key_code, registration.modifiers),
            registration.description.as_str(),
        ) {
            return Err(format!(
                "keyboard layout maps {previous} and {} to virtual key {:#04x} with modifiers {:#06x}",
                registration.description, registration.key_code, registration.modifiers
            ));
        }
    }
    Ok(())
}

impl Hotkeys {
    pub fn install(sender: Sender<Action>, plan: HotkeyPlan) -> Result<Self, String> {
        let context = Box::new(HandlerContext { sender });
        let context_pointer = (&raw const *context).cast_mut();
        let event_type = EventTypeSpec {
            event_class: EVENT_CLASS_KEYBOARD,
            event_kind: EVENT_HOTKEY_PRESSED,
        };
        let mut handler = ptr::null_mut();
        // SAFETY: `event_type` lives across installation, `context` stays heap-pinned in `self`
        // (or is leaked if removal fails), and `handler` is a valid out-pointer.
        let status = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                hotkey_handler,
                1,
                &raw const event_type,
                context_pointer.cast(),
                &raw mut handler,
            )
        };
        if status != 0 {
            return Err(format!("failed to install hotkey handler ({status})"));
        }

        let mut hotkeys = Self {
            registrations: Vec::with_capacity(plan.registrations.len()),
            handler,
            context: Some(context),
            layout_fingerprint: plan.layout_fingerprint,
            bindings: Vec::new(),
        };
        for (id, registration) in (0..).zip(&plan.registrations) {
            hotkeys.register(registration.key_code, registration.modifiers, id)?;
        }
        hotkeys.bindings = plan.registrations;
        Ok(hotkeys)
    }

    pub(crate) fn len(&self) -> usize {
        self.bindings.len()
    }

    /// The action and description of the hotkey Carbon delivered as `id`.
    pub(crate) fn binding(&self, id: u32) -> Option<(&WorkerAction, &str)> {
        let registration = self.bindings.get(usize::try_from(id).ok()?)?;
        Some((&registration.action, &registration.description))
    }

    /// The hotkey registered for this key and these Carbon modifiers.
    pub(crate) fn id_for(&self, key_code: u32, modifiers: u32) -> Option<u32> {
        let index = self.bindings.iter().position(|registration| {
            registration.key_code == key_code && registration.modifiers == modifiers
        })?;
        u32::try_from(index).ok()
    }

    pub fn matches_current_layout(&self) -> Result<bool, String> {
        LayoutSnapshot::current_fingerprint()
            .map(|fingerprint| self.layout_fingerprint == fingerprint)
            .map_err(|error| error.to_string())
    }

    fn register(&mut self, key_code: u32, modifiers: u32, id: u32) -> Result<(), String> {
        let mut registration = ptr::null_mut();
        // SAFETY: Carbon's application event target is valid on the main thread and `registration`
        // is a valid out-pointer.
        let status = unsafe {
            RegisterEventHotKey(
                key_code,
                modifiers,
                EventHotKeyId {
                    signature: SIGNATURE,
                    id,
                },
                GetApplicationEventTarget(),
                0,
                &raw mut registration,
            )
        };
        if status != 0 {
            return Err(format!("hotkey is unavailable ({status})"));
        }
        self.registrations.push(registration);
        Ok(())
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            // SAFETY: Each registration handle came from a successful `RegisterEventHotKey` call.
            unsafe { UnregisterEventHotKey(registration) };
        }
        // SAFETY: `self.handler` came from a successful `InstallEventHandler` call.
        let removed = unsafe { RemoveEventHandler(self.handler) };
        if removed != 0
            && let Some(context) = self.context.take()
        {
            Box::leak(context);
        }
    }
}

unsafe extern "C" fn hotkey_handler(
    _: *mut c_void,
    event: *mut c_void,
    user_data: *mut c_void,
) -> i32 {
    // SAFETY: `user_data` points to the `HandlerContext` boxed in `Hotkeys::install`, which
    // outlives the installed Carbon event handler.
    let context = unsafe { &*user_data.cast::<HandlerContext>() };
    let mut hotkey_id = EventHotKeyId {
        signature: 0,
        id: 0,
    };
    // SAFETY: `event` is provided by Carbon for `EVENT_HOTKEY_PRESSED`, and `hotkey_id` matches
    // the expected `EventHotKeyId` layout and byte count.
    let status = unsafe {
        GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            ptr::null_mut(),
            u32::try_from(std::mem::size_of::<EventHotKeyId>()).unwrap_or_default(),
            ptr::null_mut(),
            (&raw mut hotkey_id).cast(),
        )
    };
    if status == 0 && hotkey_id.signature == SIGNATURE {
        drop(context.sender.send(Action::Hotkey(hotkey_id.id)));
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> LayoutSnapshot {
        LayoutSnapshot::for_test(
            vec![
                (60, 'h', false, false),
                (61, 'j', false, false),
                (62, 'k', false, false),
                (63, 'l', false, false),
                (64, 'f', false, false),
                (65, '/', false, false),
                (66, '-', false, false),
                (67, '=', false, false),
                (68, '`', false, false),
            ],
            vec![],
        )
    }

    fn action<'a>(plan: &'a HotkeyPlan, description: &str) -> Option<&'a WorkerAction> {
        plan.registrations
            .iter()
            .find(|registration| registration.description == description)
            .map(|registration| &registration.action)
    }

    #[test]
    fn registrations_carry_switch_move_and_layout_actions_including_zero_key() {
        let enabled = WorkspaceId::numbered().collect::<Vec<_>>();
        let plan = HotkeyPlan::from_layout(&enabled, &layout()).unwrap();
        let one = WorkspaceId::new(1).unwrap();
        let ten = WorkspaceId::new(10).unwrap();
        for (description, expected) in [
            ("switch workspace 1", WorkerAction::SwitchWorkspace(one)),
            (
                "toggle fullscreen layout",
                WorkerAction::ToggleFullscreenLayout,
            ),
            ("toggle layout", WorkerAction::ToggleLayout),
            (
                "workspace back-and-forth",
                WorkerAction::WorkspaceBackAndForth,
            ),
            (
                "move workspace to next display",
                WorkerAction::MoveWorkspaceToDisplay(DisplayTarget::Next),
            ),
            (
                "shrink layout",
                WorkerAction::Resize(ResizeDirection::Shrink),
            ),
            ("grow layout", WorkerAction::Resize(ResizeDirection::Grow)),
            ("cycle workspace windows", WorkerAction::CycleFocus),
            ("focus left", WorkerAction::Focus(Direction::Left)),
            ("focus right", WorkerAction::Focus(Direction::Right)),
            ("move window left", WorkerAction::Reorder(Direction::Left)),
            ("move window right", WorkerAction::Reorder(Direction::Right)),
            ("switch workspace 10", WorkerAction::SwitchWorkspace(ten)),
            (
                "move window to workspace 10",
                WorkerAction::MoveFocused(ten),
            ),
        ] {
            assert_eq!(action(&plan, description), Some(&expected), "{description}");
        }
    }

    #[test]
    fn full_plan_contains_all_thirty_five_registrations() {
        let enabled = WorkspaceId::numbered().collect::<Vec<_>>();
        let plan = HotkeyPlan::from_layout(&enabled, &layout()).unwrap();
        assert_eq!(plan.registrations.len(), 35);
        assert!(plan.registrations.capacity() >= 35);
        let move_ws = plan
            .registrations
            .iter()
            .find(|registration| {
                registration.action == WorkerAction::MoveWorkspaceToDisplay(DisplayTarget::Next)
            })
            .unwrap();
        assert_eq!(move_ws.modifiers, OPTION_KEY | SHIFT_KEY);
    }

    #[test]
    fn named_workspace_claims_its_letter_shortcuts() {
        let mut enabled = WorkspaceId::numbered().collect::<Vec<_>>();
        enabled.push(WorkspaceId::named('H').unwrap());
        let plan = HotkeyPlan::from_layout(&enabled, &layout()).unwrap();
        let h = WorkspaceId::named('H').unwrap();
        assert_eq!(
            action(&plan, "switch workspace H"),
            Some(&WorkerAction::SwitchWorkspace(h))
        );
        assert_eq!(
            action(&plan, "move window to workspace H"),
            Some(&WorkerAction::MoveFocused(h))
        );
        assert_eq!(action(&plan, "focus left"), None);
        assert_eq!(action(&plan, "move window left"), None);
    }

    #[test]
    fn layout_required_shift_is_composed_with_command_modifiers() {
        let layout = LayoutSnapshot::for_test(
            vec![
                (60, 'h', false, false),
                (61, 'j', false, false),
                (62, 'k', false, false),
                (63, 'l', false, false),
                (64, 'f', false, false),
                (65, '/', true, false),
                (66, '-', false, false),
                (67, '=', false, false),
                (68, '`', false, false),
            ],
            vec![],
        );
        let plan = HotkeyPlan::from_layout(&[], &layout).unwrap();
        let toggle_layout = plan
            .registrations
            .iter()
            .find(|registration| registration.action == WorkerAction::ToggleLayout)
            .unwrap();
        assert_eq!(toggle_layout.modifiers, OPTION_KEY | SHIFT_KEY);
    }

    #[test]
    fn duplicate_chords_are_rejected_before_registration() {
        let layout = LayoutSnapshot::for_test(
            vec![
                (60, 'h', false, false),
                (60, 'j', false, false),
                (62, 'k', false, false),
                (63, 'l', false, false),
                (64, 'f', false, false),
                (65, '/', false, false),
                (66, '-', false, false),
                (67, '=', false, false),
                (68, '`', false, false),
            ],
            vec![],
        );
        let error = HotkeyPlan::from_layout(&[], &layout).unwrap_err();
        assert!(error.contains("focus left and focus down"), "{error}");
    }

    #[test]
    fn unsupported_layout_cannot_produce_a_replacement_plan() {
        let non_latin_without_ascii_fallback = LayoutSnapshot::for_test(
            vec![
                (60, 'ж', false, false),
                (61, 'д', false, false),
                (62, 'л', false, false),
                (63, 'о', false, false),
            ],
            vec![],
        );

        let error = HotkeyPlan::from_layout(&[], &non_latin_without_ascii_fallback).unwrap_err();
        assert!(error.contains("does not produce 'h'"), "{error}");
    }
}
