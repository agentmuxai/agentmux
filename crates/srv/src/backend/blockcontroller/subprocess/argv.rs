// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Provider-aware construction of per-turn subprocess arguments.

/// Build the argv for one provider turn, adding session continuation when a
/// prior session id is available.
///
/// Existing providers use a trailing flag (`--resume <id>` / `-r <id>`).
/// Codex is different: its continuation command is
/// `codex exec resume [OPTIONS] <id> -`, so `resume` must be inserted after
/// `exec` and the session id must remain before the stdin prompt marker.
pub(super) fn build_turn_argv(
    base: &[String],
    resume_strategy: &str,
    resume_flag: &str,
    session_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let session_id = session_id.filter(|sid| !sid.trim().is_empty());

    match resume_strategy {
        "codex-exec" => build_codex_argv(base, session_id),
        _ if session_id.is_none() => Ok(base.to_vec()),
        "" => build_legacy_argv(base, resume_flag, session_id.unwrap()),
        "none" => Ok(base.to_vec()),
        "flag" => {
            if resume_flag.is_empty() {
                return Err("resume strategy 'flag' requires a non-empty resume flag".to_string());
            }
            let mut argv = base.to_vec();
            argv.push(resume_flag.to_string());
            argv.push(session_id.unwrap().to_string());
            Ok(argv)
        }
        other => Err(format!("unsupported resume strategy '{other}'")),
    }
}

fn build_legacy_argv(
    base: &[String],
    resume_flag: &str,
    session_id: &str,
) -> Result<Vec<String>, String> {
    if resume_flag.is_empty() {
        return Ok(base.to_vec());
    }
    let mut argv = base.to_vec();
    argv.push(resume_flag.to_string());
    argv.push(session_id.to_string());
    Ok(argv)
}

/// Build the argv for a `/btw` side-question one-shot turn
/// (`server/agent_handlers/side_question.rs`) from `base_one_shot_args` —
/// the same one-shot `cli_args` a normal turn on this block would use.
///
/// Deliberately does NOT touch resume/session-id handling: the caller
/// always passes `SubprocessSpawnConfig::session_id: None` for this turn
/// (a `/btw` never resumes a session — context comes from a text prefix in
/// the prompt instead, per the module's design), so `build_turn_argv`'s own
/// `session_id.is_none()` branch already returns `base` unchanged with no
/// `--resume` appended — nothing here needs to duplicate that guarantee.
///
/// Appends two flags, confirmed against
/// `code.claude.com/docs/en/cli-reference` (checked 2026-09-19):
///   - `--disallowedTools "*"` — the docs' own words are "a bare tool name
///     removes the matching tools... `\"*\"` removes every tool", making
///     this turn fully tool-less. The doc's other caveat — "a rule naming
///     `EndConversation` can't remove it while any other tool remains" — is
///     about a rule that NAMES `EndConversation` specifically; a wildcard
///     that merely happens to cover it too is a different case and isn't
///     described as failing.
///   - `--max-turns 1` — print mode's own hard cap on agentic turns, a
///     backstop against looping that holds even if the no-tools flag were
///     somehow bypassed.
///
/// `--output-format stream-json` is NOT added here — every subprocess-mode
/// provider's base one-shot `launch_args` already carries it (see
/// `providers.rs`), so `base_one_shot_args` already has it.
pub(crate) fn build_side_question_argv(base_one_shot_args: &[String]) -> Vec<String> {
    let mut argv = base_one_shot_args.to_vec();
    argv.push("--disallowedTools".to_string());
    argv.push("*".to_string());
    argv.push("--max-turns".to_string());
    argv.push("1".to_string());
    argv
}

/// Codex features that each add a tool to the model's request. `/btw`
/// disables every one of them. Checked against the pinned CLI (0.154.0,
/// `codex features list`) by capturing the request it sends: with these
/// off, plus `--ignore-user-config` and `web_search="disabled"`, the only
/// tool left is `request_user_input`, which `exec` mode refuses to run
/// ("request_user_input is not supported in exec mode").
const CODEX_TOOL_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "view_image",
    "multi_agent",
    "goals",
    "apps",
    "plugins",
    "browser_use",
    "computer_use",
    "image_generation",
    "sleep_tool",
    "tool_suggest",
];

/// Build the argv for a `/btw` side question on a Codex pane: a tool-less
/// `codex exec` turn, built from scratch rather than from the pane's own
/// argv, which may be the app-server's and always carries
/// `--dangerously-bypass-approvals-and-sandbox`.
///
/// - `--ignore-user-config`: `$CODEX_HOME/config.toml` is where MCP servers
///   (and so the agentmux tools) come from; an empty `-c mcp_servers={}`
///   override does NOT remove them. Auth still reads `CODEX_HOME`.
/// - `--sandbox read-only`, the tool features off, `web_search="disabled"`:
///   no tool that can act.
/// - `--ephemeral`: the side question leaves no session file behind.
/// - `--skip-git-repo-check`: the pane's bypass flag implied it; a
///   tool-less turn has nothing to protect with it.
///
/// `model` is the pane's `-m`/`--model`, if it had one.
pub(crate) fn build_codex_side_question_argv(model: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = [
        "exec",
        "--json",
        "--ignore-user-config",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for feature in CODEX_TOOL_FEATURES {
        argv.push("--disable".to_string());
        argv.push(feature.to_string());
    }
    argv.push("-c".to_string());
    argv.push("web_search=\"disabled\"".to_string());
    if let Some(model) = model {
        argv.push("-m".to_string());
        argv.push(model.to_string());
    }
    // The prompt comes from stdin; `-` stays last.
    argv.push("-".to_string());
    argv
}

/// The Gemini policy that denies every tool, built-in and MCP. Written to
/// disk for `--policy`. Checked against the pinned CLI (0.60.0) by
/// capturing its request: with this policy the request declares no tools
/// at all, even with `--yolo` on.
pub(crate) const GEMINI_DENY_ALL_TOOLS_POLICY: &str = "\
[[rule]]
toolName = \"*\"
decision = \"deny\"
priority = 999

[[rule]]
toolName = \"*\"
mcpName = \"*\"
decision = \"deny\"
priority = 999
";

/// Build the argv for a `/btw` side question on a Gemini pane: headless
/// stream-json with `GEMINI_DENY_ALL_TOOLS_POLICY` at `policy_path`, and
/// approval mode `default` instead of the pane's `--yolo`. No
/// `--skip-trust`: trusting the folder would load the project's own Gemini
/// settings, hooks included, so `/btw` gets the same trust as the pane.
pub(crate) fn build_gemini_side_question_argv(policy_path: &str, model: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = [
        "--output-format",
        "stream-json",
        "--approval-mode",
        "default",
        "--policy",
        policy_path,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(model) = model {
        argv.push("-m".to_string());
        argv.push(model.to_string());
    }
    // Headless; the prompt comes from stdin.
    argv.push("-p".to_string());
    argv.push(String::new());
    argv
}

/// The value of `-m` / `--model` (or `--model=<v>`) in `args`, if any.
pub(crate) fn model_flag_value(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "-m" || arg == "--model" {
            return it.next().filter(|v| !v.is_empty() && !v.starts_with('-')).cloned();
        }
        if let Some(v) = arg.strip_prefix("--model=") {
            return Some(v.to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

fn build_codex_argv(base: &[String], session_id: Option<&str>) -> Result<Vec<String>, String> {
    let exec_index = base
        .iter()
        .position(|arg| arg == "exec")
        .ok_or_else(|| "codex resume argv is missing the 'exec' subcommand".to_string())?;
    let prompt_index = base
        .iter()
        .rposition(|arg| arg == "-")
        .ok_or_else(|| "codex resume argv is missing the stdin prompt marker '-'".to_string())?;
    if prompt_index <= exec_index {
        return Err("codex resume argv has the stdin marker before 'exec'".to_string());
    }

    // Provider flags are appended to launch_args elsewhere, which can put
    // `--model` and friends after the stdin marker. Normalize the marker to
    // the end for both fresh and resumed Codex turns.
    let mut argv = base.to_vec();
    argv.remove(prompt_index);
    if let Some(session_id) = session_id {
        argv.insert(exec_index + 1, "resume".to_string());
        argv.push(session_id.to_string());
    }
    argv.push("-".to_string());
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::{
        build_codex_side_question_argv, build_gemini_side_question_argv, build_side_question_argv,
        build_turn_argv, model_flag_value, CODEX_TOOL_FEATURES,
    };

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn first_codex_turn_keeps_exec_argv_unchanged() {
        let base = strings(&[
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "-",
        ]);
        assert_eq!(
            build_turn_argv(&base, "codex-exec", "", None).unwrap(),
            base,
        );
    }

    #[test]
    fn first_codex_turn_moves_appended_provider_flags_before_stdin() {
        let base = strings(&["exec", "--json", "-", "--model", "gpt-5.4"]);
        assert_eq!(
            build_turn_argv(&base, "codex-exec", "", None).unwrap(),
            strings(&["exec", "--json", "--model", "gpt-5.4", "-"]),
        );
    }

    #[test]
    fn resumed_codex_turn_inserts_subcommand_and_session_before_stdin() {
        let base = strings(&[
            "exec",
            "--json",
            "--dangerously-bypass-approvals-and-sandbox",
            "-",
        ]);
        assert_eq!(
            build_turn_argv(
                &base,
                "codex-exec",
                "",
                Some("00000000-0000-0000-0000-000000000005")
            )
            .unwrap(),
            strings(&[
                "exec",
                "resume",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "00000000-0000-0000-0000-000000000005",
                "-",
            ]),
        );
    }

    #[test]
    fn container_codex_argv_preserves_executable_prefix() {
        let base = strings(&["codex", "exec", "--json", "-"]);
        assert_eq!(
            build_turn_argv(&base, "codex-exec", "", Some("thread-name")).unwrap(),
            strings(&["codex", "exec", "resume", "--json", "thread-name", "-"]),
        );
    }

    #[test]
    fn flag_resume_remains_backward_compatible() {
        let base = strings(&["-p", "--output-format", "stream-json"]);
        assert_eq!(
            build_turn_argv(&base, "flag", "--resume", Some("claude-session")).unwrap(),
            strings(&[
                "-p",
                "--output-format",
                "stream-json",
                "--resume",
                "claude-session"
            ]),
        );
    }

    #[test]
    fn malformed_codex_argv_fails_instead_of_starting_a_fresh_turn() {
        let base = strings(&["exec", "--json"]);
        assert!(build_turn_argv(&base, "codex-exec", "", Some("thread-id"))
            .unwrap_err()
            .contains("stdin prompt marker"));
    }

    // ── build_side_question_argv (/btw) ─────────────────────────────────

    #[test]
    fn side_question_argv_disables_every_tool_and_caps_turns() {
        let base = strings(&["-p", "--output-format", "stream-json", "--verbose"]);
        let got = build_side_question_argv(&base);
        assert_eq!(
            got,
            strings(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--disallowedTools",
                "*",
                "--max-turns",
                "1",
            ]),
        );
    }

    #[test]
    fn side_question_argv_never_contains_resume() {
        // No session id is ever plumbed into this path (see the function's
        // own doc comment) — assert the negative directly so a future edit
        // that accidentally threads one through gets caught here, not just
        // by omission.
        let base = strings(&["-p", "--output-format", "stream-json"]);
        let got = build_side_question_argv(&base);
        assert!(!got.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn side_question_argv_preserves_base_args_verbatim_and_in_order() {
        let base = strings(&["-p", "--model", "opus", "--my-custom-flag", "42"]);
        let got = build_side_question_argv(&base);
        assert_eq!(&got[..base.len()], base.as_slice());
    }

    #[test]
    fn side_question_argv_composes_with_build_turn_argv_resume_free() {
        // The real call path: build_side_question_argv's output becomes
        // SubprocessSpawnConfig::cli_args, which spawn_turn then feeds
        // through build_turn_argv with session_id: None (a `/btw` turn
        // never resumes). Confirm that composition still ends up
        // --resume-free and still carries both added flags, exactly as it
        // would in the real spawn.
        let base = strings(&["-p", "--output-format", "stream-json"]);
        let augmented = build_side_question_argv(&base);
        let final_argv = build_turn_argv(&augmented, "flag", "--resume", None).unwrap();
        assert_eq!(final_argv, augmented);
        assert!(!final_argv.iter().any(|a| a == "--resume"));
        assert!(final_argv.iter().any(|a| a == "--disallowedTools"));
        assert!(final_argv.iter().any(|a| a == "--max-turns"));
    }

    // ── /btw on Codex and Gemini ─────────────────────────────────────────

    #[test]
    fn the_codex_side_question_is_tool_less_and_keeps_stdin_last() {
        let got = build_codex_side_question_argv(Some("gpt-5"));
        assert_eq!(&got[..2], &strings(&["exec", "--json"])[..]);
        for flag in ["--ignore-user-config", "--ephemeral", "--skip-git-repo-check"] {
            assert!(got.iter().any(|a| a == flag), "{flag}");
        }
        assert!(got.windows(2).any(|w| w == strings(&["--sandbox", "read-only"])));
        for feature in CODEX_TOOL_FEATURES {
            assert!(got.windows(2).any(|w| w[0] == "--disable" && w[1] == *feature), "{feature}");
        }
        assert!(got.windows(2).any(|w| w == strings(&["-c", "web_search=\"disabled\""])));
        assert!(got.windows(2).any(|w| w == strings(&["-m", "gpt-5"])));
        assert!(!got.iter().any(|a| a.contains("dangerously")));
        assert_eq!(got.last().map(String::as_str), Some("-"));
        assert!(!build_codex_side_question_argv(None).iter().any(|a| a == "-m"));
    }

    #[test]
    fn the_gemini_side_question_denies_every_tool_without_yolo() {
        let got = build_gemini_side_question_argv("/data/btw/gemini.toml", Some("gemini-3-pro"));
        assert!(got.windows(2).any(|w| w == strings(&["--policy", "/data/btw/gemini.toml"])));
        assert!(got.windows(2).any(|w| w == strings(&["--approval-mode", "default"])));
        assert!(got.windows(2).any(|w| w == strings(&["--output-format", "stream-json"])));
        assert!(got.windows(2).any(|w| w == strings(&["-m", "gemini-3-pro"])));
        for flag in ["--yolo", "-y", "--skip-trust"] {
            assert!(!got.iter().any(|a| a == flag), "{flag}");
        }
        assert_eq!(&got[got.len() - 2..], &strings(&["-p", ""])[..]);
    }

    #[test]
    fn the_pane_model_is_read_from_either_flag_spelling() {
        assert_eq!(model_flag_value(&strings(&["exec", "-m", "o3", "-"])), Some("o3".into()));
        assert_eq!(model_flag_value(&strings(&["--model", "x"])), Some("x".into()));
        assert_eq!(model_flag_value(&strings(&["--model=y", "-p", ""])), Some("y".into()));
        assert_eq!(model_flag_value(&strings(&["-p", "", "--yolo"])), None);
        assert_eq!(model_flag_value(&strings(&["-m"])), None);
    }
}
