// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The keys the macOS menu bar shows next to its items. The menu is native (`macos_menu.rs`) and can't read the
//! app's shortcut table, so the app sends the key of each command (`set_menu_keys`: the table's answer now, the
//! person's own bindings first), at start and whenever the keybindings change. The menu puts the key on the item
//! with the same command id, and clears it on an item whose command has none.

use parking_lot::RwLock;

/// One command's key: the character AppKit takes as a key equivalent, and the modifiers held with it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct MenuKey {
    pub(crate) command: String,
    /// One lower-case letter, digit or punctuation character; Shift is `shift`, not an upper-case letter.
    pub(crate) key: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

// NSEventModifierFlags.
const SHIFT: u64 = 1 << 17;
const CONTROL: u64 = 1 << 18;
const OPTION: u64 = 1 << 19;
const COMMAND: u64 = 1 << 20;

impl MenuKey {
    /// The `NSEventModifierFlags` of the key's modifiers.
    pub(crate) fn mask(&self) -> u64 {
        [(self.shift, SHIFT), (self.ctrl, CONTROL), (self.alt, OPTION), (self.meta, COMMAND)]
            .into_iter()
            .filter(|(held, _)| *held)
            .fold(0, |m, (_, bit)| m | bit)
    }
}

/// What the app last pushed; empty until it does (an item shows no key).
static PUSHED: RwLock<Vec<MenuKey>> = RwLock::new(Vec::new());

/// A push is a few dozen keys; this keeps a bad one from filling the menu.
const MAX_PUSHED: usize = 256;

/// The keys the app last pushed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn current() -> Vec<MenuKey> {
    PUSHED.read().clone()
}

/// IPC `set_menu_keys`: the app's list. Refuses a list it cannot use rather than keep part of it.
pub(crate) fn set_menu_keys(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let keys: Vec<MenuKey> = serde_json::from_value(args.get("keys").cloned().ok_or("set_menu_keys: missing keys")?)
        .map_err(|e| format!("set_menu_keys: {e}"))?;
    if keys.len() > MAX_PUSHED {
        return Err(format!("set_menu_keys: at most {MAX_PUSHED} keys"));
    }
    // A bare character would take every press of it from the page; a menu key always holds a modifier.
    if let Some(bad) = keys.iter().find(|k| k.key.chars().count() != 1 || k.mask() & (CONTROL | OPTION | COMMAND) == 0) {
        return Err(format!("set_menu_keys: {:?} for {} is not a usable key", bad.key, bad.command));
    }
    let count = keys.len();
    *PUSHED.write() = keys;
    #[cfg(target_os = "macos")]
    crate::macos_menu::apply_menu_keys();
    Ok(serde_json::json!({ "count": count }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(command: &str, key: &str, mods: [bool; 4]) -> serde_json::Value {
        serde_json::json!({ "command": command, "key": key, "ctrl": mods[0], "shift": mods[1], "alt": mods[2], "meta": mods[3] })
    }

    #[test]
    fn a_key_has_the_modifier_flags_appkit_names() {
        let parsed: MenuKey = serde_json::from_value(key("tab:close", "w", [false, true, false, true])).unwrap();
        assert_eq!(parsed.mask(), COMMAND | SHIFT);
        let parsed: MenuKey = serde_json::from_value(key("x", "m", [true, false, true, true])).unwrap();
        assert_eq!(parsed.mask(), CONTROL | OPTION | COMMAND);
    }

    // One test: the list is a process-wide static.
    #[test]
    fn keeps_a_good_list_and_refuses_a_bad_one_whole() {
        let good = serde_json::json!({ "keys": [key("tab:new", "t", [false, false, false, true]), key("app:settings", ",", [false, false, false, true])] });
        assert_eq!(set_menu_keys(&good).unwrap()["count"], 2);
        assert_eq!(current().iter().map(|k| k.command.as_str()).collect::<Vec<_>>(), ["tab:new", "app:settings"]);

        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "keys": [key("a", "t", [false, false, false, false])] }),
            serde_json::json!({ "keys": [key("a", "t", [false, true, false, false])] }),
            serde_json::json!({ "keys": [key("a", "tab", [false, false, false, true])] }),
            serde_json::json!({ "keys": [key("a", "", [false, false, false, true])] }),
            serde_json::json!({ "keys": (0..300).map(|i| key(&format!("c{i}"), "a", [false, false, false, true])).collect::<Vec<_>>() }),
        ] {
            assert!(set_menu_keys(&bad).is_err(), "{bad}");
        }
        // The refused lists changed nothing.
        assert_eq!(current().len(), 2);

        assert_eq!(set_menu_keys(&serde_json::json!({ "keys": [] })).unwrap()["count"], 0);
        assert!(current().is_empty());
    }
}
