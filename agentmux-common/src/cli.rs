/// Create a Command for a CLI binary.
///
/// On Windows, npm-generated `.cmd` batch wrappers cannot be reliably spawned
/// via `cmd.exe /C` when stdio is piped — arguments get dropped, output is lost,
/// and the underlying CLI never executes. Instead, we parse the `.cmd` file to
/// extract the Node.js entry script path and invoke `node <script>` directly.
pub fn make_cli_cmd(cli_path: &str) -> tokio::process::Command {
    #[cfg(windows)]
    if cli_path.ends_with(".cmd") || cli_path.ends_with(".bat") {
        if let Some(entry_script) = parse_cmd_wrapper(cli_path) {
            tracing::debug!(cmd = %cli_path, script = %entry_script, "resolved .cmd → node");
            let mut c = tokio::process::Command::new("node");
            c.arg(&entry_script);
            return c;
        }
        tracing::warn!(cmd = %cli_path, "could not parse .cmd wrapper, falling back to cmd.exe /C");
        let mut c = tokio::process::Command::new("cmd.exe");
        c.args(["/C", cli_path]);
        return c;
    }
    tokio::process::Command::new(cli_path)
}

/// Parse an npm-generated `.cmd` wrapper to extract the Node.js entry script path.
///
/// npm `.cmd` wrappers contain a line like:
///   `"%_prog%"  "%dp0%\..\@anthropic-ai\claude-code\cli.js" %*`
/// where `%dp0%` is the directory containing the `.cmd` file itself.
/// We extract the relative path after `%dp0%\` and resolve it to an absolute path.
#[cfg(windows)]
fn parse_cmd_wrapper(cmd_path: &str) -> Option<String> {
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
            if relative_path.ends_with(".js") || relative_path.ends_with(".mjs") || relative_path.ends_with(".cjs") {
                let resolved = cmd_dir.join(relative_path);
                if let Ok(canonical) = resolved.canonicalize() {
                    return Some(canonical.to_string_lossy().to_string());
                }
                return Some(resolved.to_string_lossy().to_string());
            }
        }
    }
    None
}
