// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The modifier keys the person is holding down on the screen. A paste is typed key by key; with Control held (the paste itself is Ctrl+V) every
//! letter would be a shortcut (`a` selects all, `w` closes the tab). So while text is typed the held modifiers are let go, and pressed again after.

use super::keymap;

/// The keys that change what another key means.
const MODIFIERS: [&str; 8] = ["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"];

#[derive(Default)]
pub struct Held(Vec<u8>);

impl Held {
    /// A key went down or up: remember it when it is a modifier.
    pub fn note(&mut self, code: &str, down: bool) {
        let Some(k) = keymap::keycode(code).filter(|_| MODIFIERS.contains(&code)) else { return };
        self.0.retain(|held| *held != k);
        if down {
            self.0.push(k);
        }
    }

    /// The modifier keys down now.
    pub fn keys(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_modifiers_are_remembered_and_a_release_forgets_them() {
        let mut h = Held::default();
        h.note("ControlLeft", true);
        h.note("KeyV", true);
        h.note("ShiftLeft", true);
        assert_eq!(h.keys(), &[keymap::keycode("ControlLeft").unwrap(), keymap::keycode("ShiftLeft").unwrap()]);
        h.note("ControlLeft", false);
        assert_eq!(h.keys(), &[keymap::keycode("ShiftLeft").unwrap()]);
        h.note("ControlLeft", true);
        h.note("ControlLeft", true);
        assert_eq!(h.keys().len(), 2, "a key pressed twice is held once");
    }
}
