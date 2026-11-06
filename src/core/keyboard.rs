/// Resolve command shortcuts when an IME reports Hangul/non-Latin characters.
/// Printable ASCII from the active layout wins (including Dvorak). This is never
/// used to transform text input, only to dispatch modifier-key commands.
pub fn shortcut_key(code: u16, characters: &str) -> String {
    if characters.len() == 1 && characters.is_ascii() {
        return characters.to_ascii_lowercase();
    }
    match code {
        0 => "a",
        3 => "f",
        6 => "z",
        7 => "x",
        8 => "c",
        9 => "v",
        12 => "q",
        13 => "w",
        15 => "r",
        17 => "t",
        30 => "]",
        33 => "[",
        35 => "p",
        37 => "l",
        43 => ",",
        45 => "n",
        47 => ".",
        24 => "=",
        27 => "-",
        _ => "",
    }
    .into()
}
/// macOS virtual F2 is 120 (F3 is 99); use the function character as well
/// because external keyboards and remapping utilities may report a different code.
pub fn is_rename_key(code: u16, characters: &str) -> bool {
    code == 120 || characters == "\u{f705}"
}
pub fn mouse_history_direction(button: isize) -> Option<bool> {
    match button {
        3 => Some(false),
        4 => Some(true),
        _ => None,
    }
}

/// NX_SYSDEFINED/AUX_CONTROL_BUTTONS represents media keys, not typed text.
/// Only the brightness-increase key (the translated F2) is eligible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemF2Phase {
    Down { repeat: bool },
    Up,
}
pub fn system_f2_phase(subtype: i16, data: isize) -> Option<SystemF2Phase> {
    if subtype != 8 || ((data as u64 >> 16) & 0xffff) != 2 {
        return None;
    }
    match (data >> 8) & 0xff {
        0x0a => Some(SystemF2Phase::Down {
            repeat: data & 1 != 0,
        }),
        0x0b => Some(SystemF2Phase::Up),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemF2Decision {
    Pass,
    Rename,
    Suppress,
}

#[derive(Default)]
pub struct SystemF2Press {
    captured: bool,
}
impl SystemF2Press {
    pub fn handle(&mut self, phase: SystemF2Phase, eligible: bool) -> SystemF2Decision {
        match phase {
            SystemF2Phase::Down { .. } if self.captured => SystemF2Decision::Suppress,
            SystemF2Phase::Down { repeat: false } if eligible => {
                self.captured = true;
                SystemF2Decision::Rename
            }
            SystemF2Phase::Up if self.captured => {
                self.captured = false;
                SystemF2Decision::Suppress
            }
            _ => SystemF2Decision::Pass,
        }
    }
    pub fn reset(&mut self) {
        self.captured = false;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn korean_commands_use_physical_shortcut() {
        assert_eq!(shortcut_key(8, "ㅊ"), "c");
        assert_eq!(shortcut_key(45, "ㅜ"), "n");
        assert_eq!(shortcut_key(37, "ㅣ"), "l");
    }
    #[test]
    fn alternate_ascii_layout_is_preserved() {
        assert_eq!(shortcut_key(8, "J"), "j");
        assert_eq!(shortcut_key(0, ""), "a");
    }
    #[test]
    fn hardware_and_function_characters_resolve_without_f3_alias() {
        assert!(is_rename_key(120, ""));
        assert!(is_rename_key(0, "\u{f705}"));
        assert!(!is_rename_key(99, "\u{f706}"));
        assert_eq!(mouse_history_direction(3), Some(false));
        assert_eq!(mouse_history_direction(4), Some(true));
        assert_eq!(mouse_history_direction(2), None);
    }
    #[test]
    fn only_brightness_increase_is_a_compatibility_key() {
        assert_eq!(
            system_f2_phase(8, 0x20a00),
            Some(SystemF2Phase::Down { repeat: false })
        );
        assert_eq!(
            system_f2_phase(8, 0x20a01),
            Some(SystemF2Phase::Down { repeat: true })
        );
        assert_eq!(system_f2_phase(8, 0x20b00), Some(SystemF2Phase::Up));
        for data in [0x00a00, 0x10a00, 0x30a00, 0x40a00, 0x20c00] {
            assert_eq!(system_f2_phase(8, data), None);
        }
        assert_eq!(system_f2_phase(7, 0x20a00), None);
    }
    #[test]
    fn background_text_and_dialog_contexts_pass_the_entire_press() {
        let mut key = SystemF2Press::default();
        assert_eq!(
            key.handle(SystemF2Phase::Down { repeat: false }, false),
            SystemF2Decision::Pass
        );
        assert_eq!(key.handle(SystemF2Phase::Up, false), SystemF2Decision::Pass);
        // A press started outside the app must not turn into rename on focus gain.
        assert_eq!(
            key.handle(SystemF2Phase::Down { repeat: true }, true),
            SystemF2Decision::Pass
        );
        assert_eq!(key.handle(SystemF2Phase::Up, true), SystemF2Decision::Pass);
    }
    #[test]
    fn one_press_renames_once_even_after_the_editor_takes_focus() {
        let mut key = SystemF2Press::default();
        assert_eq!(
            key.handle(SystemF2Phase::Down { repeat: false }, true),
            SystemF2Decision::Rename
        );
        assert_eq!(
            key.handle(SystemF2Phase::Down { repeat: true }, false),
            SystemF2Decision::Suppress
        );
        assert_eq!(
            key.handle(SystemF2Phase::Up, false),
            SystemF2Decision::Suppress
        );
        assert_eq!(
            key.handle(SystemF2Phase::Down { repeat: false }, false),
            SystemF2Decision::Pass
        );
    }
    #[test]
    fn deactivation_and_shutdown_release_captured_state() {
        let mut key = SystemF2Press::default();
        key.handle(SystemF2Phase::Down { repeat: false }, true);
        key.reset();
        assert_eq!(key.handle(SystemF2Phase::Up, false), SystemF2Decision::Pass);
        assert_eq!(
            key.handle(SystemF2Phase::Down { repeat: false }, true),
            SystemF2Decision::Rename
        );
    }
    #[test]
    fn legacy_preferences_do_not_enable_system_key_access() {
        let old: crate::core::model::Preferences =
            serde_json::from_str(r#"{"keyboard":"Finder"}"#).unwrap();
        assert!(!old.system_f2_rename);
        let updated = crate::core::model::Preferences {
            system_f2_rename: true,
            ..old
        };
        let restored: crate::core::model::Preferences =
            serde_json::from_str(&serde_json::to_string(&updated).unwrap()).unwrap();
        assert!(restored.system_f2_rename);
        assert_eq!(
            restored.keyboard,
            crate::core::model::KeyboardProfile::Finder
        );
    }
}
