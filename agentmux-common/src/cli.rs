// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// Resolved target of a Windows `.cmd` npm shim.
#[cfg(windows)]
enum ResolvedShim {
    /// Shim invokes a Node.js script; run via `node <path>`.
    NodeScript(String),
    /// Shim invokes a native executable directly; run via `<path>`.
    /// Seen on @anthropic-ai/claude-code v2+, which ships a prebuilt
    /// `claude.exe` instead of a JS entry point.
    Executable(String),
}

/// Create a Command for a CLI binary.
///
/// On Windows, npm-generated `.cmd` batch wrappers cannot be reliably spawned
/// via `cmd.exe /C` when stdio is piped — arguments get dropped, output is lost,
/// and the underlying CLI never executes. Instead, we parse the `.cmd` file to
/// extract the real entry point and invoke it directly (either `node <script>`
/// for JS shims or the target `.exe` directly for native-binary shims).
pub fn make_cli_cmd(cli_path: &str) -> tokio::process::Command {
    let mut cmd = build_cli_cmd(cli_path);
    // CREATE_NO_WINDOW: the host + launcher are GUI-subsystem and srv is
    // launched windowless, so a console-subsystem child (node, claude.exe,
    // cmd.exe) spawned without this flag forces Windows to allocate a fresh
    // console window — the "terminal flash" seen during dev, most visibly
    // from the ambient Haiku calls that fire as the user types
    // (server/app_api/session.rs) and the per-check `--version`/auth probes.
    // Applied at this single chokepoint so every `make_cli_cmd` caller is
    // covered; idempotent for the agent-CLI callers (persistent/subprocess/
    // acp) that already set it manually. `tokio::process::Command` exposes
    // `creation_flags` as an inherent method on Windows — no `CommandExt`
    // import needed (see agentmux-bashwrap/src/bash_wrap.rs's note).
    #[cfg(windows)]
    {
        cmd.creation_flags(crate::win32::CREATE_NO_WINDOW);
    }
    cmd
}

/// Resolve `cli_path` to a runnable `Command` (the `.cmd`/`.bat` shim parsing
/// on Windows). `make_cli_cmd` wraps this to apply `CREATE_NO_WINDOW`.
fn build_cli_cmd(cli_path: &str) -> tokio::process::Command {
    // `None` means the `.cmd` shim didn't match either known npm shape — fall
    // back to spawning the raw path directly. For a `tokio::process::Command`
    // (piped stdio, no PTY) this has the pre-existing, already-documented
    // failure mode of npm `.cmd` wrappers (args/output can get lost through
    // `cmd.exe`'s implicit association) rather than the ConPTY indefinite
    // hang — see `resolve_cli_spawn_target`'s doc for why that hang is
    // PTY-specific and why this fallback must NOT be `cmd.exe /C` (that's
    // exactly as broken here as the raw path, so there's nothing to gain by
    // routing through it explicitly).
    let (program, args) = resolve_cli_spawn_target(cli_path)
        .unwrap_or_else(|| (cli_path.to_string(), Vec::new()));
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args);
    cmd
}

/// Resolve `cli_path` to the actual `(program, args)` spawn target — the
/// `.cmd`/`.bat` shim parsing on Windows, factored out of `build_cli_cmd` so
/// non-`tokio::process::Command` spawners (e.g. `portable_pty::CommandBuilder`,
/// which `run_cli_login_pty` uses) can reuse the same resolution instead of
/// spawning the raw shim path directly. Spawning a `.cmd` file straight
/// through `CommandBuilder` forces Windows through `cmd.exe /c`, and that
/// layer hangs indefinitely under a real ConPTY (confirmed live: 0% CPU, zero
/// output, target dir never created) — this is the fix for that class of bug,
/// not just a refactor for its own sake.
///
/// Returns `None` when `cli_path` is a `.cmd`/`.bat` shim that doesn't match
/// either known npm shape (Node-script or native-exe). A previous version
/// fell back to `("cmd.exe", ["/C", cli_path])` here, but that's the exact
/// invocation this function exists to avoid — under a ConPTY it hangs just
/// as indefinitely as the raw shim path did, silently reintroducing the bug
/// for any unrecognized shim shape (reagent P2 on PR review). Callers must
/// decide how to handle an unresolvable shim themselves: `build_cli_cmd`
/// (piped stdio, no ConPTY) falls back to the raw path, which loses some
/// stdio fidelity but doesn't hang; `run_cli_login_pty` (real ConPTY) must
/// fail fast instead of attempting a doomed spawn.
pub fn resolve_cli_spawn_target(cli_path: &str) -> Option<(String, Vec<String>)> {
    #[cfg(windows)]
    if cli_path.ends_with(".cmd") || cli_path.ends_with(".bat") {
        return match parse_cmd_wrapper(cli_path) {
            Some(ResolvedShim::NodeScript(entry_script)) => {
                tracing::debug!(cmd = %cli_path, script = %entry_script, "resolved .cmd → node");
                Some(("node".to_string(), vec![entry_script]))
            }
            Some(ResolvedShim::Executable(exe_path)) => {
                tracing::debug!(cmd = %cli_path, exe = %exe_path, "resolved .cmd → native .exe");
                Some((exe_path, Vec::new()))
            }
            None => {
                tracing::warn!(cmd = %cli_path, "could not parse .cmd wrapper — no safe spawn target");
                None
            }
        };
    }
    Some((cli_path.to_string(), Vec::new()))
}

/// Parse an npm-generated `.cmd` wrapper to extract the real entry point.
///
/// npm `.cmd` wrappers contain a line like one of:
///   `"%_prog%"  "%dp0%\..\@anthropic-ai\claude-code\cli.js" %*`   (JS shim)
///   `"%dp0%\..\@anthropic-ai\claude-code\bin\claude.exe"   %*`    (native .exe shim)
/// where `%dp0%` is the directory containing the `.cmd` file itself. We extract
/// the relative path after `%dp0%\`, resolve it to an absolute path, and tag it
/// as either a Node script (`.js/.mjs/.cjs`) or a native executable (`.exe`).
#[cfg(windows)]
fn parse_cmd_wrapper(cmd_path: &str) -> Option<ResolvedShim> {
    let content = std::fs::read_to_string(cmd_path).ok()?;
    let cmd_dir = std::path::Path::new(cmd_path).parent()?;

    for line in content.lines() {
        let line_trimmed = line.trim();
        if !line_trimmed.contains("%dp0%") || !line_trimmed.contains("%*") {
            continue;
        }
        if let Some(dp0_idx) = line_trimmed.find("%dp0%\\") {
            let after_dp0 = &line_trimmed[dp0_idx + 6..]; // skip "%dp0%\"
            let end = after_dp0.find('"')
                .or_else(|| after_dp0.find(" %*"))
                .unwrap_or(after_dp0.len());
            let relative_path = &after_dp0[..end];
            let is_node_script = relative_path.ends_with(".js")
                || relative_path.ends_with(".mjs")
                || relative_path.ends_with(".cjs");
            let is_exe = relative_path.ends_with(".exe");
            if !is_node_script && !is_exe {
                continue;
            }
            let resolved = cmd_dir.join(relative_path);
            let path_str = match resolved.canonicalize() {
                Ok(canonical) => {
                    let mut s = canonical.to_string_lossy().to_string();
                    // Windows canonicalize() returns \\?\C:\... (UNC extended path)
                    // which Node.js (and some native binaries) can't handle — strip.
                    if s.starts_with(r"\\?\") {
                        s = s[4..].to_string();
                    }
                    s
                }
                Err(_) => resolved.to_string_lossy().to_string(),
            };
            return Some(if is_node_script {
                ResolvedShim::NodeScript(path_str)
            } else {
                ResolvedShim::Executable(path_str)
            });
        }
    }
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Real Claude Code v2+ `.cmd` content (`agentmux-instances/.../claude/
    /// node_modules/.bin/claude.cmd`, verified live) — ships a prebuilt
    /// native `claude.exe`, no Node entry point. This exact shape is what
    /// made `run_cli_login_pty` hang under ConPTY when spawned raw.
    const CLAUDE_EXE_SHIM: &str = concat!(
        "@ECHO off\r\n",
        "GOTO start\r\n",
        ":find_dp0\r\n",
        "SET dp0=%~dp0\r\n",
        "EXIT /b\r\n",
        ":start\r\n",
        "SETLOCAL\r\n",
        "CALL :find_dp0\r\n",
        "\"%dp0%\\..\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n",
    );

    /// Representative JS-entry-point npm shim shape (e.g. OpenClaw), the
    /// other real shape `resolve_cli_spawn_target` must handle.
    const NODE_SCRIPT_SHIM: &str = concat!(
        "@ECHO off\r\n",
        "GOTO start\r\n",
        ":find_dp0\r\n",
        "SET dp0=%~dp0\r\n",
        "EXIT /b\r\n",
        ":start\r\n",
        "SETLOCAL\r\n",
        "\"%_prog%\"  \"%dp0%\\..\\openclaw\\cli.js\" %*\r\n",
    );

    /// Lay out a `<root>/node_modules/.bin/<name>.cmd` shim pointing at
    /// `<root>/node_modules/<pkg_rel>` — the real npm install shape
    /// `parse_cmd_wrapper` expects. Target files are NOT created; canonicalize
    /// falls back to the non-canonical path (see `parse_cmd_wrapper`'s `Err`
    /// arm), so a bare `.cmd` fixture is enough — no need to fake the CLI
    /// binary/script itself.
    fn write_shim(dir: &std::path::Path, name: &str, content: &str) -> String {
        let bin_dir = dir.join("node_modules").join(".bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let cmd_path = bin_dir.join(format!("{name}.cmd"));
        std::fs::write(&cmd_path, content).unwrap();
        cmd_path.to_string_lossy().to_string()
    }

    #[test]
    fn resolves_native_exe_shim_directly_bypassing_cmd_exe() {
        let dir = tempfile::tempdir().unwrap();
        let cmd_path = write_shim(dir.path(), "claude", CLAUDE_EXE_SHIM);

        let (program, args) = resolve_cli_spawn_target(&cmd_path).expect("parseable shim must resolve");

        assert!(
            program.ends_with("claude.exe") || program.ends_with("claude-code\\bin\\claude.exe"),
            "expected the resolved native exe, got: {program}"
        );
        assert!(args.is_empty(), "exe shim must not prefix extra args, got: {args:?}");
        assert_ne!(program, "cmd.exe", "must not fall back to spawning cmd.exe /C for a parseable shim");
    }

    #[test]
    fn resolves_node_script_shim_to_node_plus_script_arg() {
        let dir = tempfile::tempdir().unwrap();
        let cmd_path = write_shim(dir.path(), "openclaw", NODE_SCRIPT_SHIM);

        let (program, args) = resolve_cli_spawn_target(&cmd_path).expect("parseable shim must resolve");

        assert_eq!(program, "node");
        assert_eq!(args.len(), 1, "expected exactly the script path as the sole arg: {args:?}");
        assert!(args[0].ends_with("cli.js"), "expected the resolved script path, got: {args:?}");
    }

    #[test]
    fn non_cmd_path_passes_through_unchanged() {
        let (program, args) =
            resolve_cli_spawn_target(r"C:\some\native\tool.exe").expect("non-.cmd path always resolves");
        assert_eq!(program, r"C:\some\native\tool.exe");
        assert!(args.is_empty());
    }

    /// An unparseable `.cmd` shim must resolve to `None`, not a `cmd.exe /C`
    /// fallback — that fallback is exactly the invocation this resolver
    /// exists to avoid, and hangs just as indefinitely under a real ConPTY
    /// as the raw shim path did (reagent P2 on PR #2422 review). Callers
    /// decide their own fallback: `build_cli_cmd` uses the raw path (piped
    /// stdio, no hang risk); `run_cli_login_pty` fails fast instead.
    #[test]
    fn unparseable_cmd_resolves_to_none_not_cmd_exe_c() {
        let dir = tempfile::tempdir().unwrap();
        let cmd_path = write_shim(dir.path(), "weird", "@ECHO off\r\necho nothing useful here\r\n");

        assert_eq!(
            resolve_cli_spawn_target(&cmd_path),
            None,
            "must not silently fall back to the broken cmd.exe /C invocation"
        );
    }
}
