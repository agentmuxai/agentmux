// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The visible console a terminal sign-in runs in on Windows
//! (`cli_login::open_login_terminal`).
//!
//! `cmd.exe /c start` opens it. `start` gives the new window console handles
//! of its own. Spawned directly with `CREATE_NEW_CONSOLE`, the CLI instead
//! inherited this process's standard handles: Rust always passes them
//! (`STARTF_USESTDHANDLES`), and the launcher starts the host with all three
//! set to NUL (launcher `host_spawn.rs`). The CLI's output went nowhere and
//! its prompts read an empty stdin, so the console stayed blank: Muse Code's
//! device code and "Press Enter to open it in your browser" never showed.

/// The `cmd.exe` command line that runs `cmd_str` in a console window of its
/// own, titled for the user. `cmd_str` is wrapped in one more pair of quotes:
/// `cmd /k` strips the outer pair, which leaves a quoted CLI path intact.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_console_command_line(cmd_str: &str) -> String {
    format!("/c start \"AgentMux sign-in\" cmd.exe /k \"{cmd_str}\"")
}

/// Opens `cmd_str` in a new console with `env` on top of this process's.
#[cfg(windows)]
pub(crate) fn spawn(cmd_str: &str, env: &std::collections::HashMap<String, String>) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    // The `cmd /c` that runs `start` exits at once, so it gets no window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd.exe")
        .raw_arg(windows_console_command_line(cmd_str))
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("open_login_terminal: spawn failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::windows_console_command_line;

    #[test]
    fn the_cli_runs_under_start_in_a_titled_window() {
        assert_eq!(
            windows_console_command_line(r"C:\Users\me\muse\muse-bin.exe login"),
            r#"/c start "AgentMux sign-in" cmd.exe /k "C:\Users\me\muse\muse-bin.exe login""#
        );
    }

    #[test]
    fn a_quoted_path_keeps_its_quotes_once_cmd_strips_the_outer_pair() {
        let line = windows_console_command_line(r#""C:\Program Files\x\x.exe" auth login"#);
        assert!(line.ends_with(r#"/k ""C:\Program Files\x\x.exe" auth login""#), "{line}");
    }
}
