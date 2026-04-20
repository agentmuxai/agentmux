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
    #[cfg(windows)]
    if cli_path.ends_with(".cmd") || cli_path.ends_with(".bat") {
        match parse_cmd_wrapper(cli_path) {
            Some(ResolvedShim::NodeScript(entry_script)) => {
                tracing::debug!(cmd = %cli_path, script = %entry_script, "resolved .cmd → node");
                let mut c = tokio::process::Command::new("node");
                c.arg(&entry_script);
                return c;
            }
            Some(ResolvedShim::Executable(exe_path)) => {
                tracing::debug!(cmd = %cli_path, exe = %exe_path, "resolved .cmd → native .exe");
                return tokio::process::Command::new(&exe_path);
            }
            None => {
                tracing::warn!(cmd = %cli_path, "could not parse .cmd wrapper, falling back to cmd.exe /C");
                let mut c = tokio::process::Command::new("cmd.exe");
                c.args(["/C", cli_path]);
                return c;
            }
        }
    }
    tokio::process::Command::new(cli_path)
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
