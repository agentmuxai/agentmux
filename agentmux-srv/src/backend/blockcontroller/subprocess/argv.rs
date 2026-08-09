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
    use super::build_turn_argv;

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
}
