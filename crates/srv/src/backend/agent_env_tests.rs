// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn a_line_reads_the_way_the_launch_reads_it() {
    assert_eq!(classify("   "), Line::Blank);
    assert_eq!(classify("# a note"), Line::Comment);
    assert_eq!(classify("  CLAUDE_CODE_FOO=bar baz  "), Line::Pair { key: "CLAUDE_CODE_FOO", value: "bar baz" });
    assert_eq!(classify("CLAUDE_CODE_EMPTY="), Line::Pair { key: "CLAUDE_CODE_EMPTY", value: "" });
    assert_eq!(classify("CLAUDE_CODE_A=b=c"), Line::Pair { key: "CLAUDE_CODE_A", value: "b=c" });
    // The launch skips these: no `=`, nothing before it, a space in the key, a leading digit.
    for skipped in ["CLAUDE_CODE_FOO", "=x", "FOO = 1", "1A=b", "A-CLAUDE_CODE_B=c"] {
        assert_eq!(classify(skipped), Line::Ignored, "{skipped}");
    }
}

#[test]
fn the_last_of_a_repeated_name_wins() {
    assert_eq!(effective("CLAUDE_CODE_A=1\n# c\nCLAUDE_CODE_B=2\nCLAUDE_CODE_A=3\n"), map(&[("CLAUDE_CODE_A", "3"), ("CLAUDE_CODE_B", "2")]));
}

#[test]
fn names_agentmux_owns_or_that_hold_credentials_are_refused() {
    for name in [
        "AGENTMUX_AGENT_ID",
        "agentmux_anything",
        "PATH",
        "path",
        "HOME",
        "USERPROFILE",
        "GH_CONFIG_DIR",
        "CLAUDE_CONFIG_DIR",
        "CODEX_HOME",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_BASE_URL",
        "MY_SERVICE_TOKEN",
        "DB_PASSWORD",
        "AWS_SECRET_ACCESS_KEY_ID_KEY",
        "HTTPS_PROXY",
        "NODE_OPTIONS",
        "BASH_ENV",
        "ENV",
        "LD_AUDIT",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "NODE_PATH",
        "NODE_EXTRA_CA_CERTS",
        "GIT_SSH_COMMAND",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_COUNT",
        "XDG_CONFIG_HOME",
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "MY_SERVICE_BASE_URL",
        "MY_SERVICE_ENDPOINT",
        "SOME_API_URL",
        "PROMPT_COMMAND",
        "EDITOR",
        "VISUAL",
        "PAGER",
        "GIT_PAGER",
        "GIT_EDITOR",
        "GIT_SEQUENCE_EDITOR",
        "BROWSER",
        "LESSOPEN",
        "MANPAGER",
        "OPENAI_API_BASE",
        "CLAUDE_CODE_SHELL_PREFIX",
        "CLAUDE_CODE_API_KEY_HELPER",
        "_X1",
        "1BAD",
        "has space",
    ] {
        assert!(name_problem(name).is_some(), "{name} should be refused");
    }
}

#[test]
fn ordinary_settings_are_allowed() {
    for name in ["CLAUDE_CODE_THEME", "RUST_LOG", "CLAUDE_CODE_MAX_OUTPUT_TOKENS", "CLAUDE_CODE_AUTO_COMPACT_WINDOW", "DISABLE_TELEMETRY", "MAX_THINKING_TOKENS", "TZ"] {
        assert_eq!(name_problem(name), None, "{name}");
    }
}

#[test]
fn a_value_that_looks_like_a_secret_is_refused() {
    assert!(value_problem("CLAUDE_CODE_NOTE", "ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_some());
    assert!(value_problem("CLAUDE_CODE_NOTE", "sk-abcdefghijklmnopqrstuvwx").is_some());
    assert!(value_problem("CLAUDE_CODE_NOTE", "line\rbreak").is_some());
    assert!(value_problem("CLAUDE_CODE_NOTE", HIDDEN).is_some());
    assert_eq!(value_problem("CLAUDE_CODE_NOTE", "debug,trace"), None);
    assert_eq!(value_problem("CLAUDE_CODE_NOTE", "C:\\tools\\bin"), None);
    assert_eq!(value_problem("CLAUDE_CODE_NOTE", ""), None);
}

#[test]
fn the_size_is_bounded_in_lines_and_bytes() {
    let sixty_four = "CLAUDE_CODE_A=1\n".repeat(MAX_LINES);
    assert_eq!(check_size(&sixty_four), Ok(()));
    assert!(check_size(&format!("{sixty_four}CLAUDE_CODE_B=2\n")).unwrap_err().contains("65 lines"));
    let big = format!("CLAUDE_CODE_A={}\n", "x".repeat(MAX_BYTES));
    assert!(check_size(&big).unwrap_err().contains("bytes"));
    assert_eq!(check_size(&format!("CLAUDE_CODE_A={}", "x".repeat(MAX_BYTES - 14))), Ok(()));
}

#[test]
fn whole_text_keeps_comments_and_blanks_and_refuses_what_would_be_skipped() {
    let text = "# editor\n\nCLAUDE_CODE_THEME=vim\n\n# more\nRUST_LOG=debug\n";
    assert_eq!(resolve_lines("", text), Ok(text.to_string()));
    assert!(resolve_lines("", "CLAUDE_CODE_THEME=vim\nnot a setting\n").unwrap_err().starts_with("line 2:"));
    assert!(resolve_lines("", "# ok\nPATH=/x\n").unwrap_err().starts_with("line 2:"));
    assert!(resolve_lines("", "CLAUDE_CODE_A=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n").is_err());
    assert!(resolve_lines("", &format!("CLAUDE_CODE_A={HIDDEN}\n")).is_err());
}

#[test]
fn a_whole_text_edit_keeps_the_lines_an_agent_cannot_write() {
    let current = "SERVICE_TOKEN=operator-value-123\nCLAUDE_CODE_THEME=vim\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n";
    // Written back as shown: each goes back where it was.
    let shown = redacted(current);
    assert_eq!(resolve_lines(current, &shown), Ok(current.to_string()));
    // Moved and edited around: the operator's lines come back in the new place.
    let edited = "CLAUDE_CODE_THEME=nano\nSERVICE_TOKEN=[hidden]\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n";
    assert_eq!(
        resolve_lines(current, edited),
        Ok("CLAUDE_CODE_THEME=nano\nSERVICE_TOKEN=operator-value-123\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n".to_string())
    );
    // Left out: still there, at the end.
    assert_eq!(
        resolve_lines(current, "CLAUDE_CODE_THEME=nano\n"),
        Ok("CLAUDE_CODE_THEME=nano\nSERVICE_TOKEN=operator-value-123\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n".to_string())
    );
    // A new value for one is still refused, and [hidden] for a name the operator never set is too.
    assert!(resolve_lines(current, "SERVICE_TOKEN=mine\n").is_err());
    assert!(resolve_lines(current, "PATH=/other\n").is_err());
    assert!(resolve_lines(current, "OTHER_TOKEN=[hidden]\n").is_err());
    assert!(resolve_lines(current, "CLAUDE_CODE_THEME=[hidden]\n").is_err());
}

#[test]
fn a_secret_looking_value_on_an_ordinary_name_is_kept_too() {
    let current = "CLAUDE_CODE_NOTES=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nCLAUDE_CODE_THEME=vim\n";
    assert_eq!(redacted(current), "CLAUDE_CODE_NOTES=[hidden]\nCLAUDE_CODE_THEME=vim\n");
    assert_eq!(resolve_lines(current, "CLAUDE_CODE_NOTES=[hidden]\nCLAUDE_CODE_THEME=nano\n"), Ok("CLAUDE_CODE_NOTES=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nCLAUDE_CODE_THEME=nano\n".to_string()));
}

#[test]
fn an_edit_replaces_in_place_appends_new_and_keeps_everything_else() {
    let current = "# tools\nCLAUDE_CODE_THEME=vim\n\nRUST_LOG=info\nCLAUDE_CODE_OTHER=1\n";
    let edited = edit(current, &map(&[("RUST_LOG", "debug"), ("CLAUDE_CODE_NEW", "x y")]), &["CLAUDE_CODE_OTHER".to_string()]).unwrap();
    assert_eq!(edited, "# tools\nCLAUDE_CODE_THEME=vim\n\nRUST_LOG=debug\nCLAUDE_CODE_NEW=x y\n");
}

#[test]
fn an_edit_of_nothing_starts_the_text_and_removing_the_last_leaves_it_empty() {
    assert_eq!(edit("", &map(&[("CLAUDE_CODE_A", "1")]), &[]).unwrap(), "CLAUDE_CODE_A=1\n");
    assert_eq!(edit("CLAUDE_CODE_A=1\n", &BTreeMap::new(), &["CLAUDE_CODE_A".to_string()]).unwrap(), "");
    assert_eq!(edit("CLAUDE_CODE_A=1", &map(&[("CLAUDE_CODE_B", "2")]), &[]).unwrap(), "CLAUDE_CODE_A=1\nCLAUDE_CODE_B=2");
}

#[test]
fn an_edit_replaces_every_copy_of_a_repeated_name() {
    assert_eq!(edit("CLAUDE_CODE_A=1\nCLAUDE_CODE_A=2\n", &map(&[("CLAUDE_CODE_A", "3")]), &[]).unwrap(), "CLAUDE_CODE_A=3\nCLAUDE_CODE_A=3\n");
    assert_eq!(edit("CLAUDE_CODE_A=1\nCLAUDE_CODE_B=2\nCLAUDE_CODE_A=2\n", &BTreeMap::new(), &["CLAUDE_CODE_A".to_string()]).unwrap(), "CLAUDE_CODE_B=2\n");
}

#[test]
fn an_edit_cannot_touch_a_refused_name_either_way() {
    assert!(edit("", &map(&[("PATH", "/x")]), &[]).is_err());
    assert!(edit("ANTHROPIC_API_KEY=abc\n", &BTreeMap::new(), &["ANTHROPIC_API_KEY".to_string()]).is_err());
    assert!(edit("", &map(&[("CLAUDE_CODE_A", "ghp_abcdefghijklmnopqrstuvwxyz0123456789")]), &[]).is_err());
}

#[test]
fn the_diff_names_what_was_added_changed_and_removed() {
    let change = diff("CLAUDE_CODE_A=1\nCLAUDE_CODE_B=2\nC=3\n", "CLAUDE_CODE_A=1\nCLAUDE_CODE_B=20\nD=4\n");
    assert_eq!(change.added, map(&[("D", "4")]));
    assert_eq!(change.changed.get("CLAUDE_CODE_B"), Some(&ValueChange { from: "2".into(), to: "20".into() }));
    assert_eq!(change.removed, vec!["C".to_string()]);
    assert_eq!(change.summary(), "added [D] changed [CLAUDE_CODE_B] removed [C]");
    assert!(diff("CLAUDE_CODE_A=1\n# c\n", "# other\nCLAUDE_CODE_A=1\n").is_empty());
}

#[test]
fn credential_looking_values_are_never_shown_to_an_agent() {
    let text = "CLAUDE_CODE_THEME=vim\nSERVICE_TOKEN=abc123\nCLAUDE_CODE_NOTE=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n# kept\n";
    assert_eq!(
        redacted(text),
        "CLAUDE_CODE_THEME=vim\nSERVICE_TOKEN=[hidden]\nCLAUDE_CODE_NOTE=[hidden]\n# kept\n"
    );
    let change = diff("", text);
    assert_eq!(change.added.get("CLAUDE_CODE_THEME").map(String::as_str), Some("vim"));
    assert_eq!(change.added.get("SERVICE_TOKEN").map(String::as_str), Some(HIDDEN));
    assert_eq!(change.added.get("CLAUDE_CODE_NOTE").map(String::as_str), Some(HIDDEN));
}
