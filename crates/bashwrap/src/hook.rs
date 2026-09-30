// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `hook` subcommand.
//!
//! Reads a PreToolUse JSON event on stdin (the contract Claude Code
//! uses for hooks: see `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`
//! §5). If the event is a Bash invocation whose `command` hasn't
//! already been wrapped, emit an `updatedInput.command` that invokes
//! `agentmux-bashwrap exec` with the original command base64-encoded
//! into argv. Idempotent — if the command is already wrapped, pass
//! through with no rewrite.
//!
//! All errors degrade to a pass-through response so a hook failure
//! never blocks Claude's tool execution.

use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};

/// Subset of the PreToolUse payload we care about. Extra fields are
/// ignored (serde default).
#[derive(Deserialize)]
struct PreToolUseInput {
    tool_name: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default)]
    tool_input: Value,
}

const WRAPPER_BINARY: &str = "agentmux-bashwrap";
const WRAPPED_PREFIX: &str = "agentmux-bashwrap exec";

pub fn run_pretooluse_bash() -> Result<()> {
    // Read entire stdin. Hook payloads are small.
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;

    let response = build_response(&buf);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", response)?;
    Ok(())
}

/// Build the hook response for a PreToolUse payload. Public for
/// testability — callers in tests feed in a payload string and assert
/// the response shape without going through stdin.
pub fn build_response(stdin_payload: &str) -> Value {
    let input: PreToolUseInput = match serde_json::from_str(stdin_payload) {
        Ok(i) => i,
        Err(_) => {
            // Malformed hook input → pass through. Claude proceeds
            // with native Bash, no streaming for this call.
            return passthrough();
        }
    };

    if input.tool_name != "Bash" {
        return passthrough();
    }

    let command = input
        .tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Idempotence: if the command already invokes our wrapper, don't
    // re-wrap. Defends against weird hook re-fire scenarios.
    if command.starts_with(WRAPPED_PREFIX) {
        return passthrough();
    }

    // Tee a trailing file redirect back through the PTY so the agent's tool
    // feed shows the output live while the file is still written. Conservative:
    // only unambiguous trailing redirects are rewritten; anything else is
    // encoded verbatim (today's behavior). See
    // SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md §3.
    let effective_command =
        tee_redirect_rewrite(command).unwrap_or_else(|| command.to_string());

    // Issue #2491: the CLI's own `run_in_background` declaration is already
    // present in `tool_input` right alongside `command` — thread it through
    // so `bash_wrap.rs::effective_idle_timeout` can exempt a declared
    // long-runner from the idle-kill safety net instead of that signal
    // being dropped here and rediscovered from scratch at every consuming
    // layer (the same duplication trap `docs/retro/retro-stuck-background-
    // dock-timer-2026-08-10.md` already documented for the frontend's own
    // `isAcceptedBackgroundLaunch` classifier).
    let declared_background = input
        .tool_input
        .get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let b64 = URL_SAFE_NO_PAD.encode(effective_command.as_bytes());
    let wrapped = format!(
        "{} exec --tool-id={} --b64-cmd={}{}",
        WRAPPER_BINARY,
        shell_quote(&input.tool_use_id),
        b64,
        if declared_background { " --declared-background" } else { "" }
    );

    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {
                "command": wrapped
            }
        }
    })
}

fn passthrough() -> Value {
    // Empty object → Claude Code treats as "no opinion", proceeds.
    json!({})
}

// ─────────────────────────────────────────────────────────────────────────────
// Trailing-redirect → `tee` rewrite (Feature 1)
//
// A command like `task package > build.log 2>&1` sends the inner shell's stdout
// to the file, so the PTY (hence the tool-chunk stream, hence the feed) sees
// nothing. We recognize an unambiguous *trailing* file redirect and rewrite it
// so the same bytes still reach the file AND flow through the PTY via `tee`:
//
//   CMD > F            → set -o pipefail; { CMD ; } | tee -- F
//   CMD >> F           → set -o pipefail; { CMD ; } | tee -a -- F
//   CMD > F 2>&1       → set -o pipefail; { CMD ; } 2>&1 | tee -- F
//   CMD >> F 2>&1      → set -o pipefail; { CMD ; } 2>&1 | tee -a -- F
//   CMD &> F           → set -o pipefail; { CMD ; } 2>&1 | tee -- F
//   CMD &>> F          → set -o pipefail; { CMD ; } 2>&1 | tee -a -- F
//
// `set -o pipefail` makes the pipeline's exit code reflect CMD (not `tee`), so
// bashwrap still mirrors the real exit code. Wrapping the whole command in a
// `{ ; }` group is safe for a single command OR a pipeline (a pipeline's stdout
// is its last stage's stdout — exactly what the trailing `>` redirected), which
// is why we bail on list operators (`&&`/`||`/`;`/`&`), where the redirect would
// bind to only one command of the list.
//
// EVERYTHING ambiguous bails (returns None → encode verbatim, today's behavior):
// quoted/comment/heredoc/process-substitution redirects, `/dev/null`, fd-specific
// or dup redirects (`2>f`, `>&2`, and the `2>&1 > F` order), more than one output
// redirect, and non-trailing redirects.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum OutKind {
    Out,
    Append,
    AmpOut,
    AmpAppend,
}

struct ScanResult {
    redir_start: usize,
    target_start: usize,
    target_end: usize,
    append: bool,
    combine_stderr: bool,
}

/// Rewrite a recognized trailing redirect to a `tee` pipeline, or `None` to pass
/// the command through unchanged.
fn tee_redirect_rewrite(raw: &str) -> Option<String> {
    let cmd = raw.trim();
    if cmd.is_empty() {
        return None;
    }
    let scan = scan_redirect(cmd)?;
    let cmd_part = cmd[..scan.redir_start].trim();
    if cmd_part.is_empty() {
        return None;
    }
    let target = cmd[scan.target_start..scan.target_end].trim();
    if target.is_empty() || is_dev_null(target) {
        return None;
    }
    let aflag = if scan.append { "-a " } else { "" };
    let stream = if scan.combine_stderr { " 2>&1 |" } else { " |" };
    Some(format!(
        "set -o pipefail; {{ {} ; }}{} tee {}-- {}",
        cmd_part, stream, aflag, target
    ))
}

fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
}

/// Strip one layer of matching surrounding quotes for the `/dev/null` check.
fn is_dev_null(target: &str) -> bool {
    let t = target.trim();
    let unq = if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        &t[1..t.len() - 1]
    } else {
        t
    };
    unq == "/dev/null"
}

/// Quote/group-aware scan for a single top-level trailing output redirect.
/// Returns None on any bail condition (see the module comment above).
fn scan_redirect(cmd: &str) -> Option<ScanResult> {
    let b = cmd.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0; // `${...}` parameter-expansion nesting

    let mut out_redirs: Vec<(OutKind, usize, usize)> = Vec::new();
    let mut dups: Vec<(usize, usize)> = Vec::new(); // 2>&1 occurrences (start, end)
    let mut other_redir = false;
    let mut list_op = false;

    while i < n {
        let c = b[i];
        if in_single {
            if c == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == b'\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        if in_backtick {
            if c == b'`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        // Unquoted. Quote/escape/group/comment first.
        match c {
            b'\\' => {
                i += 2;
                continue;
            }
            b'\'' => {
                in_single = true;
                i += 1;
                continue;
            }
            b'"' => {
                in_double = true;
                i += 1;
                continue;
            }
            b'`' => {
                in_backtick = true;
                i += 1;
                continue;
            }
            b'#' => {
                // Comment iff at a word boundary; otherwise a literal `#`.
                let prev = if i == 0 { b' ' } else { b[i - 1] };
                if is_space(prev) || matches!(prev, b';' | b'|' | b'&' | b'(' | b'`') {
                    return None;
                }
                i += 1;
                continue;
            }
            b'(' => {
                paren_depth += 1;
                i += 1;
                continue;
            }
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                i += 1;
                continue;
            }
            b'$' => {
                // `${...}` parameter expansion: a literal `>`/`<`/`|` inside it
                // (e.g. `${X:->f}`, `${X/>/_}`) must not be read as an operator.
                // `$(...)` is handled by paren_depth via the `(` below.
                if i + 1 < n && b[i + 1] == b'{' {
                    brace_depth += 1;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            b'}' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        // Inside a subshell / $(...) / ${...} group: ignore operators (the
        // redirect there doesn't bind the outer command), but keep tracking
        // quotes (above).
        if paren_depth > 0 || brace_depth > 0 {
            i += 1;
            continue;
        }
        // Depth 0, unquoted — operator recognition.
        match c {
            b'<' => {
                if i + 1 < n && (b[i + 1] == b'(' || b[i + 1] == b'<') {
                    return None; // process substitution or heredoc
                }
                other_redir = true; // input redirect
                i += 1;
            }
            b'>' => {
                if i + 1 < n && b[i + 1] == b'(' {
                    return None; // process substitution >(
                }
                if i > 0 && b[i - 1].is_ascii_digit() {
                    // fd-prefixed: recognize exactly `2>&1`, else bail.
                    if b[i - 1] == b'2' && i + 2 < n && b[i + 1] == b'&' && b[i + 2] == b'1' {
                        dups.push((i - 1, i + 3));
                        i += 3;
                    } else {
                        other_redir = true;
                        i += 1;
                    }
                } else if i + 1 < n && b[i + 1] == b'>' {
                    out_redirs.push((OutKind::Append, i, i + 2));
                    i += 2;
                } else if i + 1 < n && b[i + 1] == b'&' {
                    other_redir = true; // >&N dup
                    i += 1;
                } else {
                    out_redirs.push((OutKind::Out, i, i + 1));
                    i += 1;
                }
            }
            b'&' => {
                if i + 1 < n && b[i + 1] == b'&' {
                    list_op = true;
                    i += 2;
                } else if i + 1 < n && b[i + 1] == b'>' {
                    if i + 2 < n && b[i + 2] == b'>' {
                        out_redirs.push((OutKind::AmpAppend, i, i + 3));
                        i += 3;
                    } else {
                        out_redirs.push((OutKind::AmpOut, i, i + 2));
                        i += 2;
                    }
                } else {
                    list_op = true; // standalone & (background)
                    i += 1;
                }
            }
            b'|' => {
                // Both `||` and a plain pipe bail. A plain pipe is bailed (not
                // wrapped) because `set -o pipefail` on `{ a | b ; } | tee` would
                // make the INNER pipeline report its first failure instead of the
                // last stage's exit — changing the exit code the original
                // `a | b > f` reported. Single commands have no inner pipe, so
                // pipefail correctly surfaces their exit through the tee.
                list_op = true;
                i += if i + 1 < n && b[i + 1] == b'|' { 2 } else { 1 };
            }
            // `;` and an unquoted/unescaped newline both separate commands. A
            // multi-line command whose LAST line has a trailing redirect must not
            // be wrapped, or `{ <all lines> ; } | tee` would tee the earlier
            // lines' stdout into the file too (violating the "same bytes to FILE"
            // guarantee). Quoted/heredoc/escaped newlines never reach here.
            b';' | b'\n' | b'\r' => {
                list_op = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    if in_single || in_double || in_backtick {
        return None; // unterminated quote
    }
    if list_op || other_redir || out_redirs.len() != 1 {
        return None;
    }
    let (kind, rstart, rend) = out_redirs[0];
    // No dup may precede the redirect (the `2>&1 > F` order has different
    // semantics; bail rather than mis-tee it).
    if dups.iter().any(|(ds, _)| *ds < rstart) {
        return None;
    }
    // Target word immediately after the redirect.
    let (tstart, tend) = read_word(b, rend)?;
    // After the target: an optional trailing `2>&1`, then EOF.
    let mut j = skip_spaces(b, tend);
    let mut combine = matches!(kind, OutKind::AmpOut | OutKind::AmpAppend);
    if let Some((_, de)) = dups.iter().find(|(ds, _)| *ds == j) {
        combine = true;
        j = skip_spaces(b, *de);
    }
    if j != n {
        return None; // trailing junk (incl. a second dup)
    }
    Some(ScanResult {
        redir_start: rstart,
        target_start: tstart,
        target_end: tend,
        append: matches!(kind, OutKind::Append | OutKind::AmpAppend),
        combine_stderr: combine,
    })
}

fn skip_spaces(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_space(b[i]) {
        i += 1;
    }
    i
}

/// Read one shell word (quote-aware) starting at/after `from`, skipping leading
/// spaces. Returns the word's byte range, or None if there's no word.
fn read_word(b: &[u8], from: usize) -> Option<(usize, usize)> {
    let n = b.len();
    let mut i = skip_spaces(b, from);
    if i >= n {
        return None;
    }
    let start = i;
    let mut in_single = false;
    let mut in_double = false;
    while i < n {
        let c = b[i];
        if in_single {
            if c == b'\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == b'\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' => {
                in_single = true;
                i += 1;
            }
            b'"' => {
                in_double = true;
                i += 1;
            }
            b'\\' => {
                i += 2;
            }
            _ if is_space(c) => break,
            // operator chars terminate a bare word
            b'>' | b'<' | b'|' | b'&' | b';' | b'(' | b')' => break,
            _ => {
                i += 1;
            }
        }
    }
    if i == start {
        return None;
    }
    Some((start, i))
}

/// Quote a value for embedding in a shell argv position.
///
/// The tool-use-id is provider-generated and currently always opaque
/// ASCII, but be defensive: if it contains a non-safe char, escape
/// per the host shell's rules.
///
/// - **Unix shells** (bash/sh/zsh): single-quote wrap with `'\''`
///   escape for embedded single quotes.
/// - **Windows `cmd.exe /C`**: cmd doesn't interpret single quotes
///   as quoting; we use double quotes and escape embedded `"` per
///   cmd.exe rules. This is the shell Claude Code's Bash tool
///   invokes on Win32.
///
/// Pure-alphanumeric ids return unchanged on every platform.
fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return s.to_string();
    }
    #[cfg(windows)]
    {
        // cmd.exe: double-quote wrap, escape `"` as `""`. Reasonable
        // approximation — full cmd.exe quoting is famously gnarly,
        // but for opaque-ID-with-weird-char defense this is enough.
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    }
    #[cfg(not(windows))]
    {
        // bash/sh/zsh: single-quote wrap, escape `'` as `'\''`.
        let escaped = s.replace('\'', r"'\''");
        format!("'{}'", escaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(cmd: &str, id: &str) -> String {
        json!({
            "tool_name": "Bash",
            "tool_use_id": id,
            "tool_input": { "command": cmd },
            "session_id": "sess-1",
            "hook_event_name": "PreToolUse"
        })
        .to_string()
    }

    #[test]
    fn rewrites_simple_bash_command() {
        let resp = build_response(&payload("echo hi", "toolu_abc"));
        let cmd = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.starts_with("agentmux-bashwrap exec"));
        assert!(cmd.contains("--tool-id=toolu_abc"));
        // base64("echo hi") = ZWNobyBoaQ
        assert!(cmd.contains("--b64-cmd=ZWNobyBoaQ"));
        assert_eq!(
            resp["hookSpecificOutput"]["permissionDecision"].as_str(),
            Some("allow")
        );
    }

    #[test]
    fn preserves_multi_line_and_quotes_via_base64() {
        let cmd = r#"echo "hello" && echo 'multi
line' && cat $HOME/.env"#;
        let resp = build_response(&payload(cmd, "toolu_x"));
        let wrapped = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        // The wrapped form must round-trip — pull the b64 back and decode.
        let b64 = wrapped
            .split("--b64-cmd=")
            .nth(1)
            .expect("b64 arg present");
        let decoded = URL_SAFE_NO_PAD.decode(b64.as_bytes()).unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        assert_eq!(decoded, cmd);
    }

    #[test]
    fn is_no_op_on_already_wrapped_command() {
        let cmd =
            "agentmux-bashwrap exec --tool-id=toolu_y --b64-cmd=ZWNobyBoaQ";
        let resp = build_response(&payload(cmd, "toolu_y"));
        assert!(
            resp.as_object().map(|m| m.is_empty()).unwrap_or(false),
            "should pass through (empty object), got {}",
            resp
        );
    }

    #[test]
    fn passes_through_non_bash_tools() {
        let payload = json!({
            "tool_name": "Read",
            "tool_use_id": "toolu_r",
            "tool_input": { "file_path": "x.txt" }
        })
        .to_string();
        let resp = build_response(&payload);
        assert!(
            resp.as_object().map(|m| m.is_empty()).unwrap_or(false),
            "non-Bash should pass through"
        );
    }

    #[test]
    fn passes_through_malformed_input() {
        let resp = build_response("not json");
        assert!(
            resp.as_object().map(|m| m.is_empty()).unwrap_or(false),
            "malformed → passthrough"
        );
    }

    #[test]
    fn rewrites_with_empty_b64_when_command_field_missing() {
        // Codex P2 round 3: the previous name `passes_through_when_
        // command_field_missing` lied about behavior, and the inline
        // comment claimed `bash -c ""` would "fail at wrapper start
        // time" — which is wrong; an empty bash command exits 0
        // silently. Renamed + comment corrected to describe what
        // actually happens.
        //
        // Behavior under test: when `tool_input.command` is missing,
        // we still emit a rewrite (with `--b64-cmd=` of an empty
        // string). The wrapper then runs `bash -c ""` which exits 0
        // immediately. Net effect: a no-op Bash call streams a "0
        // chars" system chunk + an empty body, and Claude sees an
        // empty tool_result. Not catastrophic; the empty input was
        // already going to do nothing.
        let payload = json!({
            "tool_name": "Bash",
            "tool_use_id": "toolu_z",
            "tool_input": {}
        })
        .to_string();
        let resp = build_response(&payload);
        let cmd = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        // The b64 of an empty string is the empty string, so we
        // expect `--b64-cmd=` immediately followed by whatever ends
        // the command (whitespace or EOL). Match the prefix.
        assert!(
            cmd.contains("--b64-cmd=") && (cmd.ends_with("--b64-cmd=") || cmd.contains("--b64-cmd= ")),
            "expected empty b64-cmd argument, got: {}",
            cmd
        );
    }

    #[test]
    fn threads_declared_background_flag_when_run_in_background_true() {
        let payload = json!({
            "tool_name": "Bash",
            "tool_use_id": "toolu_bg",
            "tool_input": { "command": "task dev", "run_in_background": true }
        })
        .to_string();
        let resp = build_response(&payload);
        let cmd = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        assert!(
            cmd.ends_with(" --declared-background"),
            "expected the wrapped command to carry --declared-background, got: {cmd}"
        );
    }

    #[test]
    fn omits_declared_background_flag_when_run_in_background_false_or_absent() {
        let with_false = json!({
            "tool_name": "Bash",
            "tool_use_id": "toolu_fg",
            "tool_input": { "command": "echo hi", "run_in_background": false }
        })
        .to_string();
        let without_field = json!({
            "tool_name": "Bash",
            "tool_use_id": "toolu_fg2",
            "tool_input": { "command": "echo hi" }
        })
        .to_string();
        for payload in [with_false, without_field] {
            let resp = build_response(&payload);
            let cmd = resp["hookSpecificOutput"]["updatedInput"]["command"]
                .as_str()
                .unwrap();
            assert!(
                !cmd.contains("--declared-background"),
                "expected no --declared-background flag, got: {cmd}"
            );
        }
    }

    #[test]
    fn shell_quote_passes_safe_ids_unchanged() {
        assert_eq!(shell_quote("toolu_abc-123"), "toolu_abc-123");
    }

    #[cfg(not(windows))]
    #[test]
    fn shell_quote_wraps_unsafe_ids_unix() {
        let q = shell_quote("weird id");
        assert_eq!(q, "'weird id'");
    }

    #[cfg(not(windows))]
    #[test]
    fn shell_quote_escapes_embedded_single_quotes_unix() {
        // Standard bash idiom: `'` → `'\''` (close, escaped, reopen).
        // For `o'malley` the result is `'o'\''malley'`.
        let q = shell_quote("o'malley");
        assert_eq!(q, r"'o'\''malley'");
    }

    #[cfg(windows)]
    #[test]
    fn shell_quote_wraps_unsafe_ids_windows() {
        let q = shell_quote("weird id");
        assert_eq!(q, "\"weird id\"");
    }

    #[cfg(windows)]
    #[test]
    fn shell_quote_escapes_embedded_double_quotes_windows() {
        // cmd.exe `""` is the escape for a literal `"` inside a
        // double-quoted argument. For `a"b` the result is `"a""b"`.
        let q = shell_quote("a\"b");
        assert_eq!(q, "\"a\"\"b\"");
    }

    // ── tee redirect rewrite (Feature 1) ──────────────────────────────

    #[test]
    fn tee_rewrites_basic_redirect_forms() {
        assert_eq!(
            tee_redirect_rewrite("task package > build.log").unwrap(),
            "set -o pipefail; { task package ; } | tee -- build.log"
        );
        assert_eq!(
            tee_redirect_rewrite("make >> out.log").unwrap(),
            "set -o pipefail; { make ; } | tee -a -- out.log"
        );
        assert_eq!(
            tee_redirect_rewrite("task package > build.log 2>&1").unwrap(),
            "set -o pipefail; { task package ; } 2>&1 | tee -- build.log"
        );
        assert_eq!(
            tee_redirect_rewrite("make >> out.log 2>&1").unwrap(),
            "set -o pipefail; { make ; } 2>&1 | tee -a -- out.log"
        );
        assert_eq!(
            tee_redirect_rewrite("cmd &> all.log").unwrap(),
            "set -o pipefail; { cmd ; } 2>&1 | tee -- all.log"
        );
        assert_eq!(
            tee_redirect_rewrite("cmd &>> all.log").unwrap(),
            "set -o pipefail; { cmd ; } 2>&1 | tee -a -- all.log"
        );
    }

    #[test]
    fn tee_bails_on_internal_pipeline() {
        // A pipe inside the command bails: `set -o pipefail` on the tee wrapper
        // would change the inner pipeline's exit-code semantics vs the original.
        assert!(tee_redirect_rewrite("cargo build | grep error > errors.log").is_none());
    }

    #[test]
    fn tee_bails_on_multiline_command() {
        // A multi-line command with a trailing redirect on the last line must NOT
        // be wrapped — wrapping would tee the earlier lines' output into the file.
        assert!(tee_redirect_rewrite("echo building\nmake > build.log").is_none());
        assert!(tee_redirect_rewrite("cd src\nmake > build.log 2>&1").is_none());
        // A backslash-newline line continuation is NOT a separator — still rewritten.
        assert_eq!(
            tee_redirect_rewrite("make \\\n  --flag > build.log").unwrap(),
            "set -o pipefail; { make \\\n  --flag ; } | tee -- build.log"
        );
    }

    #[test]
    fn tee_handles_subshell_command() {
        assert_eq!(
            tee_redirect_rewrite("(cd src && make) > b.log 2>&1").unwrap(),
            "set -o pipefail; { (cd src && make) ; } 2>&1 | tee -- b.log"
        );
    }

    #[test]
    fn tee_preserves_quoted_target() {
        assert_eq!(
            tee_redirect_rewrite(r#"cmd > "my out.log""#).unwrap(),
            r#"set -o pipefail; { cmd ; } | tee -- "my out.log""#
        );
    }

    #[test]
    fn tee_bails_on_dev_null() {
        assert!(tee_redirect_rewrite("noisy > /dev/null").is_none());
        assert!(tee_redirect_rewrite("noisy > /dev/null 2>&1").is_none());
        assert!(tee_redirect_rewrite(r#"noisy > "/dev/null""#).is_none());
    }

    #[test]
    fn tee_bails_on_quoted_comment_heredoc_procsub() {
        assert!(tee_redirect_rewrite("echo 'a > b'").is_none()); // quoted >
        assert!(tee_redirect_rewrite("echo \"x > y\"").is_none()); // quoted >
        assert!(tee_redirect_rewrite("make # writes > log").is_none()); // comment
        assert!(tee_redirect_rewrite("cat <<EOF > f\nhi\nEOF").is_none()); // heredoc
        assert!(tee_redirect_rewrite("diff <(a) > f").is_none()); // process subst
    }

    #[test]
    fn tee_bails_on_non_trailing_multiple_or_fd_redirects() {
        assert!(tee_redirect_rewrite("cmd > f | grep x").is_none()); // not trailing
        assert!(tee_redirect_rewrite("cmd > f && other").is_none()); // list op
        assert!(tee_redirect_rewrite("a > f1 > f2").is_none()); // two redirects
        assert!(tee_redirect_rewrite("cmd 2> err.log").is_none()); // fd-specific
        assert!(tee_redirect_rewrite("cmd >&2").is_none()); // dup
        assert!(tee_redirect_rewrite("cmd 2>&1 > f").is_none()); // leading 2>&1 order
        assert!(tee_redirect_rewrite("echo hi").is_none()); // no redirect
        assert!(tee_redirect_rewrite("server > log &").is_none()); // background
    }

    #[test]
    fn tee_handles_param_expansion() {
        // A literal `>` inside a ${...} expansion is NOT a redirect → no rewrite.
        assert!(tee_redirect_rewrite("echo ${X:->f}").is_none());
        assert!(tee_redirect_rewrite("echo ${X/>/_}").is_none());
        // A real trailing redirect to a ${...}-bearing target IS rewritten.
        assert_eq!(
            tee_redirect_rewrite("make > ${LOG}/out.log").unwrap(),
            "set -o pipefail; { make ; } | tee -- ${LOG}/out.log"
        );
        // ...and a ${...} with an inner `>` plus a real trailing redirect.
        assert_eq!(
            tee_redirect_rewrite("echo ${X/>/_} > out.log").unwrap(),
            "set -o pipefail; { echo ${X/>/_} ; } | tee -- out.log"
        );
    }

    #[test]
    fn tee_rewrite_round_trips_through_build_response_b64() {
        let resp = build_response(&payload("task package > build.log 2>&1", "toolu_t"));
        let wrapped = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        let b64 = wrapped.split("--b64-cmd=").nth(1).expect("b64 arg");
        let decoded =
            String::from_utf8(URL_SAFE_NO_PAD.decode(b64.as_bytes()).unwrap()).unwrap();
        assert_eq!(
            decoded,
            "set -o pipefail; { task package ; } 2>&1 | tee -- build.log"
        );
        // pipefail present → pipeline exit code reflects the command, not tee.
        assert!(decoded.starts_with("set -o pipefail;"));
    }

    #[test]
    fn tee_non_redirect_command_encodes_verbatim() {
        // No recognized redirect → command encodes unchanged (today's behavior).
        let resp = build_response(&payload("echo hi", "toolu_a"));
        let wrapped = resp["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        let b64 = wrapped.split("--b64-cmd=").nth(1).unwrap();
        let decoded =
            String::from_utf8(URL_SAFE_NO_PAD.decode(b64.as_bytes()).unwrap()).unwrap();
        assert_eq!(decoded, "echo hi");
    }
}
