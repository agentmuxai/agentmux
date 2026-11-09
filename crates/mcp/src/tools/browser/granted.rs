// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `request_auto_approve` tells an agent when its grant is made.

use serde_json::Value;

/// What `request_auto_approve` tells the agent when it was granted: for how long
/// (a standing grant has no clock: it stands until the user revokes it) and what
/// the grant lets it do.
pub(super) fn granted_text(d: &Value) -> String {
    let sites: Vec<&str> = d["origins"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    let has = |p: &str| d["powers"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str() == Some(p)));
    // Asking about each line is the stricter of the two, so it wins when both are there.
    let (terminal, per_line) = (has("terminal") && !has("terminal_ask"), has("terminal_ask"));
    let how_long = if d["always"].as_bool() == Some(true) {
        "until the user revokes it".to_string()
    } else {
        format!("for {} more min", d["minutes_left"].as_u64().unwrap_or(0))
    };
    // With the power, Enter and Space are taken as the click they are; a key whose click can't be named is refused.
    let keys = if has("enter_space") {
        "eval stays refused, and Enter and Space (action key) are taken as the click they are: a form's default button, \
         a button, a link or a checkbox, checked like any click; on anything else they are refused, so click by ref"
    } else {
        "eval and Enter/Space stay refused"
    };
    format!(
        "Auto-approve on {how_long} on {}: committing clicks (action click) run without asking. Nobody \
         checks them now, so check each one yourself. A payment still asks, {keys}, and on any other site clicks ask as \
         before.{}{} Stop it with action stop_auto_approve when the job is done.",
        sites.join(", "),
        if terminal {
            " You may also type and run commands in a web terminal on these sites with action terminal: \
             nobody checks each line, the user can Stop or Pause you, and a password or code prompt is theirs to type."
        } else if per_line {
            " You may also use a web terminal on these sites with action terminal, one line per call: the user is shown \
             each line (give a `reason`) and presses Run it or Don't run; a password or code prompt is theirs to type."
        } else {
            ""
        },
        if has("uploads") {
            " When a page opens its file chooser, AgentMux keeps it for 30 seconds: fill it with action set_files, chooser true and \
             paths (files in your own workspace); a chooser nobody fills in time is cancelled."
        } else {
            ""
        }
    )
}
