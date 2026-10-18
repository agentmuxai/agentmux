// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Prefix mode: `agentmux-bashwrap '<script>'` as Claude Code's
//! `CLAUDE_CODE_SHELL_PREFIX`.
//!
//! The CLI runs every Bash tool command as `<prefix> '<its script>'` when the
//! variable is set, and leaves the tool call itself alone: its permission
//! rules, labels and checks see the model's own command. The `PreToolUse`
//! hook records each call here (`register`) instead of rewriting it; the
//! prefix process takes the model's command back out of the CLI's script,
//! claims the matching record for the call's `tool_use_id`, and runs the
//! whole script the way `exec` runs a command (bash_wrap.rs).
//!
//! docs/specs/SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md §2.

use std::ffi::OsString;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use agentmux_common::time::now_ms_u64 as now_ms;
use anyhow::{Context, Result};

use serde::{Deserialize, Serialize};

/// The CLI's environment variable naming the prefix.
pub const SHELL_PREFIX_ENV: &str = "CLAUDE_CODE_SHELL_PREFIX";
/// The CLI's session id, as the prefix process sees it. Equal to the
/// `session_id` in the hook's payload.
pub const SESSION_ID_ENV: &str = "CLAUDE_CODE_SESSION_ID";
/// Overrides where records are kept (tests).
const CALLS_DIR_OVERRIDE_ENV: &str = "AGENTMUX_BASHWRAP_CALLS_DIR";
/// Records older than this are deleted on each claim: a call the CLI never
/// ran (denied, interrupted) leaves its record behind.
const RECORD_MAX_AGE: Duration = Duration::from_secs(3600);
/// The subcommands, which a lone argument must not be mistaken for.
const SUBCOMMANDS: &[&str] = &["exec", "hook", "precompact", "sessionstart", "help"];

/// One Bash call, as the hook saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRecord {
    pub tool_use_id: String,
    pub command: String,
    #[serde(default)]
    pub run_in_background: bool,
    pub created_ms: u64,
    /// What the model said the call does: Tower labels the process that ran
    /// it with this (`report_call`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The subagent that made the call (the PreToolUse payload's
    /// `agent_id`); none for the main conversation. Its working directory is
    /// its own (`cwd_state::CwdState::load`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// The longest description reported for a call.
const MAX_DESCRIPTION_CHARS: usize = 200;

/// The script, when this process was started as the CLI's shell prefix:
/// exactly one argument, which is neither a subcommand nor a flag.
///
/// The CLI puts the prefix in front of more than Bash calls: hook commands,
/// and every stdio MCP server, whose command and arguments it joins into
/// that one argument. So a lone word is a prefix call too: the `agentmux`
/// MCP server arrives as just `agentmux-mcp`, and refusing it as an unknown
/// subcommand left agents without their AgentMux tools
/// (docs/reports/REPORT_AGENT_MCP_DOWN_THROUGH_SHELL_PREFIX_2026_10_11.md).
pub fn prefix_script(args: &[OsString]) -> Option<String> {
    if args.len() != 2 {
        return None;
    }
    let script = args[1].to_str()?;
    let looks_like_cli_arg = script.starts_with('-') || SUBCOMMANDS.contains(&script);
    (!looks_like_cli_arg && !script.trim().is_empty()).then(|| script.to_string())
}

/// Whether this process runs under a CLI whose shell prefix is AgentMux's
/// wrapper, so the hook records calls instead of rewriting them.
pub fn prefix_active() -> bool {
    std::env::var_os(SHELL_PREFIX_ENV)
        .map(|v| is_bashwrap_path(Path::new(&v)))
        .unwrap_or(false)
}

fn is_bashwrap_path(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("agentmux-bashwrap"))
        .unwrap_or(false)
}

/// The model's command inside the CLI's script, and where its quoted form
/// sits in the script: the shell word after the CLI's `eval `, which the CLI
/// quotes with single quotes (an embedded `'` becomes `'"'"'`). `None` when
/// the script doesn't have one, e.g. after a change to the CLI's format.
pub fn extract_command(script: &str) -> Option<(String, Range<usize>)> {
    // The CLI's own `eval` comes before the command, which is inside it.
    let at = script.find("&& eval '")? + "&& eval ".len();
    let (command, end) = read_shell_word(script, at)?;
    Some((command, at..end))
}

/// Read one shell word starting at `start`: single-quoted, double-quoted and
/// backslash-escaped parts, up to unquoted whitespace or the end. Returns the
/// word's value and the byte offset after it.
fn read_shell_word(s: &str, start: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                let close = s[i + 1..].find('\'')? + i + 1;
                out.push_str(&s[i + 1..close]);
                i = close + 1;
            }
            b'"' => {
                i += 1;
                loop {
                    let c = *bytes.get(i)?;
                    if c == b'"' {
                        i += 1;
                        break;
                    }
                    if c == b'\\' && matches!(bytes.get(i + 1), Some(b'"' | b'\\' | b'$' | b'`')) {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    let ch = s[i..].chars().next()?;
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
            b'\\' => {
                let ch = s[i + 1..].chars().next()?;
                out.push(ch);
                i += 1 + ch.len_utf8();
            }
            c if c.is_ascii_whitespace() => break,
            _ => {
                let ch = s[i..].chars().next()?;
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    (i > start).then_some((out, i))
}

/// `s` as one single-quoted shell word, the way the CLI quotes it.
pub fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// The script with the command at `range` replaced by `command`.
pub fn replace_command(script: &str, range: Range<usize>, command: &str) -> String {
    format!("{}{}{}", &script[..range.start], single_quote(command), &script[range.end..])
}

fn calls_root() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os(CALLS_DIR_OVERRIDE_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(over));
    }
    Some(dirs::home_dir()?.join(".agentmux").join("state").join("bashwrap-calls"))
}

fn session_dir(session_id: &str) -> Option<PathBuf> {
    Some(calls_root()?.join(sanitize(session_id)))
}

fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Record a call for the prefix process to claim. Written to a temporary
/// name and renamed, so a claim never reads half a record. Then other
/// sessions' leftovers are swept (`sweep_other_sessions`).
pub fn register(session_id: &str, record: &CallRecord) -> std::io::Result<()> {
    let dir = session_dir(session_id)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    let name = format!("{}.json", sanitize(&record.tool_use_id));
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    let bytes = serde_json::to_vec(record)?;
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, dir.join(&name))
    };
    // A claim or a sweep removes a directory it finds empty, which can land
    // between creating it and writing into it: then once more.
    write().or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { write() } else { Err(e) })?;
    if let Some(root) = calls_root() {
        sweep_other_sessions(&root, &dir);
    }
    Ok(())
}

/// Every other session's directory: its stale records deleted, and the
/// directory itself removed once nothing is left in it. Sessions end
/// without telling anyone, and a call the CLI never ran (denied) leaves its
/// record; without this both would stay forever.
fn sweep_other_sessions(root: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !path.is_dir() {
            continue;
        }
        if read_records(&path).is_empty() {
            // Fails (and is left) if a record or a temporary file is in it.
            let _ = std::fs::remove_dir(&path);
        }
    }
}

/// Claim the record of the call this script runs: the newest one for the
/// session whose command is `command`; failing that, the session's only
/// record. Newest, because the hook records a call just before the CLI runs
/// it, while a call the CLI never ran (a deny rule, a refused prompt) leaves
/// an older record behind that a retry of the same command must not take. A claim renames the record away, so two
/// prefix processes never claim the same call. `None` when there is nothing
/// to claim: the script then runs unlinked. Only a Bash call's script, one
/// with a command in its `eval`, ever claims (`run`).
pub fn claim(session_id: &str, command: &str) -> Option<CallRecord> {
    let dir = session_dir(session_id)?;
    let mut records = read_records(&dir);
    records.sort_by_key(|(_, r)| std::cmp::Reverse(r.created_ms));
    let matching: Vec<&(PathBuf, CallRecord)> =
        records.iter().filter(|(_, r)| r.command == command).collect();
    let candidates: Vec<&(PathBuf, CallRecord)> = if !matching.is_empty() {
        matching
    } else if records.len() == 1 {
        records.iter().collect()
    } else {
        Vec::new()
    };
    let mut claimed_record = None;
    for (path, record) in candidates {
        let claimed = path.with_extension(format!("claimed.{}", std::process::id()));
        if std::fs::rename(path, &claimed).is_ok() {
            let _ = std::fs::remove_file(&claimed);
            claimed_record = Some(record.clone());
            break;
        }
    }
    // The session's last waiting call: its directory goes too (it is made
    // again for the next one). Fails, and stays, while anything is in it.
    let _ = std::fs::remove_dir(&dir);
    claimed_record
}

/// The session's unclaimed records, deleting any past their age.
fn read_records(dir: &Path) -> Vec<(PathBuf, CallRecord)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let cutoff = now_ms().saturating_sub(RECORD_MAX_AGE.as_millis() as u64);
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_record = path.extension().and_then(|e| e.to_str()) == Some("json")
            && !path.file_name().and_then(|n| n.to_str()).unwrap_or("").starts_with('.');
        if !is_record {
            continue;
        }
        match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<CallRecord>(&b).ok()) {
            Some(record) if record.created_ms >= cutoff => out.push((path, record)),
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    out
}

/// Run the CLI's whole `script` for the Bash call it belongs to, streaming it
/// exactly as `exec` streams a command. The call is the record the hook left
/// for this session's command. A script that can't be linked to a call (a
/// `claude` run by hand or nested inside an agent, whose session has no
/// records, or the CLI's own shell calls) runs as plain bash, as if there
/// were no prefix: no live output, nothing else changed.
/// docs/specs/SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md §2.2, §2.3.
pub async fn run(script: String) -> Result<i32> {
    // Only a Bash call's script has the CLI's `eval`. Anything else is a hook
    // or an MCP server, which runs as it is and never takes a Bash call's
    // record: in the PTY, a hook's answer or a server's JSON-RPC would be
    // garbled, and the call it took would lose its live output.
    let Some((command, range)) = extract_command(&script) else {
        tracing::info!(target: "bashwrap", "prefix: not a Bash call; running it as it is");
        return run_passthrough(&script);
    };
    let session = std::env::var(SESSION_ID_ENV).ok().filter(|s| !s.is_empty());
    let record = session.as_deref().and_then(|s| claim(s, &command));
    let Some(record) = record else {
        tracing::info!(target: "bashwrap", session = ?session, "prefix: no call to link; running the script as plain bash");
        return run_plain(&script);
    };
    // The tee rewrite works on the model's command, so it is applied to the
    // command inside the script's `eval` and the script rebuilt around it.
    let script = match crate::hook::tee_redirect_rewrite(&command) {
        Some(teed) => replace_command(&script, range, &teed),
        None => script,
    };
    // Not awaited before the command starts: a slow srv must not hold it up.
    let report = record.description.clone().map(|d| tokio::spawn(report_call(record.tool_use_id.clone(), d)));
    let args = crate::bash_wrap::Args {
        tool_id: record.tool_use_id,
        b64_cmd: String::new(),
        block_id: None,
        declared_background: record.run_in_background,
        agent: record.agent_id,
    };
    let result = crate::bash_wrap::run_command(args, script).await;
    if let Some(task) = report {
        // Bounded past the publish's own timeout; main exits right after.
        let _ = tokio::time::timeout(Duration::from_secs(6), task).await;
    }
    result
}

/// Tell srv which call this process runs, once per linked call, so srv can say which tool call
/// a process tree is (Tower's "started by"): this wrapper's pid, the call and what it does. srv
/// keeps it and does not forward it to the pane, which would take an unknown op for output.
/// SPEC_TOWER_AGENT_CENTRIC_VIEWS_2026_10_08.md §5.3. Best effort.
async fn report_call(tool_id: String, description: String) {
    let Some(client) = crate::mps_client::WpsClient::from_env() else { return };
    let block_id = std::env::var("AGENTMUX_BLOCKID").ok().filter(|b| !b.is_empty());
    let description: String = description.chars().take(MAX_DESCRIPTION_CHARS).collect();
    let msg = agentmux_common::client_events::ToolChunk::Call { tool_id: tool_id.clone(), pid: std::process::id(), description, timestamp: now_ms() };
    if let Err(e) = client.publish_chunk(block_id.as_deref(), &msg).await {
        tracing::warn!(target: "bashwrap", tool_id, error = %e, "call report failed");
    }
}

/// Run a hook command or an MCP server's command line on this process's own
/// stdio, untouched: an MCP server speaks JSON-RPC on it, so one added byte
/// breaks it. A lone word is a program, started directly (no shell to
/// mangle a Windows path, nothing to locate first); anything else is a
/// command line, which the CLI joined unquoted, so bash runs it. Except on
/// Windows when the program is a Windows path: bash would strip its
/// backslashes, and only an MCP server's line can have one (the CLI makes a
/// hook's paths POSIX), so it is split back into words, as the CLI joined it.
fn run_passthrough(script: &str) -> Result<i32> {
    let script = script.trim();
    let mut words = script.split_whitespace();
    let program = words.next().unwrap_or_default();
    if !script.contains(char::is_whitespace) || (cfg!(windows) && program.contains('\\')) {
        return run_program(program, &words.collect::<Vec<_>>());
    }
    run_plain(script)
}

/// Become the program, so the CLI's signals and pid reach it. Returns only
/// when it can't be started.
#[cfg(unix)]
fn run_program(program: &str, args: &[&str]) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(program).args(args).exec();
    eprintln!("agentmux-bashwrap: cannot run {program}: {err}");
    Ok(127)
}

/// Run the program and wait for it. The block's Job Object ends it with the
/// agent, and it sees stdin close when the CLI closes its end. Found through
/// `PATHEXT` the way the CLI would find it without a prefix, so a `.cmd`
/// shim (what a global npm install puts on `PATH`) starts too: std looks
/// only for `.exe`, but runs a `.cmd` given its full path.
#[cfg(windows)]
fn run_program(program: &str, args: &[&str]) -> Result<i32> {
    let resolved = which::which(program).unwrap_or_else(|_| PathBuf::from(program));
    let mut cmd = std::process::Command::new(resolved);
    cmd.args(args);
    // A console program, like bash.exe in `run_plain`.
    cmd.no_window();
    match cmd.status() {
        Ok(status) => Ok(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("agentmux-bashwrap: cannot run {program}: {e}");
            Ok(127)
        }
    }
}

/// Run `script` with bash on this process's own stdio, for its exit code.
fn run_plain(script: &str) -> Result<i32> {
    let bash = crate::bash_wrap::locate_bash()?;
    let mut cmd = std::process::Command::new(&bash);
    cmd.arg("-c").arg(script);
    // Windows only: bash.exe is a console program; without this it opens a
    // console window (SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20).
    #[cfg(windows)]
    cmd.no_window();
    let status = cmd.status().with_context(|| format!("running {}", bash.display()))?;
    Ok(status.code().unwrap_or(1))
}

/// A record for a call the hook just saw.
pub fn new_record(tool_use_id: &str, command: &str, run_in_background: bool) -> CallRecord {
    CallRecord {
        tool_use_id: tool_use_id.to_string(),
        command: command.to_string(),
        run_in_background,
        created_ms: now_ms(),
        description: None,
        agent_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The script Claude Code 2.1.288 passed to the prefix, recorded for
    /// `echo 'it''s' "q" && echo two > out.txt; cat out.txt`.
    const CLI_SCRIPT: &str = r#"source /c/Users/u/AppData/Local/Temp/config/shell-snapshots/snapshot-bash-1.sh 2>/dev/null || true && export TEMP='C:\Users\u\AppData\Local\Temp' TMP='C:\Users\u\AppData\Local\Temp' && { shopt -u extglob || setopt NO_EXTENDED_GLOB NO_BARE_GLOB_QUAL; } >/dev/null 2>&1 || true && { \builtin unalias -- 'unsetenv'; \builtin unset -f -- 'unsetenv'; } >/dev/null 2>&1 || true && eval 'echo '"'"'it'"'"''"'"'s'"'"' "q" && echo two > out.txt; cat out.txt' < /dev/null && pwd -P >| /c/Users/u/AppData/Local/Temp/claude-941e-cwd"#;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_lone_script_argument_is_a_prefix_call() {
        assert_eq!(prefix_script(&os(&["bw", CLI_SCRIPT])).as_deref(), Some(CLI_SCRIPT));
    }

    #[test]
    fn subcommands_flags_and_other_shapes_are_not() {
        for args in [
            vec!["bw", "hook"],
            vec!["bw", "exec"],
            vec!["bw", "--version"],
            vec!["bw", "--help"],
            vec!["bw"],
            vec!["bw", "exec", "--tool-id=x"],
            vec!["bw", "  "],
        ] {
            assert_eq!(prefix_script(&os(&args)), None, "{args:?}");
        }
    }

    /// The CLI also prefixes hooks and stdio MCP servers, joining a server's
    /// command and arguments into one argument; `agentmux-mcp` arrives alone.
    #[test]
    fn hooks_and_mcp_servers_are_prefix_calls_too() {
        for script in ["agentmux-mcp", r"C:\tools\server.exe", "npx -y @scope/server", "agentmux-bashwrap hook"] {
            assert_eq!(prefix_script(&os(&["bw", script])).as_deref(), Some(script));
        }
    }

    /// A hook or an MCP server started while a Bash call waits must not take
    /// that call's record: it would run in the PTY, and the call would lose
    /// its live output.
    #[test]
    fn a_script_without_the_clis_eval_never_claims_a_call() {
        with_calls_dir(|| {
            register("s", &new_record("toolu_1", "cargo build", false)).unwrap();
            std::env::set_var(SESSION_ID_ENV, "s");
            let rt = tokio::runtime::Runtime::new().unwrap();
            let code = rt.block_on(run("agentmux-no-such-program-4f1c".to_string())).unwrap();
            std::env::remove_var(SESSION_ID_ENV);
            assert_eq!(code, 127, "a program that can't be started exits like a shell's");
            assert_eq!(claim("s", "cargo build").unwrap().tool_use_id, "toolu_1", "the call is still waiting");
        });
    }

    /// A global npm install puts a `.cmd` shim on `PATH`; named alone, as an
    /// MCP server's command, it must start.
    #[cfg(windows)]
    #[test]
    fn a_cmd_shim_on_path_starts_as_a_lone_word() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bashwrap-shim-test-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bw-shim-test.cmd"), "@exit /b 9\r\n").unwrap();
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![dir.clone()];
        paths.extend(std::env::split_paths(&old_path));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        let code = run_program("bw-shim-test", &[]);
        std::env::set_var("PATH", &old_path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(code.unwrap(), 9);
    }

    /// `C:\tools\server.exe --port 1`, an MCP server's joined line: started
    /// directly with its arguments, not through bash, which would eat the
    /// backslashes.
    #[cfg(windows)]
    #[test]
    fn a_windows_path_with_arguments_starts_without_bash() {
        let dir = std::env::temp_dir().join(format!("bashwrap-winpath-test-{}-{}", std::process::id(), now_ms()));
        if dir.to_string_lossy().contains(char::is_whitespace) {
            return; // A path with a space is a command line the CLI made ambiguous.
        }
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("server.cmd");
        std::fs::write(&script, "@exit /b %1\r\n").unwrap();
        let line = format!("{} 5", script.display());
        let code = run_passthrough(&line);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(code.unwrap(), 5, "its argument reached it");
    }

    #[test]
    fn takes_the_command_out_of_the_clis_script() {
        let (command, range) = extract_command(CLI_SCRIPT).unwrap();
        assert_eq!(command, r#"echo 'it''s' "q" && echo two > out.txt; cat out.txt"#);
        assert!(CLI_SCRIPT[range.end..].starts_with(" < /dev/null && pwd -P"));
        assert!(CLI_SCRIPT[..range.start].ends_with("&& eval "));
    }

    #[test]
    fn putting_the_command_back_round_trips() {
        let (command, range) = extract_command(CLI_SCRIPT).unwrap();
        assert_eq!(replace_command(CLI_SCRIPT, range.clone(), &command), CLI_SCRIPT);
        let rewritten = replace_command(CLI_SCRIPT, range, "echo 'a' | tee out.txt");
        assert_eq!(extract_command(&rewritten).unwrap().0, "echo 'a' | tee out.txt");
    }

    #[test]
    fn multi_line_and_unicode_commands_survive() {
        let cmd = "for f in *; do\n  echo \"$f\" — ✓ 'x'\ndone";
        let script = format!("source s.sh || true && eval {} < /dev/null && pwd -P >| f", single_quote(cmd));
        assert_eq!(extract_command(&script).unwrap().0, cmd);
    }

    #[test]
    fn a_script_without_the_clis_eval_has_no_command() {
        assert_eq!(extract_command("echo hi && ls"), None);
        assert_eq!(extract_command("x && eval 'unterminated"), None);
    }

    #[test]
    fn only_a_path_to_agentmux_bashwrap_turns_the_prefix_on() {
        assert!(is_bashwrap_path(Path::new("C:/Program Files/AgentMux/tools/bin/agentmux-bashwrap.exe")));
        assert!(is_bashwrap_path(Path::new("/usr/local/bin/agentmux-bashwrap")));
        assert!(!is_bashwrap_path(Path::new("/usr/local/bin/logger.sh")));
        assert!(!is_bashwrap_path(Path::new("")));
    }

    fn with_calls_dir<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bashwrap-calls-test-{}-{}", std::process::id(), now_ms()));
        std::env::set_var(CALLS_DIR_OVERRIDE_ENV, &dir);
        let out = f();
        std::env::remove_var(CALLS_DIR_OVERRIDE_ENV);
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn a_claim_takes_the_matching_record_once() {
        with_calls_dir(|| {
            register("s1", &new_record("toolu_a", "cargo build", false)).unwrap();
            register("s1", &new_record("toolu_b", "npm test", true)).unwrap();
            register("s1", &new_record("toolu_c", "ls", false)).unwrap();
            let got = claim("s1", "npm test").unwrap();
            assert_eq!((got.tool_use_id.as_str(), got.run_in_background), ("toolu_b", true));
            assert_eq!(claim("s1", "npm test"), None, "claimed once: two others left, neither matches");
            assert_eq!(claim("s1", "cargo build").unwrap().tool_use_id, "toolu_a");
            // One record left: a command that doesn't match exactly (the CLI
            // may tidy it) still gets the session's only waiting call.
            assert_eq!(claim("s1", "ls ").unwrap().tool_use_id, "toolu_c");
        });
    }

    /// A call the CLI never ran (denied) leaves its record; the retry of the
    /// same command, recorded later, is the one its run claims.
    #[test]
    fn the_newest_record_for_a_command_is_claimed_first() {
        with_calls_dir(|| {
            let mut denied = new_record("toolu_denied", "make", false);
            denied.created_ms -= 10_000;
            register("s", &denied).unwrap();
            register("s", &new_record("toolu_retry", "make", true)).unwrap();
            let got = claim("s", "make").unwrap();
            assert_eq!((got.tool_use_id.as_str(), got.run_in_background), ("toolu_retry", true));
        });
    }

    #[test]
    fn without_a_match_only_a_lone_record_is_claimed() {
        with_calls_dir(|| {
            register("s", &new_record("toolu_1", "a", false)).unwrap();
            assert_eq!(claim("s", "no match").unwrap().tool_use_id, "toolu_1");
            register("s", &new_record("toolu_2", "b", false)).unwrap();
            register("s", &new_record("toolu_3", "c", false)).unwrap();
            assert_eq!(claim("s", "zzz"), None, "two candidates: ambiguous");
            assert_eq!(claim("other-session", "b"), None, "another session's records are never claimed");
        });
    }

    #[test]
    fn a_session_directory_goes_once_its_last_call_is_claimed() {
        with_calls_dir(|| {
            register("s", &new_record("toolu_1", "a", false)).unwrap();
            register("s", &new_record("toolu_2", "b", false)).unwrap();
            let dir = session_dir("s").unwrap();
            claim("s", "a").unwrap();
            assert!(dir.is_dir(), "a call still waits in it");
            claim("s", "b").unwrap();
            assert!(!dir.exists());
            // The next call makes it again.
            register("s", &new_record("toolu_3", "c", false)).unwrap();
            assert_eq!(claim("s", "c").unwrap().tool_use_id, "toolu_3");
        });
    }

    #[test]
    fn a_new_record_sweeps_other_sessions_left_behind() {
        with_calls_dir(|| {
            // A session whose only call was denied: its record is past its age.
            let mut denied = new_record("toolu_old", "x", false);
            denied.created_ms -= RECORD_MAX_AGE.as_millis() as u64 + 1;
            register("ended", &denied).unwrap();
            // A session that ended with nothing waiting: an empty directory.
            std::fs::create_dir_all(session_dir("empty").unwrap()).unwrap();
            // A live session with a call waiting.
            register("live", &new_record("toolu_live", "y", false)).unwrap();
            register("current", &new_record("toolu_now", "z", false)).unwrap();
            assert!(!session_dir("ended").unwrap().exists());
            assert!(!session_dir("empty").unwrap().exists());
            assert_eq!(claim("live", "y").unwrap().tool_use_id, "toolu_live");
            assert_eq!(claim("current", "z").unwrap().tool_use_id, "toolu_now");
        });
    }

    #[test]
    fn stale_records_are_dropped() {
        with_calls_dir(|| {
            let mut old = new_record("toolu_old", "x", false);
            old.created_ms -= RECORD_MAX_AGE.as_millis() as u64 + 1;
            register("s", &old).unwrap();
            assert_eq!(claim("s", "x"), None);
            assert!(read_records(&session_dir("s").unwrap()).is_empty());
        });
    }
}
