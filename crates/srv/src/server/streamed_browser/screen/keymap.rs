// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keys as the X server numbers them. The pane sends a key's DOM `code` (the physical key: `KeyA`, `ArrowLeft`); the X server under
//! Xvfb numbers keys as Linux does plus 8. Layout is US: a key is the key at that place, not the letter a layout would give it.

/// DOM key codes and the Linux scancodes of those keys.
const SCANCODES: &[(&str, u8)] = &[
    ("Escape", 1),
    ("Digit1", 2),
    ("Digit2", 3),
    ("Digit3", 4),
    ("Digit4", 5),
    ("Digit5", 6),
    ("Digit6", 7),
    ("Digit7", 8),
    ("Digit8", 9),
    ("Digit9", 10),
    ("Digit0", 11),
    ("Minus", 12),
    ("Equal", 13),
    ("Backspace", 14),
    ("Tab", 15),
    ("KeyQ", 16),
    ("KeyW", 17),
    ("KeyE", 18),
    ("KeyR", 19),
    ("KeyT", 20),
    ("KeyY", 21),
    ("KeyU", 22),
    ("KeyI", 23),
    ("KeyO", 24),
    ("KeyP", 25),
    ("BracketLeft", 26),
    ("BracketRight", 27),
    ("Enter", 28),
    ("ControlLeft", 29),
    ("KeyA", 30),
    ("KeyS", 31),
    ("KeyD", 32),
    ("KeyF", 33),
    ("KeyG", 34),
    ("KeyH", 35),
    ("KeyJ", 36),
    ("KeyK", 37),
    ("KeyL", 38),
    ("Semicolon", 39),
    ("Quote", 40),
    ("Backquote", 41),
    ("ShiftLeft", 42),
    ("Backslash", 43),
    ("KeyZ", 44),
    ("KeyX", 45),
    ("KeyC", 46),
    ("KeyV", 47),
    ("KeyB", 48),
    ("KeyN", 49),
    ("KeyM", 50),
    ("Comma", 51),
    ("Period", 52),
    ("Slash", 53),
    ("ShiftRight", 54),
    ("NumpadMultiply", 55),
    ("AltLeft", 56),
    ("Space", 57),
    ("CapsLock", 58),
    ("F1", 59),
    ("F2", 60),
    ("F3", 61),
    ("F4", 62),
    ("F5", 63),
    ("F6", 64),
    ("F7", 65),
    ("F8", 66),
    ("F9", 67),
    ("F10", 68),
    ("Numpad7", 71),
    ("Numpad8", 72),
    ("Numpad9", 73),
    ("NumpadSubtract", 74),
    ("Numpad4", 75),
    ("Numpad5", 76),
    ("Numpad6", 77),
    ("NumpadAdd", 78),
    ("Numpad1", 79),
    ("Numpad2", 80),
    ("Numpad3", 81),
    ("Numpad0", 82),
    ("NumpadDecimal", 83),
    ("F11", 87),
    ("F12", 88),
    ("NumpadEnter", 96),
    ("ControlRight", 97),
    ("NumpadDivide", 98),
    ("AltRight", 100),
    ("Home", 102),
    ("ArrowUp", 103),
    ("PageUp", 104),
    ("ArrowLeft", 105),
    ("ArrowRight", 106),
    ("End", 107),
    ("ArrowDown", 108),
    ("PageDown", 109),
    ("Insert", 110),
    ("Delete", 111),
    ("MetaLeft", 125),
    ("MetaRight", 126),
    ("ContextMenu", 127),
];

/// The X keycode of a DOM key code, or none for a key this prototype does not know.
pub fn keycode(code: &str) -> Option<u8> {
    SCANCODES.iter().find(|(name, _)| *name == code).map(|(_, evdev)| evdev + 8)
}

/// The keycode of Left Shift, and of Left Control and Left Alt, for the keys srv presses itself (an address typed in, a paste).
pub const SHIFT: u8 = 42 + 8;
pub const CONTROL: u8 = 29 + 8;
pub const ALT: u8 = 56 + 8;

/// The key (and whether Shift is held) that types `c` on a US keyboard, for the characters of an address or a short paste.
pub fn ascii(c: char) -> Option<(u8, bool)> {
    let dom = |code: &str, shift: bool| keycode(code).map(|k| (k, shift));
    match c {
        'a'..='z' => dom(&format!("Key{}", c.to_ascii_uppercase()), false),
        'A'..='Z' => dom(&format!("Key{c}"), true),
        '0'..='9' => dom(&format!("Digit{c}"), false),
        ' ' => dom("Space", false),
        '\n' | '\r' => dom("Enter", false),
        '\t' => dom("Tab", false),
        '-' => dom("Minus", false),
        '_' => dom("Minus", true),
        '=' => dom("Equal", false),
        '+' => dom("Equal", true),
        '[' => dom("BracketLeft", false),
        '{' => dom("BracketLeft", true),
        ']' => dom("BracketRight", false),
        '}' => dom("BracketRight", true),
        ';' => dom("Semicolon", false),
        ':' => dom("Semicolon", true),
        '\'' => dom("Quote", false),
        '"' => dom("Quote", true),
        '`' => dom("Backquote", false),
        '~' => dom("Backquote", true),
        '\\' => dom("Backslash", false),
        '|' => dom("Backslash", true),
        ',' => dom("Comma", false),
        '<' => dom("Comma", true),
        '.' => dom("Period", false),
        '>' => dom("Period", true),
        '/' => dom("Slash", false),
        '?' => dom("Slash", true),
        '!' => dom("Digit1", true),
        '@' => dom("Digit2", true),
        '#' => dom("Digit3", true),
        '$' => dom("Digit4", true),
        '%' => dom("Digit5", true),
        '^' => dom("Digit6", true),
        '&' => dom("Digit7", true),
        '*' => dom("Digit8", true),
        '(' => dom("Digit9", true),
        ')' => dom("Digit0", true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dom_key_code_is_the_linux_scancode_plus_eight() {
        assert_eq!(keycode("KeyA"), Some(38));
        assert_eq!(keycode("Enter"), Some(36));
        assert_eq!(keycode("ArrowLeft"), Some(113));
        assert_eq!(keycode("ShiftLeft"), Some(SHIFT));
        assert_eq!(keycode("ControlLeft"), Some(CONTROL));
        assert_eq!(keycode("AltLeft"), Some(ALT));
        assert_eq!(keycode("NoSuchKey"), None);
        assert_eq!(keycode(""), None);
    }

    #[test]
    fn no_two_keys_share_a_keycode() {
        let codes = [
            "Escape", "Digit1", "Digit0", "Minus", "Equal", "Backspace", "Tab", "KeyQ", "KeyP", "BracketLeft", "BracketRight", "Enter", "ControlLeft", "KeyA", "KeyL",
            "Semicolon", "Quote", "Backquote", "ShiftLeft", "Backslash", "KeyZ", "KeyM", "Comma", "Period", "Slash", "ShiftRight", "AltLeft", "Space", "F1", "F10", "F11", "F12",
            "Home", "ArrowUp", "PageUp", "ArrowLeft", "ArrowRight", "End", "ArrowDown", "PageDown", "Insert", "Delete", "ControlRight", "AltRight",
        ];
        let mut seen = std::collections::HashSet::new();
        for c in codes {
            let k = keycode(c).unwrap_or_else(|| panic!("{c} has no keycode"));
            assert!(seen.insert(k), "{c} repeats keycode {k}");
        }
    }

    #[test]
    fn an_address_can_be_typed_on_a_us_keyboard() {
        for c in "https://example.com/a_b-c?d=e&f=1#x%20y:8080~+@!$*(),;'".chars() {
            assert!(ascii(c).is_some(), "{c:?} has no key");
        }
        assert_eq!(ascii('a'), Some((38, false)));
        assert_eq!(ascii('A'), Some((38, true)));
        assert_eq!(ascii(':'), Some((keycode("Semicolon").unwrap(), true)));
        assert_eq!(ascii('\u{e9}'), None);
    }
}
