// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A web terminal as a typing target (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §4).
//!
//! `terminal_state` finds the page's terminal and reads the rows it shows;
//! `terminal_keys` focuses it and sends real key events, one `keyDown` and
//! `keyUp` per key. xterm.js in screen-reader mode acts only on real key
//! events, not on `Input.insertText`, which is why `fill` and `key` text
//! never reached a shell. Whether a terminal may be typed into at all, and
//! what may be typed, is srv's decision (a grant, the credential and secret
//! prompt checks); this module only does what it is asked, on a dedicated
//! browser pane, and says what it found.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::act::{fail, isolated_context, unauthorized};
use super::cdp::CdpSession;
use super::types::ApiResponse;
use crate::state::AppState;

/// Pace between keys, so a page that handles each one on a timer keeps up.
const KEY_PACE: Duration = Duration::from_millis(2);

/// Most keys one `terminal_keys` call sends; srv sends longer text in chunks,
/// so that Stop and Pause act between them.
const MAX_KEYS_PER_CALL: usize = 256;

/// Finds the terminals in the page and its same-origin frames, picks the one
/// to use (the focused one, else the first that is showing), and describes it.
/// With `focus`, also gives it focus. Runs in an isolated world: the page's
/// own scripts can't change what it sees.
const TERMINAL_JS: &str = r#"(focus) => {
  const found = [];
  const visit = (doc, win, depth) => {
    for (const ta of doc.querySelectorAll('textarea.xterm-helper-textarea, textarea[aria-label="Terminal input"]')) {
      // A real terminal is not part of a form; a lookalike that could submit one takes the form rules.
      if (ta.form) continue;
      found.push({ doc, win, ta, host: ta.closest('.xterm') || ta.parentElement });
    }
    if (depth >= 3) return;
    for (const fr of doc.querySelectorAll('iframe, frame')) {
      try {
        const d = fr.contentDocument;
        if (d && fr.contentWindow) visit(d, fr.contentWindow, depth + 1);
      } catch (e) { /* another site's frame: not reachable from here */ }
    }
  };
  visit(document, window, 0);
  if (!found.length) return { count: 0 };
  const showing = (t) => { const r = t.host.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  const pick = found.find((t) => t.doc.activeElement === t.ta) || found.find(showing) || found[0];
  if (focus) { pick.win.focus(); pick.ta.focus({ preventScroll: true }); }
  const tree = pick.host.querySelector('.xterm-accessibility-tree');
  const rows = tree ? Array.from(tree.children).map((d) => d.textContent || '') : null;
  return {
    count: found.length,
    kind: 'xterm',
    origin: pick.win.location.origin,
    // The two addresses a grant is checked against, read in the same
    // evaluation that picks the terminal (see `off_granted_sites`).
    page: location.href,
    frame: pick.win.location.href,
    focused: pick.doc.activeElement === pick.ta,
    screen_reader: !!tree,
    rows,
  };
}"#;

#[derive(Debug, Deserialize)]
pub struct TerminalStateReq {
    pub block_id: String,
}

/// One key: a `keyDown` (with `text` when it types a character) and a `keyUp`.
#[derive(Debug, Deserialize)]
pub struct KeyStep {
    pub key: String,
    pub code: String,
    #[serde(default)]
    pub text: Option<String>,
    pub vk: i64,
    /// CDP modifier bits: 1 Alt, 2 Control, 4 Meta, 8 Shift.
    #[serde(default)]
    pub modifiers: i64,
}

#[derive(Debug, Deserialize)]
pub struct TerminalKeysReq {
    pub block_id: String,
    pub keys: Vec<KeyStep>,
    /// The sites the person's grant is bound to (`allowed_origins` normal
    /// form). Checked here, against the terminal this call picks, and not
    /// only by srv before it: a page that navigates or redirects between
    /// srv's look and these keys must not get them.
    pub allowed_origins: Vec<String>,
}

fn ok(data: Value) -> (StatusCode, Json<ApiResponse<Value>>) {
    (StatusCode::OK, Json(ApiResponse::ok(data)))
}

/// Run [`TERMINAL_JS`] in the pane's main frame.
async fn look(cdp: &mut CdpSession, focus: bool) -> Result<Value, String> {
    let ctx = isolated_context(cdp, None).await?;
    let reply = cdp
        .call(
            "Runtime.evaluate",
            json!({
                "expression": format!("({TERMINAL_JS})({focus})"),
                "contextId": ctx,
                "returnByValue": true,
            }),
        )
        .await
        .map_err(|e| format!("CDP Runtime.evaluate: {e}"))?;
    if let Some(exc) = reply.get("exceptionDetails") {
        let msg = exc
            .pointer("/exception/description")
            .and_then(|d| d.as_str())
            .unwrap_or("unknown exception");
        return Err(format!("page script error: {msg}"));
    }
    Ok(reply
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

/// `POST /agentmux/browser/terminal_state` — is there a web terminal on the
/// page, which site's frame holds it, and what rows does it show.
pub async fn terminal_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<TerminalStateReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "terminal").await
    {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let found = look(&mut cdp, false).await;
    let _ = cdp.close().await;
    match found {
        Ok(v) => ok(v),
        Err(e) => fail(e),
    }
}

/// `POST /agentmux/browser/terminal_keys` — focus the terminal and send `keys`
/// to it as real key events. Answers how many were sent.
pub async fn terminal_keys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<TerminalKeysReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    if req.keys.len() > MAX_KEYS_PER_CALL {
        return fail(format!(
            "terminal_keys: at most {MAX_KEYS_PER_CALL} keys per call"
        ));
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "terminal").await
    {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let sent = send_keys(&mut cdp, &req.keys, &req.allowed_origins).await;
    let _ = cdp.close().await;
    match sent {
        Ok(n) => ok(json!({ "sent": n })),
        Err(e) => fail(e),
    }
}

/// Why `seen` (what [`TERMINAL_JS`] reported) is off the granted sites, if it
/// is: the page itself, or the frame that holds the terminal, is not on one of
/// `allowed`. Missing addresses count as off: never type on a guess.
fn off_granted_sites(seen: &Value, allowed: &[String]) -> Option<String> {
    use agentmux_common::allowed_origins::{allows, target_origin};
    for (field, what) in [("page", "the page"), ("frame", "the terminal's frame")] {
        let on = seen
            .get(field)
            .and_then(|v| v.as_str())
            .is_some_and(|url| target_origin(url).is_some() && allows(allowed, url));
        if !on {
            return Some(format!(
                "{what} is off the sites the person granted, so nothing was typed: go back to the granted site"
            ));
        }
    }
    None
}

async fn send_keys(
    cdp: &mut CdpSession,
    keys: &[KeyStep],
    allowed: &[String],
) -> Result<usize, String> {
    let seen = look(cdp, true).await?;
    if seen.get("count").and_then(|c| c.as_u64()).unwrap_or(0) == 0 {
        return Err("no web terminal on this page".to_string());
    }
    if let Some(why) = off_granted_sites(&seen, allowed) {
        return Err(why);
    }
    if seen.get("focused").and_then(|f| f.as_bool()) != Some(true) {
        return Err("the terminal didn't take focus (is the pane showing?)".to_string());
    }
    for (i, k) in keys.iter().enumerate() {
        let mut down = json!({
            "type": "keyDown",
            "key": k.key,
            "code": k.code,
            "windowsVirtualKeyCode": k.vk,
            "nativeVirtualKeyCode": k.vk,
            "modifiers": k.modifiers,
        });
        if let Some(text) = &k.text {
            down["text"] = json!(text);
            down["unmodifiedText"] = json!(text);
        }
        let up = json!({
            "type": "keyUp",
            "key": k.key,
            "code": k.code,
            "windowsVirtualKeyCode": k.vk,
            "nativeVirtualKeyCode": k.vk,
            "modifiers": k.modifiers,
        });
        for (name, params) in [("keyDown", down), ("keyUp", up)] {
            cdp.call("Input.dispatchKeyEvent", params)
                .await
                .map_err(|e| format!("CDP Input.dispatchKeyEvent {name} (after {i} keys): {e}"))?;
        }
        tokio::time::sleep(KEY_PACE).await;
    }
    Ok(keys.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(page: &str, frame: &str) -> Value {
        json!({ "count": 1, "page": page, "frame": frame })
    }
    fn allowed(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_page_and_frame_on_the_granted_site_may_be_typed_into() {
        let s = seen(
            "https://shell.example.com/",
            "https://shell.example.com/term",
        );
        assert_eq!(
            off_granted_sites(&s, &allowed(&["https://shell.example.com"])),
            None
        );
    }

    #[test]
    fn a_page_that_left_the_granted_site_is_refused() {
        let s = seen("https://evil.example.net/", "https://evil.example.net/");
        let why = off_granted_sites(&s, &allowed(&["https://shell.example.com"])).unwrap();
        assert!(
            why.contains("the page") && why.contains("nothing was typed"),
            "{why}"
        );
    }

    #[test]
    fn a_terminal_frame_from_another_site_is_refused_even_on_a_granted_page() {
        let s = seen(
            "https://shell.example.com/",
            "https://other.example.org/term",
        );
        let why = off_granted_sites(&s, &allowed(&["https://shell.example.com"])).unwrap();
        assert!(why.contains("terminal's frame"), "{why}");
    }

    #[test]
    fn a_missing_or_non_web_address_is_off_never_a_guess() {
        let ok = allowed(&["https://shell.example.com"]);
        assert!(off_granted_sites(&json!({ "count": 1 }), &ok).is_some());
        assert!(off_granted_sites(&seen("about:blank", "about:blank"), &ok).is_some());
        assert!(
            off_granted_sites(&seen("https://shell.example.com/", "data:text/html,x"), &ok)
                .is_some()
        );
    }

    #[test]
    fn no_granted_sites_means_nowhere() {
        let s = seen("https://shell.example.com/", "https://shell.example.com/");
        assert!(off_granted_sites(&s, &[]).is_some());
    }
}
