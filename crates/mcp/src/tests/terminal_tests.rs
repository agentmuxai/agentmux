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
    // fish has no $?: its status, braced so the name ends before the underscores.
    assert_eq!(
        with_exit_marker("npm test", ShellFamily::Fish, "ab12cd34"),
        "npm test; echo \"__AMX_RC_ab12cd34_\"$status\"__\""
    );
    // PowerShell: a native command's exit code, else the cmdlet's success.
    assert_eq!(
        with_exit_marker("npm test", ShellFamily::Pwsh, "n"),
        "npm test; echo \"__AMX_RC_n_$(if ($?) { 0 } elseif ($LASTEXITCODE) { $LASTEXITCODE } else { 1 })__\""
    );
    assert_eq!(clear_line(ShellFamily::Pwsh), "\x1b");
    assert_eq!(in_dir("x", "C:\\w", ShellFamily::Pwsh), "Set-Location -LiteralPath \"C:\\w\"; x");
    // run's cwd is a cd in front of the command, in the shell's syntax.
    assert_eq!(in_dir("npm test", "D:\\w", ShellFamily::Cmd), "cd /d \"D:\\w\" && npm test");
    assert_eq!(in_dir("npm test", "/w", ShellFamily::Posix), "cd \"/w\" && npm test");
    // run clears a half-typed line first: Esc in cmd, Ctrl+U in a Unix shell.
    assert_eq!(clear_line(ShellFamily::Cmd), "\x1b");
    assert_eq!(clear_line(ShellFamily::Posix), "\u{15}");
    assert_eq!(clear_line(ShellFamily::Fish), "\u{15}");
    assert!(at_prompt("~/x> ", ShellFamily::Fish));
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
    // Output that doesn't end in a newline is kept: the marker follows it on its line.
    let printf = "$ printf x; echo \"__AMX_RC_n5_$?__\"\nx__AMX_RC_n5_0__\n$ ";
    assert_eq!(exit_marker(printf, "n5"), Some(("x".to_string(), 0)));
    // An earlier run's marker is not this run's result.
    assert_eq!(exit_marker("$ y; echo \"__AMX_RC_new_$?__\"\n__AMX_RC_old_0__\n", "new"), None);
}

#[test]
fn a_command_left_open_is_refused_not_run_into_the_marker() {
    use ShellFamily::*;
    assert_eq!(open_ending("npm start &", Posix), Some("&"));
    assert_eq!(open_ending("make && ", Fish), Some("&&"));
    assert_eq!(open_ending("cat x |", Pwsh), Some("|"));
    assert_eq!(open_ending("a ||", Cmd), Some("||"));
    assert_eq!(open_ending("echo one \\", Posix), Some("\\"));
    assert_eq!(open_ending("dir ^", Cmd), Some("^"));
    assert_eq!(open_ending("Get-Item `", Pwsh), Some("`"));
    // Escaped, the character is an argument; doubled, the escape is.
    assert_eq!(open_ending("echo \\&", Posix), None);
    assert_eq!(open_ending("echo ^|", Cmd), None);
    assert_eq!(open_ending("echo a\\\\", Posix), None);
    assert_eq!(open_ending("npm test", Posix), None);
}

#[test]
fn a_trailing_semicolon_is_dropped_before_the_markers_own() {
    use ShellFamily::*;
    assert_eq!(without_trailing_semicolon("echo hi;", Posix), "echo hi");
    assert_eq!(without_trailing_semicolon("echo hi ; ", Fish), "echo hi");
    assert_eq!(without_trailing_semicolon("Get-Date;", Pwsh), "Get-Date");
    // Escaped, it's an argument; cmd has no separator to drop.
    assert_eq!(without_trailing_semicolon("echo \\;", Posix), "echo \\;");
    assert_eq!(without_trailing_semicolon("echo hi;", Cmd), "echo hi;");
    assert_eq!(without_trailing_semicolon("npm test", Posix), "npm test");
}

#[test]
fn the_echo_of_every_typed_line_is_skipped() {
    let text = "first\nsecond READY\nREADY\n";
    assert_eq!(after_lines(text, 0), text);
    assert_eq!(after_lines(text, 2), "READY\n");
    assert_eq!(after_lines(text, 5), "");
    assert_eq!(after_first_line(text), "second READY\nREADY\n");
}

#[test]
fn a_trailing_comment_is_dropped_so_it_cant_swallow_the_marker() {
    assert_eq!(without_trailing_comment("echo ok # explanation", ShellFamily::Posix), "echo ok");
    assert_eq!(without_trailing_comment("echo ok #", ShellFamily::Fish), "echo ok");
    // A # inside quotes, after an escape, or inside a word isn't a comment.
    assert_eq!(without_trailing_comment("echo '# not' \"# not\"", ShellFamily::Posix), "echo '# not' \"# not\"");
    assert_eq!(without_trailing_comment("echo \\# a#b", ShellFamily::Posix), "echo \\# a#b");
    assert_eq!(without_trailing_comment("git log --format=%H#x", ShellFamily::Posix), "git log --format=%H#x");
    // PowerShell escapes with a backtick; cmd has no # comments.
    assert_eq!(without_trailing_comment("echo `# x # y", ShellFamily::Pwsh), "echo `# x");
    assert_eq!(without_trailing_comment("echo a # b", ShellFamily::Cmd), "echo a # b");
}

#[test]
fn a_prompt_counts_for_any_shell_when_the_family_isnt_known() {
    assert!(at_any_prompt("C:\\w>", None));
    assert!(at_any_prompt("user@h:~$ ", None));
    assert!(!at_any_prompt("Building...", None));
    // A known family narrows it to that shell's prompt.
    assert!(!at_any_prompt("user@h:~$ ", Some(ShellFamily::Cmd)));
}

#[test]
fn a_remote_shell_is_told_apart_by_what_it_expands() {
    let line = probe_line("p1");
    assert_eq!(line, "echo __AMX_FAM_p1_%COMSPEC%_$FISH_VERSION.$PSVersionTable.end");
    // The echo of the probe line comes first and is skipped.
    let echo = format!("C:\\>{line}\n");
    assert_eq!(classify_probe(&format!("{echo}__AMX_FAM_p1_C:\\Windows\\system32\\cmd.exe_$FISH_VERSION.$PSVersionTable.end\n"), "p1"), Some(ShellFamily::Cmd));
    assert_eq!(classify_probe(&format!("{echo}__AMX_FAM_p1_%COMSPEC%_.System.Management.Automation.PSVersionHashTable.end\n"), "p1"), Some(ShellFamily::Pwsh));
    assert_eq!(classify_probe(&format!("{echo}__AMX_FAM_p1_%COMSPEC%_3.7.1..end\n"), "p1"), Some(ShellFamily::Fish));
    assert_eq!(classify_probe(&format!("{echo}__AMX_FAM_p1_%COMSPEC%_..end\n"), "p1"), Some(ShellFamily::Posix));
    // Only the echo so far: no answer yet.
    assert_eq!(classify_probe(&echo, "p1"), None);
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
