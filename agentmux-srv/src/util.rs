// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Small OS-level helpers shared across backend modules.

/// Open `url` in the user's default browser.
///
/// On Windows this shells out via `cmd /C start`. `cmd.exe` re-parses its
/// trailing argument as a batch command line, where `&`, `|`, `^`, `<`, `>`
/// are operators outside of quotes — an OAuth authorize URL is full of
/// unencoded `&` between query params, so passing it bare truncates the URL
/// at the first `&` and silently drops every parameter after it (client_id,
/// redirect_uri, scope, ...). Wrapping the URL in its own quoted argument via
/// `raw_arg` keeps `cmd.exe` in quoted mode for the whole string, so those
/// characters are never seen as operators. `raw_arg` (not `arg`) is required
/// here: `Command::arg` would apply MSVCRT-style quote escaping, which
/// `cmd.exe`'s simpler quote-toggle parser does not understand and would
/// re-break out of the quoted region.
pub fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").raw_arg(format!("start \"\" \"{url}\""));
        // CREATE_NO_WINDOW: console-flash suppression, see agentmux-common/src/cli.rs
        use agentmux_common::win32::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use std::os::windows::process::CommandExt;

    // Regresses the truncation bug this module fixes: cmd.exe splits an
    // unquoted `&` into a new command, so an OAuth URL's query params after
    // the first `&` never reached the browser. Runs the same quoted `raw_arg`
    // construction as `open_browser`, swapping `start` for `echo` so the
    // output is inspectable, and asserts the URL survives intact.
    #[test]
    fn quoted_url_survives_cmd_ampersand_splitting() {
        let url = "https://example.com/oauth2/authorize?response_type=code&client_id=abc&redirect_uri=http://127.0.0.1:9379/callback&scope=openid";

        let output = std::process::Command::new("cmd")
            .arg("/C")
            .raw_arg(format!("echo \"{url}\""))
            .output()
            .expect("failed to run cmd.exe");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(url),
            "URL was truncated by cmd.exe's `&` splitting: {stdout}"
        );
    }

    // Sanity check that the bug is real: the old unquoted invocation truncates
    // at the first `&`.
    #[test]
    fn unquoted_url_is_truncated_by_cmd_ampersand_splitting() {
        let url = "https://example.com/oauth2/authorize?response_type=code&client_id=abc";

        let output = std::process::Command::new("cmd")
            .args(["/C", "echo", url])
            .output()
            .expect("failed to run cmd.exe");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains(url),
            "expected truncation, got intact URL: {stdout}"
        );
        assert!(stdout.contains("response_type=code"));
    }
}
