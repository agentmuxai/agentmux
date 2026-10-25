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
    assert_eq!(classify("  FOO=bar baz  "), Line::Pair { key: "FOO", value: "bar baz" });
    assert_eq!(classify("EMPTY="), Line::Pair { key: "EMPTY", value: "" });
    assert_eq!(classify("A=b=c"), Line::Pair { key: "A", value: "b=c" });
    // The launch skips these: no `=`, nothing before it, a space in the key, a leading digit.
    for skipped in ["FOO", "=x", "FOO = 1", "1A=b", "A-B=c"] {
        assert_eq!(classify(skipped), Line::Ignored, "{skipped}");
    }
}

#[test]
fn the_last_of_a_repeated_name_wins() {
    assert_eq!(effective("A=1\n# c\nB=2\nA=3\n"), map(&[("A", "3"), ("B", "2")]));
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
        "1BAD",
        "has space",
    ] {
        assert!(name_problem(name).is_some(), "{name} should be refused");
    }
}

#[test]
fn ordinary_settings_are_allowed() {
    for name in ["EDITOR", "RUST_LOG", "CLAUDE_CODE_MAX_OUTPUT_TOKENS", "GIT_EDITOR", "_X1"] {
        assert_eq!(name_problem(name), None, "{name}");
    }
}

#[test]
fn a_value_that_looks_like_a_secret_is_refused() {
    assert!(value_problem("NOTE", "ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_some());
    assert!(value_problem("NOTE", "sk-abcdefghijklmnopqrstuvwx").is_some());
    assert!(value_problem("NOTE", "line\rbreak").is_some());
    assert!(value_problem("NOTE", HIDDEN).is_some());
    assert_eq!(value_problem("NOTE", "debug,trace"), None);
    assert_eq!(value_problem("NOTE", "C:\\tools\\bin"), None);
    assert_eq!(value_problem("NOTE", ""), None);
}

#[test]
fn the_size_is_bounded_in_lines_and_bytes() {
    let sixty_four = "A=1\n".repeat(MAX_LINES);
    assert_eq!(check_size(&sixty_four), Ok(()));
    assert!(check_size(&format!("{sixty_four}B=2\n")).unwrap_err().contains("65 lines"));
    let big = format!("A={}\n", "x".repeat(MAX_BYTES));
    assert!(check_size(&big).unwrap_err().contains("bytes"));
    assert_eq!(check_size(&format!("A={}", "x".repeat(MAX_BYTES - 2))), Ok(()));
}

#[test]
fn whole_text_keeps_comments_and_blanks_and_refuses_what_would_be_skipped() {
    let text = "# editor\n\nEDITOR=vim\n\n# more\nRUST_LOG=debug\n";
    assert_eq!(validate_lines(text), Ok(()));
    assert!(validate_lines("EDITOR=vim\nnot a setting\n").unwrap_err().starts_with("line 2:"));
    assert!(validate_lines("# ok\nPATH=/x\n").unwrap_err().starts_with("line 2:"));
    assert!(validate_lines("A=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n").is_err());
    assert!(validate_lines(&format!("A={HIDDEN}\n")).is_err());
}

#[test]
fn an_edit_replaces_in_place_appends_new_and_keeps_everything_else() {
    let current = "# tools\nEDITOR=vim\n\nRUST_LOG=info\nOTHER=1\n";
    let edited = edit(current, &map(&[("RUST_LOG", "debug"), ("NEW", "x y")]), &["OTHER".to_string()]).unwrap();
    assert_eq!(edited, "# tools\nEDITOR=vim\n\nRUST_LOG=debug\nNEW=x y\n");
}

#[test]
fn an_edit_of_nothing_starts_the_text_and_removing_the_last_leaves_it_empty() {
    assert_eq!(edit("", &map(&[("A", "1")]), &[]).unwrap(), "A=1\n");
    assert_eq!(edit("A=1\n", &BTreeMap::new(), &["A".to_string()]).unwrap(), "");
    assert_eq!(edit("A=1", &map(&[("B", "2")]), &[]).unwrap(), "A=1\nB=2");
}

#[test]
fn an_edit_replaces_every_copy_of_a_repeated_name() {
    assert_eq!(edit("A=1\nA=2\n", &map(&[("A", "3")]), &[]).unwrap(), "A=3\nA=3\n");
    assert_eq!(edit("A=1\nB=2\nA=2\n", &BTreeMap::new(), &["A".to_string()]).unwrap(), "B=2\n");
}

#[test]
fn an_edit_cannot_touch_a_refused_name_either_way() {
    assert!(edit("", &map(&[("PATH", "/x")]), &[]).is_err());
    assert!(edit("ANTHROPIC_API_KEY=abc\n", &BTreeMap::new(), &["ANTHROPIC_API_KEY".to_string()]).is_err());
    assert!(edit("", &map(&[("A", "ghp_abcdefghijklmnopqrstuvwxyz0123456789")]), &[]).is_err());
}

#[test]
fn the_diff_names_what_was_added_changed_and_removed() {
    let change = diff("A=1\nB=2\nC=3\n", "A=1\nB=20\nD=4\n");
    assert_eq!(change.added, map(&[("D", "4")]));
    assert_eq!(change.changed.get("B"), Some(&ValueChange { from: "2".into(), to: "20".into() }));
    assert_eq!(change.removed, vec!["C".to_string()]);
    assert_eq!(change.summary(), "added [D] changed [B] removed [C]");
    assert!(diff("A=1\n# c\n", "# other\nA=1\n").is_empty());
}

#[test]
fn credential_looking_values_are_never_shown_to_an_agent() {
    let text = "EDITOR=vim\nSERVICE_TOKEN=abc123\nNOTE=ghp_abcdefghijklmnopqrstuvwxyz0123456789\n# kept\n";
    assert_eq!(
        redacted(text),
        "EDITOR=vim\nSERVICE_TOKEN=[hidden]\nNOTE=[hidden]\n# kept\n"
    );
    let change = diff("", text);
    assert_eq!(change.added.get("EDITOR").map(String::as_str), Some("vim"));
    assert_eq!(change.added.get("SERVICE_TOKEN").map(String::as_str), Some(HIDDEN));
    assert_eq!(change.added.get("NOTE").map(String::as_str), Some(HIDDEN));
}
