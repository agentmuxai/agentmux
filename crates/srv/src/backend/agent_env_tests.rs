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
    assert_eq!(classify("  NO_COLOR=bar baz  "), Line::Pair { key: "NO_COLOR", value: "bar baz" });
    assert_eq!(classify("MCP_TOOL_TIMEOUT="), Line::Pair { key: "MCP_TOOL_TIMEOUT", value: "" });
    assert_eq!(classify("TZ=b=c"), Line::Pair { key: "TZ", value: "b=c" });
    // The launch skips these: no `=`, nothing before it, a space in the key, a leading digit.
    for skipped in ["NO_COLOR", "=x", "FOO = 1", "1A=b", "A-LANG=c"] {
        assert_eq!(classify(skipped), Line::Ignored, "{skipped}");
    }
}

#[test]
fn the_last_of_a_repeated_name_wins() {
    assert_eq!(effective("TZ=1\n# c\nLANG=2\nTZ=3\n"), map(&[("TZ", "3"), ("LANG", "2")]));
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
        "CLAUDE_CODE_GIT_BASH_PATH",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_ANYTHING_ELSE",
        "DISABLE_ANYTHING_ELSE",
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
    for name in ["RUST_BACKTRACE", "RUST_LOG", "CLAUDE_CODE_MAX_OUTPUT_TOKENS", "CLAUDE_CODE_AUTO_COMPACT_WINDOW", "DISABLE_TELEMETRY", "MAX_THINKING_TOKENS", "TZ"] {
        assert_eq!(name_problem(name), None, "{name}");
    }
}

#[test]
fn a_value_that_looks_like_a_secret_is_refused() {
    assert!(value_problem("FORCE_COLOR", "ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_some());
    assert!(value_problem("FORCE_COLOR", "sk-abcdefghijklmnopqrstuvwx").is_some());
    assert!(value_problem("FORCE_COLOR", "line\rbreak").is_some());
    assert!(value_problem("FORCE_COLOR", HIDDEN).is_some());
    assert_eq!(value_problem("FORCE_COLOR", "debug,trace"), None);
    assert_eq!(value_problem("FORCE_COLOR", "C:\\tools\\bin"), None);
    assert_eq!(value_problem("FORCE_COLOR", ""), None);
}

#[test]
fn the_size_is_bounded_in_lines_and_bytes() {
    let sixty_four = "TZ=1\n".repeat(MAX_LINES);
    assert_eq!(check_size(&sixty_four), Ok(()));
    assert!(check_size(&format!("{sixty_four}LANG=2\n")).unwrap_err().contains("65 lines"));
    let big = format!("TZ={}\n", "x".repeat(MAX_BYTES));
    assert!(check_size(&big).unwrap_err().contains("bytes"));
    assert_eq!(check_size(&format!("TZ={}", "x".repeat(MAX_BYTES - 3))), Ok(()));
}

#[test]
fn whole_text_keeps_comments_and_blanks_and_refuses_what_would_be_skipped() {
    let text = "# editor\n\nRUST_BACKTRACE=vim\n\n# more\nRUST_LOG=debug\n";
    assert_eq!(resolve_lines("", text), Ok(text.to_string()));
    assert!(resolve_lines("", "RUST_BACKTRACE=vim\nnot a setting\n").unwrap_err().starts_with("line 2:"));
    assert!(resolve_lines("", "# ok\nPATH=/x\n").unwrap_err().starts_with("line 2:"));
    assert!(resolve_lines("", "TZ=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n").is_err());
    assert!(resolve_lines("", &format!("TZ={HIDDEN}\n")).is_err());
}

#[test]
fn a_whole_text_edit_keeps_the_lines_an_agent_cannot_write() {
    let current = "SERVICE_TOKEN=operator-value-123\nRUST_BACKTRACE=vim\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n";
    // Written back as shown: each goes back where it was.
    let shown = redacted(current);
    assert_eq!(resolve_lines(current, &shown), Ok(current.to_string()));
    // Moved and edited around: the operator's lines come back in the new place.
    let edited = "RUST_BACKTRACE=nano\nSERVICE_TOKEN=[hidden]\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n";
    assert_eq!(
        resolve_lines(current, edited),
        Ok("RUST_BACKTRACE=nano\nSERVICE_TOKEN=operator-value-123\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n".to_string())
    );
    // Left out: still there, at the end.
    assert_eq!(
        resolve_lines(current, "RUST_BACKTRACE=nano\n"),
        Ok("RUST_BACKTRACE=nano\nSERVICE_TOKEN=operator-value-123\nPATH=/operator/bin\nAGENTMUX_NOTE=1\n".to_string())
    );
    // A new value for one is still refused, and [hidden] for a name the operator never set is too.
    assert!(resolve_lines(current, "SERVICE_TOKEN=mine\n").is_err());
    assert!(resolve_lines(current, "PATH=/other\n").is_err());
    assert!(resolve_lines(current, "OTHER_TOKEN=[hidden]\n").is_err());
    assert!(resolve_lines(current, "RUST_BACKTRACE=[hidden]\n").is_err());
}

#[test]
fn a_secret_looking_value_on_an_ordinary_name_is_kept_too() {
    let current = "LC_ALL=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nRUST_BACKTRACE=vim\n";
    assert_eq!(redacted(current), "LC_ALL=[hidden]\nRUST_BACKTRACE=vim\n");
    assert_eq!(resolve_lines(current, "LC_ALL=[hidden]\nRUST_BACKTRACE=nano\n"), Ok("LC_ALL=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nRUST_BACKTRACE=nano\n".to_string()));
}

#[test]
fn an_edit_replaces_in_place_appends_new_and_keeps_everything_else() {
    let current = "# tools\nRUST_BACKTRACE=vim\n\nRUST_LOG=info\nDISABLE_AUTOUPDATER=1\n";
    let edited = edit(current, &map(&[("RUST_LOG", "debug"), ("DISABLE_TELEMETRY", "x y")]), &["DISABLE_AUTOUPDATER".to_string()]).unwrap();
    assert_eq!(edited, "# tools\nRUST_BACKTRACE=vim\n\nRUST_LOG=debug\nDISABLE_TELEMETRY=x y\n");
}

#[test]
fn an_edit_of_nothing_starts_the_text_and_removing_the_last_leaves_it_empty() {
    assert_eq!(edit("", &map(&[("TZ", "1")]), &[]).unwrap(), "TZ=1\n");
    assert_eq!(edit("TZ=1\n", &BTreeMap::new(), &["TZ".to_string()]).unwrap(), "");
    assert_eq!(edit("TZ=1", &map(&[("LANG", "2")]), &[]).unwrap(), "TZ=1\nLANG=2");
}

#[test]
fn an_edit_replaces_every_copy_of_a_repeated_name() {
    assert_eq!(edit("TZ=1\nTZ=2\n", &map(&[("TZ", "3")]), &[]).unwrap(), "TZ=3\nTZ=3\n");
    assert_eq!(edit("TZ=1\nLANG=2\nTZ=2\n", &BTreeMap::new(), &["TZ".to_string()]).unwrap(), "LANG=2\n");
}

#[test]
fn an_edit_cannot_touch_a_refused_name_either_way() {
    assert!(edit("", &map(&[("PATH", "/x")]), &[]).is_err());
    assert!(edit("ANTHROPIC_API_KEY=abc\n", &BTreeMap::new(), &["ANTHROPIC_API_KEY".to_string()]).is_err());
    assert!(edit("", &map(&[("TZ", "ghp_abcdefghijklmnopqrstuvwxyz0123456789")]), &[]).is_err());
}

#[test]
fn the_diff_names_what_was_added_changed_and_removed() {
    let change = diff("TZ=1\nLANG=2\nC=3\n", "TZ=1\nLANG=20\nD=4\n");
    assert_eq!(change.added, map(&[("D", "4")]));
    assert_eq!(change.changed.get("LANG"), Some(&ValueChange { from: "2".into(), to: "20".into() }));
    assert_eq!(change.removed, vec!["C".to_string()]);
    assert_eq!(change.summary(), "added [D] changed [LANG] removed [C]");
    assert!(diff("TZ=1\n# c\n", "# other\nTZ=1\n").is_empty());
}

#[test]
fn credential_looking_values_are_never_shown_to_an_agent() {
    let text = "RUST_BACKTRACE=vim\nSERVICE_TOKEN=abc123\nFORCE_COLOR=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n# kept\n";
    assert_eq!(
        redacted(text),
        "RUST_BACKTRACE=vim\nSERVICE_TOKEN=[hidden]\nFORCE_COLOR=[hidden]\n# kept\n"
    );
    let change = diff("", text);
    assert_eq!(change.added.get("RUST_BACKTRACE").map(String::as_str), Some("vim"));
    assert_eq!(change.added.get("SERVICE_TOKEN").map(String::as_str), Some(HIDDEN));
    assert_eq!(change.added.get("FORCE_COLOR").map(String::as_str), Some(HIDDEN));
}
