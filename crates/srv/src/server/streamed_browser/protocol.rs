// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the pane in the web UI says to srv, and how each message becomes a CDP call.
//! docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4.
//!
//! Everything here is pure: a message that is too long, malformed or out of range is refused with
//! words, never a panic, and a navigation to anything but http or https is refused.

use serde::Deserialize;
use serde_json::{json, Value};

/// The longest text message the pane may send.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// The longest paste (it arrives as a message of its own size class).
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;
/// The smallest and largest side of the pane's viewport.
pub const MIN_SIDE: u32 = 100;
pub const MAX_SIDE: u32 = 4096;

/// A message from the pane.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMsg {
    Mouse {
        #[serde(rename = "type")]
        kind: MouseKind,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        buttons: u32,
        #[serde(default)]
        clicks: u32,
        #[serde(default)]
        mods: u32,
    },
    Wheel { x: f64, y: f64, dx: f64, dy: f64, #[serde(default)] mods: u32 },
    Key {
        #[serde(rename = "type")]
        kind: KeyKind,
        key: String,
        #[serde(default)]
        code: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        vk: u32,
        #[serde(default)]
        mods: u32,
    },
    Paste { text: String },
    Resize { w: u32, h: u32 },
    Nav { go: Go, #[serde(default)] url: String },
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum MouseKind {
    Down,
    Up,
    Move,
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Middle,
    Right,
    #[default]
    None,
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    Down,
    Up,
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Go {
    Url,
    Back,
    Forward,
    Reload,
    Stop,
}

/// Parse one text message, with the size limits applied before any parsing.
pub fn parse(text: &str) -> Result<ClientMsg, String> {
    let paste = text.starts_with(r#"{"t":"paste""#);
    let limit = if paste { MAX_PASTE_BYTES + 64 } else { MAX_MESSAGE_BYTES };
    if text.len() > limit {
        return Err("that message is too long".into());
    }
    let msg: ClientMsg = serde_json::from_str(text).map_err(|_| "that message is not one the pane understands".to_string())?;
    check(&msg)?;
    Ok(msg)
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite() && v.abs() < 1.0e6)
}

fn check(msg: &ClientMsg) -> Result<(), String> {
    match msg {
        ClientMsg::Mouse { x, y, .. } if !finite(&[*x, *y]) => Err("a mouse position is out of range".into()),
        ClientMsg::Wheel { x, y, dx, dy, .. } if !finite(&[*x, *y, *dx, *dy]) => Err("a wheel position is out of range".into()),
        ClientMsg::Resize { w, h } if !(MIN_SIDE..=MAX_SIDE).contains(w) || !(MIN_SIDE..=MAX_SIDE).contains(h) => {
            Err(format!("the pane's size must be between {MIN_SIDE} and {MAX_SIDE}"))
        }
        ClientMsg::Paste { text } if text.len() > MAX_PASTE_BYTES => Err("that paste is too long".into()),
        ClientMsg::Nav { go: Go::Url, url } => check_url(url).map(|_| ()),
        _ => Ok(()),
    }
}

/// The address a pane may be sent to: http or https with a host, and nothing else (no `file:`,
/// `chrome:`, `devtools:`, `view-source:`, `javascript:` or `data:`). `about:blank` is how a pane starts.
pub fn check_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw == "about:blank" {
        return Ok(raw.to_string());
    }
    let url = url::Url::parse(raw).map_err(|_| "that is not a web address".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err("only http and https addresses can be opened here".into());
    }
    Ok(url.to_string())
}

fn button_name(b: Button) -> &'static str {
    match b {
        Button::Left => "left",
        Button::Middle => "middle",
        Button::Right => "right",
        Button::None => "none",
    }
}

/// The CDP call for an input message: its method and parameters, or none for a message that is not input.
pub fn input_call(msg: &ClientMsg) -> Option<(&'static str, Value)> {
    match msg {
        ClientMsg::Mouse { kind, x, y, button, buttons, clicks, mods } => {
            let ty = match kind {
                MouseKind::Down => "mousePressed",
                MouseKind::Up => "mouseReleased",
                MouseKind::Move => "mouseMoved",
            };
            let clicks = if matches!(kind, MouseKind::Move) { 0 } else { (*clicks).clamp(1, 3) };
            Some((
                "Input.dispatchMouseEvent",
                json!({ "type": ty, "x": x, "y": y, "button": button_name(*button), "buttons": buttons, "clickCount": clicks, "modifiers": mods & 15 }),
            ))
        }
        ClientMsg::Wheel { x, y, dx, dy, mods } => Some((
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy, "modifiers": mods & 15 }),
        )),
        ClientMsg::Key { kind, key, code, text, vk, mods } => {
            // A key that types something is a `keyDown` with its text; one that does not (an arrow, Shift) is a raw one.
            let ty = match kind {
                KeyKind::Up => "keyUp",
                KeyKind::Down if text.is_empty() => "rawKeyDown",
                KeyKind::Down => "keyDown",
            };
            let mut p = json!({ "type": ty, "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": mods & 15 });
            if matches!(kind, KeyKind::Down) && !text.is_empty() {
                p["text"] = json!(text);
                p["unmodifiedText"] = json!(text);
            }
            Some(("Input.dispatchKeyEvent", p))
        }
        ClientMsg::Paste { text } => Some(("Input.insertText", json!({ "text": text }))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mouse_press_becomes_a_cdp_press_with_a_click_count() {
        let m = parse(r#"{"t":"mouse","type":"down","x":10.5,"y":20,"button":"left","buttons":1,"clicks":1}"#).unwrap();
        let (method, p) = input_call(&m).unwrap();
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(p["type"], "mousePressed");
        assert_eq!(p["button"], "left");
        assert_eq!(p["clickCount"], 1);
        assert_eq!(p["x"], 10.5);
    }

    #[test]
    fn a_move_has_no_click_and_modifiers_keep_only_the_four_known_bits() {
        let m = parse(r#"{"t":"mouse","type":"move","x":1,"y":2,"mods":255}"#).unwrap();
        let (_, p) = input_call(&m).unwrap();
        assert_eq!(p["type"], "mouseMoved");
        assert_eq!(p["clickCount"], 0);
        assert_eq!(p["modifiers"], 15);
    }

    #[test]
    fn a_key_with_text_types_it_and_one_without_is_raw() {
        let typed = parse(r#"{"t":"key","type":"down","key":"a","code":"KeyA","text":"a","vk":65}"#).unwrap();
        let (method, p) = input_call(&typed).unwrap();
        assert_eq!((method, p["type"].as_str(), p["text"].as_str()), ("Input.dispatchKeyEvent", Some("keyDown"), Some("a")));
        let arrow = parse(r#"{"t":"key","type":"down","key":"ArrowLeft","code":"ArrowLeft","vk":37}"#).unwrap();
        let (_, p) = input_call(&arrow).unwrap();
        assert_eq!(p["type"], "rawKeyDown");
        assert!(p.get("text").is_none());
        let up = parse(r#"{"t":"key","type":"up","key":"a","code":"KeyA","text":"a","vk":65}"#).unwrap();
        assert_eq!(input_call(&up).unwrap().1["type"], "keyUp");
    }

    #[test]
    fn a_wheel_and_a_paste_are_input_and_a_resize_is_not() {
        let w = parse(r#"{"t":"wheel","x":5,"y":6,"dx":0,"dy":120}"#).unwrap();
        assert_eq!(input_call(&w).unwrap().1["deltaY"], 120.0);
        let p = parse(r#"{"t":"paste","text":"hello"}"#).unwrap();
        assert_eq!(input_call(&p).unwrap(), ("Input.insertText", json!({ "text": "hello" })));
        assert!(input_call(&parse(r#"{"t":"resize","w":800,"h":600}"#).unwrap()).is_none());
    }

    #[test]
    fn a_size_outside_the_limits_is_refused() {
        assert!(parse(r#"{"t":"resize","w":99,"h":600}"#).is_err());
        assert!(parse(r#"{"t":"resize","w":800,"h":4097}"#).is_err());
        assert!(parse(r#"{"t":"resize","w":100,"h":4096}"#).is_ok());
    }

    #[test]
    fn a_malformed_or_unknown_message_is_refused_in_words() {
        for bad in ["", "nonsense", r#"{"t":"mouse"}"#, r#"{"t":"launch","cmd":"x"}"#, r#"{"t":"mouse","type":"down","x":"a","y":1}"#] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains("pane understands"), "{bad}: {e}");
        }
        assert!(parse(r#"{"t":"wheel","x":1e300,"y":0,"dx":0,"dy":0}"#).is_err());
    }

    #[test]
    fn a_message_over_its_limit_is_refused_before_it_is_parsed() {
        let long = format!(r#"{{"t":"key","type":"down","key":"{}"}}"#, "k".repeat(MAX_MESSAGE_BYTES));
        assert!(parse(&long).unwrap_err().contains("too long"));
        let paste = |n: usize| format!(r#"{{"t":"paste","text":"{}"}}"#, "p".repeat(n));
        assert!(parse(&paste(MAX_PASTE_BYTES)).is_ok());
        assert!(parse(&paste(MAX_PASTE_BYTES + 1)).is_err());
    }

    #[test]
    fn only_web_addresses_can_be_opened() {
        assert_eq!(check_url("about:blank").unwrap(), "about:blank");
        assert_eq!(check_url(" https://example.com/a?b=1 ").unwrap(), "https://example.com/a?b=1");
        assert!(check_url("http://127.0.0.1:8080/").is_ok());
        for bad in ["file:///etc/passwd", "chrome://settings", "devtools://x", "view-source:https://a.test", "javascript:alert(1)", "data:text/html,x", "ftp://a.test", "example.com", "https://", ""] {
            assert!(check_url(bad).is_err(), "{bad} must be refused");
        }
        assert!(parse(r#"{"t":"nav","go":"url","url":"file:///etc/passwd"}"#).is_err());
        assert!(parse(r#"{"t":"nav","go":"back"}"#).is_ok());
    }
}
