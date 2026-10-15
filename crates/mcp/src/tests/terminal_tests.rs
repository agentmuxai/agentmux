// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The parts of `Terminal` that decide what reaches the PTY and what the agent
//! reads back (SPEC_AGENT_TERMINAL_DRIVING_2026_10_15.md §4.0, §6).

use super::*;

#[test]
fn enter_is_cr_everywhere_and_line_breaks_are_typed_as_enter() {
    assert_eq!(key_bytes("Enter").as_deref(), Some("\r"));
    assert_eq!(key_bytes("return").as_deref(), Some("\r"));
    assert_eq!(to_cr("dir\n"), "dir\r");
    assert_eq!(to_cr("a\r\nb\nc"), "a\rb\rc");
    assert_eq!(to_cr("no break"), "no break");
}

#[test]
fn keys_by_name() {
    assert_eq!(key_bytes("Ctrl+C").as_deref(), Some("\u{3}"));
    assert_eq!(key_bytes("ctrl+d").as_deref(), Some("\u{4}"));
    assert_eq!(key_bytes("Ctrl+[").as_deref(), Some("\x1b"));
    assert_eq!(key_bytes("Esc").as_deref(), Some("\x1b"));
    assert_eq!(key_bytes("Tab").as_deref(), Some("\t"));
    assert_eq!(key_bytes("Shift+Tab").as_deref(), Some("\x1b[Z"));
    assert_eq!(key_bytes("Up").as_deref(), Some("\x1b[A"));
    assert_eq!(key_bytes("PageDown").as_deref(), Some("\x1b[6~"));
    assert_eq!(key_bytes("F1").as_deref(), Some("\x1bOP"));
    assert_eq!(key_bytes("F12").as_deref(), Some("\x1b[24~"));
    assert_eq!(key_bytes("Backspace").as_deref(), Some("\x7f"));
    assert_eq!(key_bytes("Space").as_deref(), Some(" "));
    // Alt keeps the key's case; Alt with a named key prefixes its sequence.
    assert_eq!(key_bytes("Alt+X").as_deref(), Some("\x1bX"));
    assert_eq!(key_bytes("Alt+Enter").as_deref(), Some("\x1b\r"));
}

#[test]
fn an_unknown_key_is_refused_never_typed() {
    for bad in ["Enterr", "Ctrl+", "Ctrl+Shift+C", "Hyper+A", "", "F13"] {
        assert!(key_bytes(bad).is_none(), "{bad:?}");
    }
    let err = key_list(&json!({ "keys": ["Enter", "Nope"] })).unwrap_err().to_string();
    assert!(err.contains("unknown key \"Nope\""), "{err}");
    assert_eq!(key_list(&json!({ "keys": "Ctrl+C" })).unwrap(), vec!["\u{3}".to_string()]);
}

#[test]
fn long_input_is_chunked_on_char_boundaries() {
    let s = "é".repeat(1000);
    let parts = chunks(&s, 400);
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|p| p.chars().count() <= 400));
    assert_eq!(parts.concat(), s);
    assert_eq!(chunks("", 400).len(), 0);
}

#[test]
fn output_reads_back_as_plain_text() {
    // Colour (CSI), a window title (OSC with BEL), a cwd report (OSC with ST),
    // cursor show/hide, CRLF.
    let raw = "\x1b[32mok\x1b[0m\r\n\x1b]0;C:\\Windows\x07\x1b]7;file:///c/x\x1b\\next\x1b[?25l line\r\n";
    assert_eq!(strip_ansi(raw), "ok\nnext line\n");
    // A progress bar redrawn with CR reads as its last state.
    assert_eq!(strip_ansi("10%\r50%\r100%\r\ndone"), "100%\ndone");
    // Backspace is applied; other control bytes go.
    assert_eq!(strip_ansi("abx\x08c\u{7}\n"), "abc\n");
    // A trailing prompt with no newline is kept.
    assert_eq!(strip_ansi("out\r\nC:\\work>"), "out\nC:\\work>");
    // ConPTY moving the cursor to the next row instead of writing CRLF.
    assert_eq!(strip_ansi("reserved.\x1b[4;1HC:\\work>"), "reserved.\nC:\\work>");
    // A cursor move on an empty line adds no blank line.
    assert_eq!(strip_ansi("\x1b[1;1Hfirst"), "first");
}

#[test]
fn the_exit_marker_is_in_the_shells_own_syntax() {
    assert_eq!(
        with_exit_marker("npm test", ShellFamily::Cmd, "ab12cd34"),
        "npm test & call echo __AMX_RC_ab12cd34_%^ERRORLEVEL%__"
    );
    assert_eq!(
        with_exit_marker("npm test", ShellFamily::Posix, "ab12cd34"),
        "npm test; echo \"__AMX_RC_ab12cd34_$?__\""
    );
    // run clears a half-typed line first: Esc in cmd, Ctrl+U in a Unix shell.
    assert_eq!(clear_line(ShellFamily::Cmd), "\x1b");
    assert_eq!(clear_line(ShellFamily::Posix), "\u{15}");
}

#[test]
fn the_exit_marker_gives_the_commands_own_output_and_code() {
    // cmd: the echoed line, the output, the marker line, the next prompt.
    let cmd = "C:\\w>npm test & call echo __AMX_RC_n1_%^ERRORLEVEL%__\n1 passing\n2 failing\n__AMX_RC_n1_1__\n\nC:\\w>";
    assert_eq!(exit_marker(cmd, "n1"), Some(("1 passing\n2 failing".to_string(), 1)));
    // POSIX, a negative code, no output.
    let sh = "$ true; echo \"__AMX_RC_n2_$?__\"\n__AMX_RC_n2_-1__\n$ ";
    assert_eq!(exit_marker(sh, "n2"), Some((String::new(), -1)));
    // The echo alone (no digits) is not a result.
    assert_eq!(exit_marker("C:\\w>x & call echo __AMX_RC_n3_%^ERRORLEVEL%__\n", "n3"), None);
    assert_eq!(exit_marker("$ x; echo \"__AMX_RC_n4_$?__\"\nstill running", "n4"), None);
    // An earlier run's marker is not this run's result.
    assert_eq!(exit_marker("$ y; echo \"__AMX_RC_new_$?__\"\n__AMX_RC_old_0__\n", "new"), None);
}

#[test]
fn a_prompt_is_recognised_per_shell() {
    assert!(at_prompt("out\nC:\\Users\\a>", ShellFamily::Cmd));
    assert!(at_prompt("out\nC:\\Users\\a>\n\n", ShellFamily::Cmd));
    assert!(at_prompt("PS C:\\Users\\a>", ShellFamily::Cmd));
    assert!(!at_prompt("Building...", ShellFamily::Cmd));
    assert!(at_prompt("user@host:~/x$ ", ShellFamily::Posix));
    assert!(at_prompt("# ", ShellFamily::Posix));
    assert!(at_prompt("~/x ❯", ShellFamily::Posix));
    assert!(!at_prompt("Compiling crate", ShellFamily::Posix));
    assert!(!at_prompt("", ShellFamily::Posix));
}

#[test]
fn a_wait_needs_a_known_condition() {
    assert!(parse_until(&json!({ "until": "pattern", "pattern": "ready on (\\d+)" })).is_ok());
    assert!(parse_until(&json!({ "until": "pattern", "pattern": "a.b", "fixed": true })).is_ok());
    let missing = parse_until(&json!({ "until": "pattern" })).unwrap_err().to_string();
    assert!(missing.contains("needs pattern"), "{missing}");
    let bad = parse_until(&json!({ "until": "pattern", "pattern": "(" })).unwrap_err().to_string();
    assert!(bad.starts_with("pattern:"), "{bad}");
    assert!(parse_until(&json!({ "until": "soon" })).is_err());
    assert!(parse_until(&json!({})).is_err());
    // fixed: true matches the text literally.
    if let Ok(Until::Pattern(re)) = parse_until(&json!({ "until": "pattern", "pattern": "a.b", "fixed": true })) {
        assert!(re.is_match("xa.by") && !re.is_match("axb"));
    } else {
        panic!("fixed pattern");
    }
}

#[test]
fn the_echo_of_a_typed_line_is_skipped() {
    assert_eq!(after_first_line("C:\\w>npm run dev\nready on 3000\n"), "ready on 3000\n");
    assert_eq!(after_first_line("only the echo"), "");
}

#[test]
fn every_action_is_in_the_schema() {
    let schema: Value = serde_json::from_str(crate::tool_schemas::TERMINAL_TOOL).unwrap();
    let listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let mut want = ACTIONS.to_vec();
    want.sort();
    let mut got = listed.clone();
    got.sort();
    assert_eq!(got, want);
}

#[tokio::test]
async fn an_unknown_action_lists_the_actions() {
    let client = reqwest::Client::new();
    let loops = crate::LoopRegistry::default();
    let counter = std::sync::atomic::AtomicU64::new(0);
    let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
    let err = call("Terminal", &json!({ "action": "launch" }), &cx).await.unwrap_err().to_string();
    assert!(err.contains("unknown action \"launch\"") && err.contains("open, type, read"), "{err}");
    let err = call("Terminal", &json!({}), &cx).await.unwrap_err().to_string();
    assert!(err.starts_with("missing required parameter: action"), "{err}");
}
