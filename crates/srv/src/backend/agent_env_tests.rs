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
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
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
fn a_commented_out_or_mistyped_credential_is_hidden_and_kept() {
    let current = "# note about the build\n# GH_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nPGPASSWORD = hunter2\nTZ=UTC\n";
    let shown = redacted(current);
    assert_eq!(shown, "# note about the build\n# [hidden]\n# [hidden]\nTZ=UTC\n");
    assert!(!shown.contains("ghp_") && !shown.contains("hunter2"));
    // Written back as shown, the operator's lines return; the edit around them stands.
    let edited = shown.replace("TZ=UTC", "TZ=Europe/Berlin");
    assert_eq!(
        resolve_lines(current, &edited),
        Ok("# note about the build\n# GH_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nPGPASSWORD = hunter2\nTZ=Europe/Berlin\n".to_string())
    );
    // Dropped: kept at the end.
    let dropped = resolve_lines(current, "TZ=UTC\n").unwrap();
    assert!(dropped.contains("GH_TOKEN=ghp_") && dropped.contains("PGPASSWORD = hunter2"));
}

#[test]
fn a_value_of_its_own_for_a_hidden_allowed_name_replaces_the_operators() {
    let current = "LC_ALL=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nTZ=UTC\n";
    assert_eq!(redacted(current), "LC_ALL=[hidden]\nTZ=UTC\n");
    assert_eq!(resolve_lines(current, "LC_ALL=C\nTZ=UTC\n"), Ok("LC_ALL=C\nTZ=UTC\n".to_string()));
    assert_eq!(effective(&resolve_lines(current, "TZ=UTC\nLC_ALL=C\n").unwrap())["LC_ALL"], "C");
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
    assert!(edit("OPENAI_API_KEY=abc\n", &BTreeMap::new(), &["OPENAI_API_KEY".to_string()]).is_err());
    assert!(edit("", &map(&[("TZ", "ghp_abcdefghijklmnopqrstuvwxyz0123456789")]), &[]).is_err());
}

#[test]
fn the_diff_names_what_was_added_changed_and_removed() {
    let change = diff("TZ=1\nLANG=2\nNO_COLOR=3\n", "TZ=1\nLANG=20\nFORCE_COLOR=4\n");
    assert_eq!(change.added, map(&[("FORCE_COLOR", "4")]));
    assert_eq!(change.changed.get("LANG"), Some(&ValueChange { from: "2".into(), to: "20".into() }));
    assert_eq!(change.removed, vec!["NO_COLOR".to_string()]);
    assert_eq!(change.summary(), "added [FORCE_COLOR] changed [LANG] removed [NO_COLOR]");
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


// ── the model endpoint of a Claude Code agent ──────────────────────────────────

#[test]
fn the_endpoint_names_are_allowed_and_other_harnesses_endpoints_are_not() {
    for name in ENDPOINT_NAMES {
        assert_eq!(name_problem(name), None, "{name}");
        assert_eq!(name_problem(&name.to_ascii_lowercase()), None, "{name}");
    }
    for name in ["ANTHROPIC_MODEL", "ANTHROPIC_SMALL_FAST_MODEL"] {
        assert_eq!(name_problem(name), None, "{name}");
    }
    // Not verified for any other harness, and the reserved patterns still catch the rest.
    for name in ["OPENAI_BASE_URL", "OPENAI_API_BASE", "GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "ANTHROPIC_FOUNDRY_BASE_URL", "ANTHROPIC_BEDROCK_BASE_URL", "AWS_BEARER_TOKEN_BEDROCK"] {
        assert!(name_problem(name).is_some(), "{name} should be refused");
    }
}

#[test]
fn a_server_address_is_plain_http_or_https_with_no_sign_in_query_or_fragment() {
    for ok in ["http://192.0.2.68:11434", "http://localhost:11434", "https://llm.example.com/v1", "http://[::1]:8080", "https://proxy.example.com:8443/anthropic"] {
        assert_eq!(value_problem("ANTHROPIC_BASE_URL", ok), None, "{ok}");
    }
    for bad in [
        "",
        "192.0.2.68:11434",
        "ftp://x.example.com",
        "file:///etc/passwd",
        "http://user:pw@example.com",
        "https://token@example.com/v1",
        "http://example.com/v1?key=abc",
        "http://example.com/#frag",
        "http://",
        "http:///path",
        "http://exa mple.com",
        "http://example.com\nX=1",
    ] {
        assert!(value_problem("ANTHROPIC_BASE_URL", bad).is_some(), "{bad:?} should be refused");
    }
}

#[test]
fn a_key_is_a_vault_reference_or_a_short_placeholder_never_a_secret() {
    for name in ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"] {
        for ok in ["ollama", "none", "local-llm_1", "vault:2f9c1d", "vault: 2f9c1d"] {
            assert_eq!(value_problem(name, ok), None, "{name}={ok}");
        }
        for bad in [
            "",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "a-long-looking-secret-value-123456",
            "has space",
            "vault:",
            "vault:   ",
            "[hidden]",
        ] {
            assert!(value_problem(name, bad).is_some(), "{name}={bad:?} should be refused");
        }
    }
    assert_eq!(vault_ref("vault:abc"), Some("abc"));
    assert_eq!(vault_ref("vault: abc "), Some("abc"));
    assert_eq!(vault_ref("vault:"), None);
    assert_eq!(vault_ref("ollama"), None);
}

#[test]
fn a_model_name_and_a_timeout_are_checked_too() {
    assert_eq!(value_problem("ANTHROPIC_DEFAULT_SONNET_MODEL", "flint:40b-a2b"), None);
    assert_eq!(value_problem("ANTHROPIC_DEFAULT_HAIKU_MODEL", "qwen3-coder:30b[1m]"), None);
    assert_eq!(value_problem("API_TIMEOUT_MS", "600000"), None);
    assert!(value_problem("ANTHROPIC_DEFAULT_OPUS_MODEL", "two words").is_some());
    assert!(value_problem("ANTHROPIC_DEFAULT_OPUS_MODEL", "").is_some());
    assert!(value_problem("API_TIMEOUT_MS", "10 minutes").is_some());
    assert!(value_problem("API_TIMEOUT_MS", "-1").is_some());
}

#[test]
fn only_api_anthropic_com_over_https_is_anthropic() {
    assert!(is_anthropic_url("https://api.anthropic.com"));
    assert!(is_anthropic_url("https://api.anthropic.com/v1"));
    assert!(is_anthropic_url(" https://API.Anthropic.com:443/ "));
    for not in ["http://api.anthropic.com", "https://api.anthropic.com.evil.example", "https://example.com/api.anthropic.com", "http://192.0.2.68:11434", "https://anthropic.com", ""] {
        assert!(!is_anthropic_url(not), "{not:?}");
    }
}

#[test]
fn a_key_is_never_shown_to_an_agent_but_the_address_and_models_are() {
    let text = "ANTHROPIC_BASE_URL=http://192.0.2.68:11434\nANTHROPIC_AUTH_TOKEN=ollama\nANTHROPIC_MODEL=flint:40b-a2b\n";
    let shown = redacted(text);
    assert!(shown.contains("ANTHROPIC_BASE_URL=http://192.0.2.68:11434"), "{shown}");
    assert!(shown.contains("ANTHROPIC_MODEL=flint:40b-a2b"), "{shown}");
    assert!(shown.contains("ANTHROPIC_AUTH_TOKEN=[hidden]"), "{shown}");
    assert!(!shown.contains("ollama"), "{shown}");
    let change = diff("", text);
    assert_eq!(change.added["ANTHROPIC_AUTH_TOKEN"], HIDDEN);
    assert_eq!(change.added["ANTHROPIC_BASE_URL"], "http://192.0.2.68:11434");
}

#[test]
fn an_edit_sets_the_endpoint_and_keeps_the_operators_other_lines() {
    let current = "# mine\nPATH_NOTE=1\nOPENAI_API_KEY=operator-secret\n";
    let set = map(&[("ANTHROPIC_BASE_URL", "http://192.0.2.68:11434"), ("ANTHROPIC_AUTH_TOKEN", "ollama"), ("ANTHROPIC_MODEL", "flint:40b-a2b")]);
    let text = edit(current, &set, &[]).unwrap();
    assert!(text.starts_with(current), "{text}");
    assert!(text.contains("ANTHROPIC_BASE_URL=http://192.0.2.68:11434\n"), "{text}");
    assert_eq!(edit(current, &map(&[("ANTHROPIC_BASE_URL", "http://u:p@host")]), &[]).is_err(), true);
}

#[test]
fn names_in_the_list_are_found_in_a_change() {
    let change = diff("TZ=UTC\n", "TZ=UTC\nANTHROPIC_BASE_URL=http://localhost:11434\n");
    assert!(change.touches_endpoint());
    assert!(!diff("", "TZ=UTC\n").touches_endpoint());
    assert!(diff("ANTHROPIC_MODEL=a\n", "ANTHROPIC_MODEL=b\n").touches_endpoint());
    assert!(diff("ANTHROPIC_API_KEY=ollama\n", "").touches_endpoint());
}
