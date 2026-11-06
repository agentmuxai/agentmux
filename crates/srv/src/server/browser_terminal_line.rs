// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `terminal` on a grant that asks about each line
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §4.4).
//!
//! The agent's line is typed without its Enter, read back from the terminal,
//! and shown to the person in the pane with Run it and Don't run. Only a click
//! on Run it, relayed by the host, lets Enter go in, and only if the terminal
//! still shows the very line the person was shown. In every other end (No, no
//! answer, Take over, Pause, the grant ending) what the agent typed is erased.

use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::server::browser_actions::{note, How, Kind};
use crate::server::browser_attention::{self as attention, Answer};
use crate::server::browser_bypass as bypass;
use crate::server::browser_terminal::{
    check_target, credential_line, let_screen_catch_up, look, output_fields, rows_of, settle, step_for_key, KeyStep, Refusal, CHUNK,
};
use crate::server::ui_handlers::{err_response, proxy_value};
use crate::server::AppState;

/// How long the person has to answer a line before it is erased.
const ANSWER_WAIT: Duration = Duration::from_secs(3 * 60);
/// The longest line shown in full; a longer one is refused, not shown cut.
pub(crate) const MAX_SHOWN: usize = 4_000;
/// Rows of the terminal the banner shows with the line.
const BANNER_ROWS: usize = 5;
/// The widest a terminal row is shown in the banner.
const BANNER_ROW_CHARS: usize = 200;
/// Rows a wrapped line is looked for over.
const MAX_WRAP_ROWS: usize = 12;
/// How long to keep reading for the typed line to reach the terminal's rows,
/// which trail the screen (more so in a window that has no focus).
const READBACK_WAIT: Duration = Duration::from_secs(5);
/// How often the rows are read while waiting.
const READBACK_POLL: Duration = Duration::from_millis(500);

/// What the handler hands over once the grant, the page and the prompt are checked.
pub(crate) struct Call<'a> {
    pub state: &'a AppState,
    pub block_id: &'a str,
    pub pane: &'a str,
    pub agent: &'a str,
    pub grant: &'a bypass::Grant,
    pub origin: &'a str,
    /// Every key of the call, in order, as the handler built them.
    pub steps: Vec<KeyStep>,
    pub typed: &'a str,
    pub reason: &'a str,
    /// What the Log keeps of the call.
    pub what: String,
    pub wait: Duration,
    pub until: Option<&'a str>,
    /// The terminal's rows before anything was typed.
    pub rows_before: Vec<String>,
    /// Who drove the pane when the call began: once that changes the person has
    /// taken it over and nothing more of the agent's is sent beyond clearing its line.
    pub owner: Option<String>,
}

/// Is `c` a character the person could not see in a line, or would read as
/// something else? Controls, joiners, direction marks and look-alike spaces.
fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00a0}' | '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{115f}' | '\u{1160}' | '\u{17b4}' | '\u{17b5}' | '\u{180e}'
                | '\u{2000}'..='\u{200f}' | '\u{2028}'..='\u{202f}' | '\u{205f}'..='\u{206f}' | '\u{3000}' | '\u{3164}'
                | '\u{feff}' | '\u{ffa0}' | '\u{0600}'..='\u{0605}' | '\u{06dd}' | '\u{070f}' | '\u{08e2}' | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}' | '\u{2800}' | '\u{fe00}'..='\u{fe0f}' | '\u{fff9}'..='\u{fffc}'
                | '\u{e0000}'..='\u{e007f}' | '\u{e0100}'..='\u{e01ef}'
        )
}

/// `s` with every hidden character written out (`\u{1b}`), so what the person
/// reads is what the shell would receive.
pub(crate) fn visible(s: &str) -> String {
    s.chars().map(|c| if is_hidden(c) { c.escape_unicode().to_string() } else { c.to_string() }).collect()
}

/// The last row that shows anything, without its trailing blanks.
fn tail_row(rows: &[String]) -> String {
    rows.iter().rev().find(|r| !r.trim().is_empty()).map(|r| r.trim_end().to_string()).unwrap_or_default()
}

/// The line the terminal shows at its prompt now. With `typed`, the line must
/// end with it, over as many rows as it wrapped; None means the terminal does
/// not show what was typed, so what Enter would run can't be confirmed. Without
/// `typed` (only keys were pressed, say ArrowUp), it is the last row.
pub(crate) fn read_back(rows: &[String], typed: &str) -> Option<String> {
    let end = rows.iter().rposition(|r| !r.trim().is_empty())?;
    let tail = &rows[..=end];
    if typed.trim().is_empty() {
        return Some(tail_row(rows));
    }
    let want = typed.trim_end();
    (1..=tail.len().min(MAX_WRAP_ROWS)).find_map(|k| {
        let joined = tail[tail.len() - k..].concat();
        joined.trim_end().ends_with(want).then(|| joined.trim_end().to_string())
    })
}

/// The last rows of the terminal as the banner shows them: hidden characters
/// written out, token shapes masked, each row cut. They are page content and
/// exist only while the banner is up.
pub(crate) fn rows_for_banner(rows: &[String]) -> Vec<String> {
    let Some(end) = rows.iter().rposition(|r| !r.trim().is_empty()) else { return Vec::new() };
    let from = (end + 1).saturating_sub(BANNER_ROWS);
    rows[from..=end]
        .iter()
        .map(|r| {
            let masked = agentmux_common::redact::redact_token_shapes(r.trim_end());
            visible(&masked).chars().take(BANNER_ROW_CHARS).collect()
        })
        .collect()
}

/// The keys before the Enter, and whether the call wants Enter pressed. One
/// line to a call, and the Enter last: the Enter is what the person answers.
pub(crate) fn split_line(mut steps: Vec<KeyStep>, typed: &str) -> Result<(Vec<KeyStep>, bool), String> {
    if typed.contains('\t') {
        return Err("a tab completes in the terminal, so the line can't be confirmed: type the line without tabs".to_string());
    }
    if typed.chars().count() > MAX_SHOWN {
        return Err(format!("this site's grant shows the person each line, and a line over {MAX_SHOWN} characters is too long to show: send a shorter one"));
    }
    let enters = steps.iter().filter(|k| k.key == "Enter").count();
    match enters {
        0 => Ok((steps, false)),
        1 if steps.last().is_some_and(|k| k.key == "Enter") => {
            steps.pop();
            Ok((steps, true))
        }
        1 => Err("Enter must be the last key: the person answers for the line before it runs".to_string()),
        _ => Err("this site's grant shows the person each line: send one line per call".to_string()),
    }
}

/// Why the keys must stop now, if they must: the pane is paused or the
/// grant that allowed them is gone.
fn brake(c: &Call<'_>) -> Option<String> {
    if crate::server::browser_actions::not_paused(c.block_id).is_err() {
        return Some("the user paused this pane, so nothing more was typed".to_string());
    }
    if !bypass::active_for(c.pane, c.agent).is_some_and(|g| g.id == c.grant.id) {
        return Some("the person ended the grant".to_string());
    }
    None
}

/// Send `keys` to the terminal in chunks.
async fn send(c: &Call<'_>, keys: &[KeyStep]) -> Result<(), String> {
    for chunk in keys.chunks(CHUNK) {
        let body = json!({ "block_id": c.block_id, "keys": chunk, "allowed_origins": c.grant.origins });
        proxy_value(c.state, "terminal_keys", body, Duration::from_secs(20)).await.map_err(|(_, e)| e)?;
    }
    Ok(())
}

/// Type the keys before the Enter, the brakes checked between chunks. Answers
/// how many went in, and why it stopped, if it did.
async fn type_pre(c: &Call<'_>, pre: &[KeyStep]) -> (usize, Option<String>) {
    let mut sent = 0;
    for chunk in pre.chunks(CHUNK) {
        if let Some(why) = brake(c) {
            return (sent, Some(why));
        }
        if let Err(e) = send(c, chunk).await {
            return (sent, Some(e));
        }
        sent += chunk.len();
    }
    (sent, None)
}

/// Take back what the agent typed: as many Backspaces as it typed characters,
/// then, if the prompt still doesn't read as it did, Ctrl+C. Best effort: the
/// pane may be gone or off the granted site, and then nothing more is sent. The
/// Backspaces go at once, before a person who took the pane over can type; the
/// Ctrl+C waits for the screen and is dropped if the person has the pane by then,
/// since it could interrupt what they started.
async fn erase(c: &Call<'_>, typed: &[KeyStep]) {
    let count = typed.iter().filter(|k| k.text.is_some()).count();
    let baseline = tail_row(&c.rows_before);
    if count > 0 {
        let Ok(back) = step_for_key("Backspace") else { return };
        let keys = vec![back; count];
        if send(c, &keys).await.is_err() {
            return;
        }
    }
    let_screen_catch_up(c.state, c.block_id).await;
    if crate::server::browser_owner::owner_of(c.pane) != c.owner {
        return;
    }
    let now = look(c.state, c.block_id).await.ok().and_then(|v| rows_of(&v)).unwrap_or_default();
    if tail_row(&now) != baseline {
        if let Ok(ctrl_c) = step_for_key("Ctrl+C") {
            let _ = send(c, &[ctrl_c]).await;
        }
    }
}

/// The answer to the agent, with the output the terminal shows once it settles.
async fn finish(c: &Call<'_>, sent: usize, stopped: Option<String>, how: How, answer: Option<&str>) -> Response {
    let (rows, screen_reader) = if sent == 0 { (None, true) } else { settle(c.state, c.block_id, c.wait, c.until).await };
    note(c.pane, c.agent, Kind::Terminal).detail(&c.what).how(how).record(c.state);
    let mut data = json!({ "sent": sent, "of": c.steps.len() });
    if let Some(a) = answer {
        data["answer"] = json!(a);
    }
    if let Some(why) = stopped {
        data["stopped"] = json!(why);
    }
    output_fields(&mut data, rows, screen_reader, sent);
    (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
}

/// A call that ended with nothing run: logged as refused and answered with `why`.
fn refuse(c: &Call<'_>, code: StatusCode, why: String) -> Response {
    note(c.pane, c.agent, Kind::Terminal).detail(&c.what).how(How::Refused(why.clone())).record(c.state);
    err_response(code, why)
}

/// Show the person the line and wait for Run it or Don't run. The returned
/// guard keeps the pane's tools paused until the caller has acted on the answer.
async fn ask_person(c: &Call<'_>, line: &str, rows: &[String]) -> Result<(Answer, attention::Waiting), String> {
    let reason: String = c.reason.trim().chars().take(300).collect();
    let payload = json!({
        "reason": reason,
        "line": visible(line),
        "long": line.chars().count() > 300,
        "rows": rows_for_banner(rows),
        "origin": c.origin,
    });
    attention::ask(c.state, c.pane, c.agent, attention::Kind::Line, payload, ANSWER_WAIT).await.map_err(|e| e.to_string())
}

/// Read the terminal until it shows the line that was typed, for at most
/// [`READBACK_WAIT`]. Answers the last rows read, and the line if they show it.
async fn rows_showing(c: &Call<'_>) -> Result<(Vec<String>, Option<String>), (StatusCode, String)> {
    let start = std::time::Instant::now();
    loop {
        let rows = rows_of(&look(c.state, c.block_id).await?).unwrap_or_default();
        let line = read_back(&rows, c.typed);
        if line.is_some() || start.elapsed() >= READBACK_WAIT {
            return Ok((rows, line));
        }
        tokio::time::sleep(READBACK_POLL).await;
    }
}

/// Read the terminal again as the person said Run it: the line must be the one
/// they were shown.
async fn still_the_line(c: &Call<'_>, shown: &str) -> Result<(), String> {
    check_target(c.state, c.block_id, c.grant).await.map_err(|r| match r {
        Refusal::Plain(_, why) | Refusal::Secret(why) => why,
    })?;
    let rows = look(c.state, c.block_id).await.map_err(|(_, e)| e)?;
    match read_back(&rows_of(&rows).unwrap_or_default(), c.typed) {
        Some(now) if now == shown => Ok(()),
        _ => Err("the line in the terminal changed after the person was shown it, so it was not run".to_string()),
    }
}

/// Run the call under a grant that asks about each line.
pub(crate) async fn run(c: Call<'_>) -> Response {
    let (pre, wants_enter) = match split_line(c.steps.clone(), c.typed) {
        Ok(x) => x,
        Err(e) => return refuse(&c, StatusCode::BAD_REQUEST, e),
    };
    let (sent, stopped) = type_pre(&c, &pre).await;
    if let Some(why) = stopped {
        erase(&c, &pre[..sent]).await;
        return finish(&c, 0, Some(why), How::Refused("stopped".to_string()), None).await;
    }
    if !wants_enter {
        return finish(&c, sent, None, How::Approved, None).await;
    }
    let_screen_catch_up(c.state, c.block_id).await;
    let (rows, found) = match rows_showing(&c).await {
        Ok(x) => x,
        Err((code, e)) => {
            erase(&c, &pre).await;
            return refuse(&c, code, e);
        }
    };
    let line = match (check_target(c.state, c.block_id, c.grant).await, found) {
        (Err(Refusal::Plain(code, why)), _) => {
            erase(&c, &pre).await;
            return refuse(&c, code, why);
        }
        (Err(Refusal::Secret(why)), _) => {
            erase(&c, &pre).await;
            return refuse(&c, StatusCode::FORBIDDEN, why);
        }
        (Ok(_), Some(line)) if !credential_line(&line) => line,
        (Ok(_), Some(_)) => {
            erase(&c, &pre).await;
            return refuse(&c, StatusCode::FORBIDDEN, "the line at the prompt looks like a credential, so it is not run: ask the person to type it".to_string());
        }
        (Ok(_), None) => {
            erase(&c, &pre).await;
            return refuse(&c, StatusCode::CONFLICT, "I can't confirm what would run: the terminal doesn't show what you typed. Hand off the pane".to_string());
        }
    };
    let (answer, _holding) = match ask_person(&c, &line, &rows).await {
        Ok(a) => a,
        Err(e) => {
            erase(&c, &pre).await;
            return refuse(&c, StatusCode::CONFLICT, e);
        }
    };
    if !matches!(answer, Answer::Yes | Answer::YesAlways | Answer::YesAsking) {
        erase(&c, &pre).await;
        let (how, said) = if answer == Answer::TimedOut { (How::TimedOut, "timed_out") } else { (How::Declined, "declined") };
        return finish(&c, 0, None, how, Some(said)).await;
    }
    let late = if crate::server::browser_owner::owner_of(c.pane) != c.owner {
        Some("the person took over this pane, so the line was not run".to_string())
    } else {
        match brake(&c) {
            Some(why) => Some(why),
            None => still_the_line(&c, &line).await.err(),
        }
    };
    if let Some(why) = late {
        erase(&c, &pre).await;
        return refuse(&c, StatusCode::CONFLICT, why);
    }
    let Ok(enter) = step_for_key("Enter") else { return refuse(&c, StatusCode::INTERNAL_SERVER_ERROR, "no Enter key".to_string()) };
    if let Err(e) = send(&c, &[enter]).await {
        return refuse(&c, StatusCode::BAD_GATEWAY, e);
    }
    tracing::info!(agent_id = %c.agent, pane = %c.pane, origin = %c.origin, "[browser-terminal] ran a line the person approved");
    finish(&c, sent + 1, None, How::Approved, Some("approved")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn key(name: &str) -> KeyStep {
        step_for_key(name).unwrap()
    }

    #[test]
    fn hidden_characters_are_written_out() {
        assert_eq!(visible("ls -la"), "ls -la");
        assert_eq!(visible("a\u{200b}b"), "a\\u{200b}b");
        assert_eq!(visible("rm\u{00a0}-rf"), "rm\\u{a0}-rf", "a look-alike space is not a space");
        assert_eq!(visible("echo \u{1b}[2J"), "echo \\u{1b}[2J");
        assert_eq!(visible("\u{202e}txt.sh"), "\\u{202e}txt.sh", "a direction mark is shown");
    }

    #[test]
    fn tag_characters_selectors_annotations_and_blank_braille_are_shown_too() {
        for c in ['\u{e0041}', '\u{e007f}', '\u{fe0f}', '\u{e0100}', '\u{fff9}', '\u{fffb}', '\u{2800}', '\u{200b}'] {
            let shown = visible(&format!("a{c}b"));
            assert!(shown.contains("\\u{") && !shown.contains(c), "U+{:04X} was left as it is: {shown:?}", c as u32);
        }
        assert_eq!(visible("café ✓ 日本語"), "café ✓ 日本語", "ordinary non-ASCII text is left readable");
    }

    #[test]
    fn the_line_is_read_back_from_the_prompt_row() {
        let r = rows(&["user@host:~$ ls", "a b c", "user@host:~$ echo hi", "", ""]);
        assert_eq!(read_back(&r, "echo hi").as_deref(), Some("user@host:~$ echo hi"));
        assert_eq!(read_back(&r, "ls"), None, "the typed text is not at the prompt");
    }

    #[test]
    fn what_was_already_at_the_prompt_is_part_of_the_line() {
        let r = rows(&["$ rm -rf build && echo hi"]);
        assert_eq!(read_back(&r, "echo hi").as_deref(), Some("$ rm -rf build && echo hi"));
    }

    #[test]
    fn a_line_that_wrapped_is_read_over_its_rows() {
        let r = rows(&["$ echo aaaaaaaaaa", "bbbbbbbbbb", ""]);
        assert_eq!(read_back(&r, "echo aaaaaaaaaabbbbbbbbbb").as_deref(), Some("$ echo aaaaaaaaaabbbbbbbbbb"));
    }

    #[test]
    fn keys_alone_read_the_last_row() {
        let r = rows(&["$ ls", "$ make test", ""]);
        assert_eq!(read_back(&r, "").as_deref(), Some("$ make test"));
        assert_eq!(read_back(&rows(&["", ""]), ""), None, "a blank terminal shows no line");
    }

    #[test]
    fn the_banner_rows_are_the_last_few_escaped_and_masked() {
        let r = rows(&["one", "two", "three", "four", "five", "six\u{1b}[0m", "", ""]);
        let shown = rows_for_banner(&r);
        assert_eq!(shown.len(), BANNER_ROWS);
        assert_eq!(shown.last().map(String::as_str), Some("six\\u{1b}[0m"));
        assert_eq!(shown.first().map(String::as_str), Some("two"));
        let secret = rows(&["token ghp_abcdefghijklmnopqrstuvwxyz0123456789"]);
        assert!(!rows_for_banner(&secret)[0].contains("abcdefghijklmnop"), "{:?}", rows_for_banner(&secret));
        let long = rows(&[&"x".repeat(500)]);
        assert_eq!(rows_for_banner(&long)[0].chars().count(), BANNER_ROW_CHARS);
    }

    #[test]
    fn a_call_is_one_line_with_its_enter_last() {
        let line = vec![key("Tab"), key("Enter")];
        let (pre, enter) = split_line(line, "ls").unwrap();
        assert_eq!((pre.len(), enter), (1, true));
        let (pre, enter) = split_line(vec![key("Ctrl+C")], "").unwrap();
        assert_eq!((pre.len(), enter), (1, false), "no Enter, nothing to answer");
        assert!(split_line(vec![key("Enter"), key("Tab")], "").unwrap_err().contains("last"));
        assert!(split_line(vec![key("Enter"), key("Enter")], "").unwrap_err().contains("one line"));
    }

    #[test]
    fn a_tab_or_a_line_too_long_to_show_is_refused() {
        assert!(split_line(Vec::new(), "ls\t").unwrap_err().contains("tab"));
        let long = "x".repeat(MAX_SHOWN + 1);
        assert!(split_line(Vec::new(), &long).unwrap_err().contains("too long"));
    }
}
