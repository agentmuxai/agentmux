// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn canonical_ids_resolve() {
    assert!(get_provider("claude").is_some());
    assert!(get_provider("codex").is_some());
    assert!(get_provider("gemini").is_some());
    assert!(get_provider("kimi").is_some());
    assert!(get_provider("openclaw").is_some());
    assert!(get_provider("qwen").is_some());
    assert!(get_provider("muxcode").is_some());
    assert!(get_provider("antigravity").is_some());
}

#[test]
fn antigravity_drives_the_real_agy_cli() {
    let p = get_provider("antigravity").unwrap();
    assert_eq!(p.controller_type, ControllerType::Subprocess);
    assert_eq!(p.controller_type_str(), "subprocess");
    assert_eq!(p.styled_output_format, "agy-stream-json");
    assert_eq!(p.cli_command, "agy");
    // Flags as agy 1.1.11 names them: no --yolo, no -r, no stdin prompt.
    assert_eq!(p.launch_args, &["--output-format", "stream-json", "--dangerously-skip-permissions"]);
    assert_eq!(p.session_id_field, "conversation_id");
    assert_eq!(p.resume_flag, Some("--conversation"));
    assert_eq!(p.resume_strategy_str(), "flag");
    assert_eq!(p.prompt_arg_flag(), Some("-p"));
    // Every other provider still reads its prompt from stdin.
    for id in ["claude", "codex", "gemini", "qwen", "kimi", "openclaw", "pi", "muxcode", "copilot"] {
        assert_eq!(get_provider(id).unwrap().prompt_arg_flag(), None, "{id}");
    }
    // Installed by Google's own script, not npm.
    assert!(p.npm_package.is_empty());
    assert!(p.pinned_version.is_empty());
    assert_eq!(p.supported_vendors, &["google"]);
    assert!(p.base_url_env_var.is_none());
}

#[test]
fn qwen_streams_claude_shaped_frames_under_its_own_format() {
    // Qwen Code is a Gemini-CLI fork, but its stream-json is Claude Code's
    // frame shape (verified on 0.24.0), and its reply text arrives only as
    // partial-message deltas.
    let p = get_provider("qwen").unwrap();
    assert_eq!(p.styled_output_format, "qwen-stream-json");
    assert!(p.launch_args.contains(&"--include-partial-messages"));
    assert!(crate::backend::blockcontroller::subprocess::persists_user_record(p.styled_output_format));
}

#[test]
fn kimi_takes_its_prompt_from_stdin_with_no_empty_prompt_flag() {
    // kimi 1.37 exits "Invalid value for --prompt: Prompt cannot be empty"
    // on `-p ""`, before reading stdin; `--print` alone reads stdin.
    let p = get_provider("kimi").unwrap();
    assert!(p.launch_args.contains(&"--print"));
    assert!(!p.launch_args.contains(&"-p"), "{:?}", p.launch_args);
    assert!(!p.launch_args.contains(&""), "{:?}", p.launch_args);
}

#[test]
fn mux_code_is_subprocess_with_claude_stream_json() {
    let p = get_provider("muxcode").unwrap();
    assert_eq!(p.controller_type, ControllerType::Subprocess);
    assert_eq!(p.controller_type_str(), "subprocess");
    assert_eq!(p.styled_output_format, "claude-stream-json");
    assert_eq!(p.cli_command, "muxcode");
    assert_eq!(p.session_id_field, "session_id");
    assert_eq!(p.resume_flag, Some("--resume"));
    assert_eq!(p.npm_package, "@agentmuxai/muxcode");
    assert_eq!(p.launch_args, &["run", "-p"]);
    assert!(p.persistent_launch_args.is_none());
}

/// The installer's default folder, so a fresh `agy` install is found
/// before AgentMux restarts and picks up the new PATH.
#[test]
fn antigravity_is_found_where_its_installer_puts_it() {
    let paths = super::antigravity_install_paths(
        Some(std::path::PathBuf::from("LAD")),
        Some(std::path::PathBuf::from("HOME")),
    );
    let expected = if cfg!(windows) {
        std::path::Path::new("LAD").join("agy").join("bin").join("agy.exe")
    } else {
        std::path::Path::new("HOME").join(".local").join("bin").join("agy")
    };
    assert_eq!(paths, vec![expected]);
    assert!(super::antigravity_install_paths(None, None).is_empty());
    // Every other provider either installs through npm or relies on PATH.
    assert!(known_install_paths("kimi").is_empty());
    let home = std::path::PathBuf::from("/h");
    let (goose, grok) = (super::goose_install_paths(Some(home.clone())), super::grok_install_paths(Some(home.clone())));
    assert!(goose.contains(&home.join(".local").join("bin").join(if cfg!(windows) { "goose.exe" } else { "goose" })));
    assert_eq!(grok, vec![home.join(".grok").join("bin").join(if cfg!(windows) { "grok.exe" } else { "grok" })]);
    assert!(super::goose_install_paths(None).is_empty() && super::grok_install_paths(None).is_empty());
    assert_eq!(
        super::aider_install_paths(Some(home.clone())),
        vec![home.join(".local").join("bin").join(if cfg!(windows) { "aider.exe" } else { "aider" })]
    );
    assert!(known_install_paths("claude").is_empty());
}

#[test]
fn pi_acp_is_pointed_at_the_pi_installed_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let pi = dir.path().join(if cfg!(windows) { "pi.cmd" } else { "pi" });
    let acp = dir.path().join(if cfg!(windows) { "pi-acp.cmd" } else { "pi-acp" });
    std::fs::write(&acp, "").unwrap();
    assert_eq!(pi_beside_pi_acp(&acp.to_string_lossy()), None, "no pi yet");
    std::fs::write(&pi, "").unwrap();
    assert_eq!(pi_beside_pi_acp(&acp.to_string_lossy()), Some(pi.to_string_lossy().into_owned()));
    // Other ACP CLIs get nothing.
    let copilot = dir.path().join("copilot.cmd");
    assert_eq!(pi_beside_pi_acp(&copilot.to_string_lossy()), None);
}

#[test]
fn every_provider_declares_at_least_one_supported_vendor() {
    for id in ["claude", "codex", "gemini", "qwen", "kimi", "openclaw", "pi", "muxcode", "copilot", "antigravity"] {
        let p = get_provider(id).unwrap_or_else(|| panic!("provider '{id}' not registered"));
        assert!(
            !p.supported_vendors.is_empty(),
            "provider '{id}' declares no supported_vendors"
        );
    }
}

#[test]
fn claude_default_vendor_is_anthropic() {
    assert_eq!(get_provider("claude").unwrap().supported_vendors, &["anthropic"]);
}

#[test]
fn aliases_resolve() {
    assert_eq!(get_provider("claude-code").unwrap().id, "claude");
    assert_eq!(get_provider("claude_code").unwrap().id, "claude");
    assert_eq!(get_provider("codex-cli").unwrap().id, "codex");
    assert_eq!(get_provider("gemini-cli").unwrap().id, "gemini");
    assert_eq!(get_provider("qwen-code").unwrap().id, "qwen");
    assert_eq!(get_provider("qwen3-coder").unwrap().id, "qwen");
    assert_eq!(get_provider("kimi-cli").unwrap().id, "kimi");
    assert_eq!(get_provider("openclaw-cli").unwrap().id, "openclaw");
    assert_eq!(get_provider("agy").unwrap().id, "antigravity");
    assert_eq!(get_provider("antigravity-cli").unwrap().id, "antigravity");
    assert_eq!(get_provider("antigravity_cli").unwrap().id, "antigravity");
}

#[test]
fn unknown_returns_none() {
    assert!(get_provider("unknown-provider").is_none());
}

#[test]
fn all_providers_covers_every_registered_id_exactly_once() {
    let ids: Vec<&str> = all_providers().map(|p| p.id).collect();
    for expected in [
        "claude",
        "codex",
        "gemini",
        "qwen",
        "kimi",
        "openclaw",
        "pi",
        "copilot",
        "muxcode",
        "antigravity",
        "kilo",
        "cline",
        "opencode",
        "goose",
        "grok",
        "dsh",
        "aider",
        "crush",
        "muse",
    ] {
        assert_eq!(
            ids.iter().filter(|id| **id == expected).count(),
            1,
            "expected exactly one entry for {expected}, got {ids:?}"
        );
    }
    assert_eq!(ids.len(), 19, "must not silently gain/lose a provider: {ids:?}");
}

#[test]
fn all_providers_includes_every_provider_with_a_linkable_history_dir() {
    let with_history: Vec<&str> = all_providers()
        .filter(|p| p.history_native_subdir.is_some())
        .map(|p| p.id)
        .collect();
    assert_eq!(
        with_history.len(),
        3,
        "expected exactly claude/codex/gemini to have a linkable history dir: {with_history:?}"
    );
    for expected in ["claude", "codex", "gemini"] {
        assert!(with_history.contains(&expected), "missing {expected}: {with_history:?}");
    }
}

#[test]
fn claude_persistent_with_persistent_args_present() {
    let p = get_provider("claude").unwrap();
    // Claude runs on the persistent controller so AskUserQuestion can block
    // on a tool_use and consume a tool_result over live stdin (see the
    // controller_type comment on `static CLAUDE`).
    assert!(p.persistent_launch_args.is_some());
    assert_eq!(p.controller_type, ControllerType::Persistent);
    assert_eq!(p.controller_type_str(), "persistent");
}

#[test]
fn codex_uses_exec_resume_strategy() {
    let p = get_provider("codex").unwrap();
    assert!(p.resume_flag.is_none());
    assert_eq!(p.controller_type, ControllerType::Subprocess);
    assert_eq!(p.resume_strategy_str(), "codex-exec");
}

#[test]
fn codex_advertises_app_server_without_enabling_it() {
    let p = get_provider("codex").unwrap();
    let app_server = p.app_server.expect("Codex App Server capability");
    assert_eq!(p.controller_type, ControllerType::Subprocess);
    assert_eq!(app_server.kind, AppServerKind::Codex);
    assert_eq!(
        app_server.launch_args,
        &["app-server", "--listen", "stdio://"]
    );
    assert_eq!(
        app_server.fallback_controller,
        ControllerType::Subprocess
    );
    assert_eq!(app_server.fallback_output_format, "codex-json");
    assert_eq!(p.auth_config_dir_env_var, "CODEX_HOME");
}

#[test]
fn gemini_auth_extra_env() {
    let p = get_provider("gemini").unwrap();
    assert!(p
        .auth_extra_env
        .iter()
        .any(|(k, v)| *k == "GEMINI_FORCE_FILE_STORAGE" && *v == "true"));
}

#[test]
fn kimi_is_subprocess_controller() {
    let p = get_provider("kimi").unwrap();
    assert_eq!(p.controller_type, ControllerType::Subprocess);
    assert_eq!(p.controller_type_str(), "subprocess");
    assert_eq!(p.styled_output_format, "kimi-stream-json");
    assert_eq!(p.cli_command, "kimi");
    assert!(p.npm_package.is_empty());
}

#[test]
fn openclaw_is_acp_controller() {
    let p = get_provider("openclaw").unwrap();
    assert_eq!(p.controller_type, ControllerType::Acp);
    assert_eq!(p.controller_type_str(), "acp");
    assert_eq!(p.styled_output_format, "acp");
    assert!(p.resume_flag.is_none());
}

#[test]
fn pi_is_acp_controller() {
    let p = get_provider("pi").unwrap();
    assert_eq!(p.controller_type, ControllerType::Acp);
    assert_eq!(p.controller_type_str(), "acp");
    assert_eq!(p.styled_output_format, "acp");
    // pi speaks no ACP itself (`pi --json` was never an option); the
    // pi-acp adapter does, and runs the pi package installed beside it.
    assert_eq!(p.cli_command, "pi-acp");
    assert!(p.launch_args.is_empty());
    assert_eq!(p.npm_package, "pi-acp");
    assert_eq!(
        p.npm_install_specs(p.pinned_version),
        vec!["pi-acp@0.0.34".to_string(), "@earendil-works/pi-coding-agent@1.0.4".to_string()]
    );
    // Everyone else installs just the one package.
    let claude = get_provider("claude").unwrap();
    assert_eq!(claude.npm_install_specs("2.1.288"), vec!["@anthropic-ai/claude-code@2.1.288".to_string()]);
    assert!(claude.companion_npm_packages().is_empty());
}

// Harness vs. model vendor decoupling: only claude has a verified,
// documented way to redirect its backend (ANTHROPIC_BASE_URL). Every
// other provider is explicit None rather than a guessed env var name.
#[test]
fn claude_declares_base_url_env_var() {
    let p = get_provider("claude").unwrap();
    assert_eq!(p.base_url_env_var, Some("ANTHROPIC_BASE_URL"));
}

#[test]
fn non_claude_providers_have_no_base_url_env_var() {
    for id in ["codex", "gemini", "qwen", "kimi", "openclaw", "pi", "copilot", "muxcode", "antigravity"] {
        let p = get_provider(id).unwrap();
        assert!(
            p.base_url_env_var.is_none(),
            "provider '{id}' should not declare base_url_env_var yet (unverified)"
        );
    }
}

// docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2's
// per-provider table, pinned so a future edit can't silently drift from
// the researched/cited values.
#[test]
fn startup_instructions_filename_matches_researched_table() {
    let expected: &[(&str, Option<&str>)] = &[
        ("claude", Some("CLAUDE.md")),
        ("codex", Some("AGENTS.md")),
        ("gemini", Some("GEMINI.md")),
        ("qwen", Some("QWEN.md")),
        ("copilot", Some("AGENTS.md")),
        ("openclaw", Some("AGENTS.md")),
        ("pi", Some("AGENTS.md")),
        ("antigravity", Some("GEMINI.md")),
        ("muxcode", Some("CLAUDE.md")),
        ("kimi", None),
    ];
    for (id, filename) in expected {
        let p = get_provider(id).unwrap_or_else(|| panic!("provider '{id}' not registered"));
        assert_eq!(
            p.startup_instructions_filename, *filename,
            "provider '{id}' startup_instructions_filename mismatch"
        );
    }
}

#[test]
fn kimi_has_no_startup_instructions_filename() {
    let p = get_provider("kimi").unwrap();
    assert!(
        p.startup_instructions_filename.is_none(),
        "kimi has no confirmed native startup-instructions file — see SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2"
    );
}

// docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md.
// `paths_resolve_to_same_dir` takes the reference side pre-resolved
// (not `get_home_dir()` itself) specifically so these tests don't
// depend on the real $HOME/%USERPROFILE% — see its own doc comment.
mod ambient_home_dir_tests {
    use super::*;

    #[test]
    fn identical_existing_dirs_match_via_canonicalize() {
        let dir = tempfile::tempdir().unwrap();
        assert!(paths_resolve_to_same_dir(dir.path(), &dir.path().to_string_lossy()));
    }

    #[test]
    fn different_existing_dirs_do_not_match() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert!(!paths_resolve_to_same_dir(a.path(), &b.path().to_string_lossy()));
    }

    #[test]
    fn nonexistent_paths_fall_back_to_lexical_comparison() {
        // Neither side exists on disk — canonicalize fails for both,
        // so this exercises the lexical fallback, not the
        // canonicalize branch. Confirms a not-yet-created ambient
        // path still gets caught (the whole point of the guard —
        // §7's "can't slip past by not existing at check time").
        let reference = std::path::Path::new("/tmp/agentmux-test-does-not-exist-ambient/.claude");
        assert!(paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-does-not-exist-ambient/.claude"));
        assert!(!paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-does-not-exist-ambient/.codex"));
    }

    #[test]
    fn lexical_fallback_ignores_trailing_slash() {
        let reference = std::path::Path::new("/tmp/agentmux-test-nonexistent/.claude");
        assert!(paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-nonexistent/.claude/"));
    }

    // codex P2, PR #2802: case-folding must match the current platform's
    // default filesystem case-sensitivity, not be unconditional —
    // asserts whichever behavior is CORRECT for whatever OS actually
    // runs this test, so it's meaningful on both windows-latest and
    // ubuntu-latest CI legs.
    #[test]
    fn lexical_fallback_case_folding_matches_platform_default() {
        let reference = std::path::Path::new("/tmp/agentmux-test-nonexistent/.claude");
        let matches = paths_resolve_to_same_dir(reference, "/TMP/agentmux-test-nonexistent/.CLAUDE");
        if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
            assert!(matches, "case-insensitive platforms must fold case in the lexical fallback");
        } else {
            assert!(!matches, "case-sensitive platforms must NOT fold case — a not-yet-created dir must not be conflated with a differently-cased one");
        }
    }

    // codex P1, PR #2802: without dot-segment collapsing, a binding
    // like `$HOME/.claude/.` compared unequal to `$HOME/.claude` under
    // plain string comparison whenever neither existed yet (the fresh-
    // machine case), bypassing the guard — Claude Code would then
    // create and use the literal ambient dir once launched.
    #[test]
    fn lexical_fallback_collapses_dot_and_dotdot_segments() {
        let reference = std::path::Path::new("/tmp/agentmux-test-nonexistent/.claude");
        assert!(paths_resolve_to_same_dir(reference, "/tmp/agentmux-test-nonexistent/.claude/."));
        assert!(paths_resolve_to_same_dir(
            reference,
            "/tmp/agentmux-test-nonexistent/sibling/../.claude",
        ));
        // A genuinely different directory reached via `..` must still
        // correctly NOT match — this isn't "ignore everything after a
        // dot-segment," it's real lexical normalization.
        assert!(!paths_resolve_to_same_dir(
            reference,
            "/tmp/agentmux-test-nonexistent/.claude/../.codex",
        ));
    }

    #[test]
    fn is_provider_ambient_home_dir_matches_the_real_configured_provider_home_suffix() {
        // Not asserting against the real $HOME (see module doc comment) —
        // just confirming the public wrapper joins get_home_dir() with
        // ".{auth_dir_name}" as documented, by checking the suffix
        // shape rather than an exact path.
        let claude = get_provider("claude").unwrap();
        assert_eq!(claude.auth_dir_name, "claude");
        // A dir that can't possibly be the real ambient home (this
        // process's actual $HOME/.claude) must never match.
        assert!(!is_provider_ambient_home_dir(claude, "/tmp/agentmux-test-definitely-not-ambient"));
    }
}

mod transcript_retention_tests {
    use super::*;

    fn seed(dir: &std::path::Path) -> bool {
        let claude = get_provider("claude").unwrap();
        seed_transcript_retention_if_missing(claude, &dir.to_string_lossy()).unwrap()
    }

    fn settings(dir: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap()
    }

    #[test]
    fn creates_settings_with_the_retention_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(seed(dir.path()));
        assert_eq!(
            settings(dir.path())["cleanupPeriodDays"],
            serde_json::json!(CLAUDE_TRANSCRIPT_RETENTION_DAYS)
        );
    }

    #[test]
    fn adds_the_retention_and_keeps_existing_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{"model":"opus","env":{"A":"1"}}"#).unwrap();
        assert!(seed(dir.path()));
        let s = settings(dir.path());
        assert_eq!(s["cleanupPeriodDays"], serde_json::json!(CLAUDE_TRANSCRIPT_RETENTION_DAYS));
        assert_eq!(s["model"], "opus");
        assert_eq!(s["env"]["A"], "1");
    }

    /// An operator who set their own retention keeps it, shorter or longer.
    #[test]
    fn an_explicit_retention_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{"cleanupPeriodDays":7}"#).unwrap();
        assert!(!seed(dir.path()));
        assert_eq!(settings(dir.path())["cleanupPeriodDays"], 7);
    }

    /// Rewriting a file we can't parse could destroy the operator's settings.
    #[test]
    fn an_unparseable_or_non_object_settings_file_is_never_rewritten() {
        for original in ["{ not json", "[1, 2]", ""] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("settings.json"), original).unwrap();
            assert!(!seed(dir.path()), "{original:?}");
            assert_eq!(std::fs::read_to_string(dir.path().join("settings.json")).unwrap(), original);
        }
    }

    #[test]
    fn is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(seed(dir.path()));
        let first = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!seed(dir.path()));
        assert_eq!(std::fs::read_to_string(dir.path().join("settings.json")).unwrap(), first);
    }

    #[test]
    fn other_providers_are_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let codex = get_provider("codex").unwrap();
        assert!(!seed_transcript_retention_if_missing(codex, &dir.path().to_string_lossy()).unwrap());
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn a_blank_config_dir_is_rejected() {
        let claude = get_provider("claude").unwrap();
        assert!(seed_transcript_retention_if_missing(claude, "  ").is_err());
    }
}

mod claude_md_placeholder_tests {
    use super::*;

    #[test]
    fn writes_the_placeholder_when_claude_md_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let claude = get_provider("claude").unwrap();
        let wrote = seed_claude_md_placeholder_if_missing(claude, &dir.path().to_string_lossy()).unwrap();
        assert!(wrote);
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("AgentMux: intentionally empty"));
    }

    #[test]
    fn creates_the_config_dir_first_if_it_does_not_exist_yet() {
        let base = tempfile::tempdir().unwrap();
        let not_yet_created = base.path().join("nested").join("claude");
        let claude = get_provider("claude").unwrap();
        let wrote =
            seed_claude_md_placeholder_if_missing(claude, &not_yet_created.to_string_lossy()).unwrap();
        assert!(wrote);
        assert!(not_yet_created.join("CLAUDE.md").exists());
    }

    #[test]
    fn never_overwrites_an_existing_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "real user content, do not touch").unwrap();
        let claude = get_provider("claude").unwrap();
        let wrote = seed_claude_md_placeholder_if_missing(claude, &dir.path().to_string_lossy()).unwrap();
        assert!(!wrote);
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert_eq!(content, "real user content, do not touch");
    }

    #[test]
    fn no_ops_for_a_non_claude_provider_even_with_no_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        let codex = get_provider("codex").unwrap();
        assert_eq!(codex.auth_dir_name, "codex");
        let wrote = seed_claude_md_placeholder_if_missing(codex, &dir.path().to_string_lossy()).unwrap();
        assert!(!wrote);
        assert!(!dir.path().join("CLAUDE.md").exists());
    }

    // Codex P2 on PR #2854: an empty config_dir would otherwise resolve
    // relative to the server process's own CWD instead of erroring.
    #[test]
    fn rejects_an_empty_config_dir() {
        let claude = get_provider("claude").unwrap();
        let err = seed_claude_md_placeholder_if_missing(claude, "").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!std::path::Path::new("CLAUDE.md").exists(), "must not write into CWD");
    }

    #[test]
    fn rejects_a_whitespace_only_config_dir() {
        let claude = get_provider("claude").unwrap();
        let err = seed_claude_md_placeholder_if_missing(claude, "   ").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}

mod prepare_provider_auth_dir_tests {
    use super::*;

    // The default spawn path (agent_open.rs) previously did
    // `create_dir_all` and the isolation seed as two separate,
    // independently-deletable statements, with no test covering the
    // seed call at all — removing that one line reopened the leak
    // silently. Fusing them means a caller cannot get a usable auth
    // dir without its isolation guarantees.
    #[test]
    fn creates_the_dir_and_seeds_the_claude_placeholder_together() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("not").join("yet").join("there");
        let claude = get_provider("claude").unwrap();
        prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
        assert!(dir.is_dir(), "auth dir must be created");
        let content = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(content.contains("AgentMux: intentionally empty"));
    }

    #[test]
    fn a_new_claude_dir_gets_the_transcript_retention() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("fresh");
        let claude = get_provider("claude").unwrap();
        prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
        let settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["cleanupPeriodDays"], serde_json::json!(CLAUDE_TRANSCRIPT_RETENTION_DAYS));
    }

    #[test]
    fn creates_the_dir_without_a_claude_md_for_a_non_claude_provider() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("codexhome");
        let codex = get_provider("codex").unwrap();
        prepare_provider_auth_dir(codex, &dir.to_string_lossy()).unwrap();
        assert!(dir.is_dir(), "auth dir must still be created");
        assert!(!dir.join("CLAUDE.md").exists(), "must not seed a foreign provider's dir");
    }

    #[test]
    fn is_idempotent_and_never_clobbers_existing_content() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("iso");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "real user content").unwrap();
        let claude = get_provider("claude").unwrap();
        prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
        prepare_provider_auth_dir(claude, &dir.to_string_lossy()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
            "real user content",
        );
    }

    #[test]
    fn rejects_an_empty_auth_dir_instead_of_touching_the_cwd() {
        let claude = get_provider("claude").unwrap();
        let err = prepare_provider_auth_dir(claude, "   ").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[test]
fn terminal_tier_harnesses_run_on_the_cmd_controller() {
    for id in ["aider", "crush", "muse"] {
        let p = get_provider(id).unwrap();
        assert_eq!(p.controller_type, ControllerType::Terminal, "{id}");
        assert_eq!(p.controller_type_str(), "cmd", "{id}");
    }
}

#[test]
fn muse_resolves_to_the_binary_its_launcher_installed() {
    let dir = tempfile::tempdir().unwrap();
    let home = std::path::PathBuf::from("/h");
    if cfg!(windows) {
        // No .muse-version yet: nothing to run.
        assert!(super::muse_install_paths(Some(dir.path().to_path_buf()), None).is_empty());
        std::fs::write(dir.path().join(".muse-version"), "1.4.7-R6101.1\n").unwrap();
        assert_eq!(
            super::muse_install_paths(Some(dir.path().to_path_buf()), None),
            vec![dir.path().join("muse-bin-1.4.7-R6101.1.exe")]
        );
        // A version that would leave the directory is refused.
        std::fs::write(dir.path().join(".muse-version"), "../x").unwrap();
        assert!(super::muse_install_paths(Some(dir.path().to_path_buf()), None).is_empty());
    } else {
        assert_eq!(super::muse_install_paths(None, Some(home.clone())), vec![home.join(".local").join("bin").join("muse")]);
    }
}

#[test]
fn a_terminal_tier_pane_is_a_terminal_started_at_once() {
    let mut meta = crate::backend::obj::MetaMapType::new();
    apply_terminal_pane_meta(get_provider("aider").unwrap(), &mut meta);
    assert_eq!(meta["view"], "term");
    assert_eq!(meta["cmd:interactive"], true);
    assert_eq!(meta["cmd:runonstart"], true);
    let mut other = crate::backend::obj::MetaMapType::new();
    apply_terminal_pane_meta(get_provider("kilo").unwrap(), &mut other);
    assert!(other.is_empty());
}
