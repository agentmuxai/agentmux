// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WSL terminals (P1 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! A WSL pane is `wsl.exe -d <distro>` in a ConPTY: no network, no helper, no
//! durability (the distro's shell lives as long as its VM, spec §3.3 and §5.5).

/// What `wsl.exe` needs to start a pane's process in a distro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslLaunch {
    /// Arguments for `wsl.exe`.
    pub args: Vec<String>,
}

/// Environment variables a WSL shell should see from the Windows side. WSL does
/// not pass the Windows environment through; `WSLENV` lists what crosses. Never
/// the local URL or auth key: those name this machine's srv, which a shell in
/// the distro has no business calling without the user knowing.
pub const FORWARDED_ENV: &[&str] = &[
    "TERM",
    "COLORTERM",
    "TERM_PROGRAM",
    "AGENTMUX_BLOCKID",
    "AGENTMUX_TABID",
    "AGENTMUX_VERSION",
];

/// The `wsl.exe` arguments for a pane: an interactive login shell, or the
/// pane's command run by the distro's `sh`. `cwd` is a path inside the distro;
/// empty means the user's home.
pub fn launch(distro: &str, cmd: &str, cmd_args: &[String], cwd: &str) -> WslLaunch {
    let mut args = vec!["-d".to_string(), distro.to_string(), "--cd".to_string()];
    args.push(if cwd.trim().is_empty() {
        "~".to_string()
    } else {
        cwd.to_string()
    });
    if !cmd.is_empty() {
        args.push("--".to_string());
        if cmd_args.is_empty() {
            // A command line, as a local pane runs it through `sh -c`.
            args.extend(["sh".to_string(), "-c".to_string(), cmd.to_string()]);
        } else {
            args.push(cmd.to_string());
            args.extend(cmd_args.iter().cloned());
        }
    }
    WslLaunch { args }
}

/// `WSLENV` with [`FORWARDED_ENV`] added to whatever the user already set,
/// without duplicates.
pub fn wslenv(existing: &str) -> String {
    // An `AGENTMUX_*` entry the user's own WSLENV already lists is dropped
    // unless it is one of ours: the pane sets AGENTMUX_AUTH_KEY and
    // AGENTMUX_LOCAL_URL on wsl.exe, and a listed name would carry them in.
    let mut parts: Vec<String> = existing
        .split(':')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let name = p.split('/').next().unwrap_or_default();
            !name.starts_with("AGENTMUX_") || FORWARDED_ENV.contains(&name)
        })
        .map(str::to_string)
        .collect();
    for name in FORWARDED_ENV {
        if !parts.iter().any(|p| p.split('/').next() == Some(*name)) {
            parts.push((*name).to_string());
        }
    }
    parts.join(":")
}

/// `WSLENV` for a process in a distro: the terminal variables, the names its
/// caller set (an agent `Shell` call's `env`, or a pane's configured
/// `cmd:env`), and `GH_CONFIG_DIR` translated to a distro path (`/p`), so an
/// agent's plain-`gh` guard holds inside the distro too and `gh` there cannot
/// act as the user. Never an `AGENTMUX_*` variable beyond the terminal ones
/// and the agent id: the auth key, the local URL and the jekt key stay on
/// Windows.
pub fn wslenv_with<'a>(existing: &str, caller_keys: impl IntoIterator<Item = &'a str>) -> String {
    let guard = crate::backend::gh_guard::GH_CONFIG_DIR;
    // Whatever flag the user's own WSLENV gives GH_CONFIG_DIR (`/u` would keep
    // it from crossing), the guard's own `/p` entry replaces it.
    let mut env = wslenv(existing)
        .split(':')
        .filter(|p| p.split('/').next() != Some(guard))
        .collect::<Vec<_>>()
        .join(":");
    let mut push = |entry: String| {
        let name = entry.split('/').next().unwrap_or_default().to_string();
        let listed = env
            .split(':')
            .any(|p| p.split('/').next() == Some(name.as_str()));
        if !listed {
            if !env.is_empty() {
                env.push(':');
            }
            env.push_str(&entry);
        }
    };
    for key in caller_keys {
        let forwardable = !key.is_empty()
            && !key.contains([':', '/', '='])
            && (!key.starts_with("AGENTMUX_")
                || FORWARDED_ENV.contains(&key)
                || key == "AGENTMUX_AGENT_ID")
            && key != guard
            && key != "WSLENV";
        if forwardable {
            push(key.to_string());
        }
    }
    push(format!("{guard}/p"));
    env
}

/// Parse `wsl.exe --list --quiet`. It writes UTF-16LE (with or without a BOM),
/// one distribution per line; with none installed it prints a sentence and
/// exits non-zero, which the caller treats as "none".
pub fn parse_list(stdout: &[u8]) -> Vec<String> {
    // UTF-16LE when it starts with its BOM, or when every second byte is zero
    // (ASCII names in UTF-16LE); otherwise UTF-8.
    let body = stdout.strip_prefix(&[0xff, 0xfe]).unwrap_or(stdout);
    let utf16 = body.len() != stdout.len()
        || (stdout.len() >= 2
            && stdout.len() % 2 == 0
            && stdout.iter().skip(1).step_by(2).all(|b| *b == 0));
    let text = if utf16 && body.len() % 2 == 0 {
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(stdout).into_owned()
    };
    text.trim_start_matches('\u{feff}')
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\0'))
        .filter(|l| !l.is_empty())
        // Docker Desktop's own utility VMs: a root shell with no home and
        // nothing a user works in. Listing them only invites a confusing pane.
        .filter(|l| {
            !l.eq_ignore_ascii_case("docker-desktop")
                && !l.eq_ignore_ascii_case("docker-desktop-data")
        })
        .filter(|l| {
            super::conn::ConnTarget::parse(&format!("{}{l}", super::conn::WSL_PREFIX)).is_ok()
        })
        .map(str::to_string)
        .collect()
}

/// The installed distributions, or none (not Windows, no WSL, or no distro).
pub async fn list() -> Vec<String> {
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        let mut cmd = tokio::process::Command::new("wsl.exe");
        cmd.args(["--list", "--quiet"]).no_window();
        match tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => parse_list(&out.stdout),
            _ => Vec::new(),
        }
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend_from_slice(&0xfeffu16.to_le_bytes());
        }
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    #[test]
    fn the_list_is_read_in_utf16_with_or_without_a_bom() {
        for bom in [false, true] {
            let out = utf16("Ubuntu\r\ndocker-desktop\r\n\r\nUbuntu-24.04\r\n", bom);
            assert_eq!(
                parse_list(&out),
                ["Ubuntu", "Ubuntu-24.04"],
                "docker-desktop is Docker's utility VM, not the user's"
            );
        }
        // Some builds print UTF-8.
        assert_eq!(
            parse_list(b"Debian\nkali-linux\n"),
            ["Debian", "kali-linux"]
        );
        assert!(parse_list(b"").is_empty());
    }

    #[test]
    fn a_line_that_is_not_a_distro_name_is_dropped() {
        // The "no installed distributions" sentence, if it ever arrives on stdout.
        let out = utf16(
            "Windows Subsystem for Linux has no installed distributions.\r\n",
            false,
        );
        assert!(parse_list(&out).is_empty());
    }

    #[test]
    fn an_interactive_pane_is_a_login_shell_at_home_or_its_cwd() {
        assert_eq!(
            launch("Ubuntu", "", &[], "").args,
            ["-d", "Ubuntu", "--cd", "~"]
        );
        assert_eq!(
            launch("Ubuntu", "", &[], "/srv/app").args,
            ["-d", "Ubuntu", "--cd", "/srv/app"]
        );
    }

    #[test]
    fn a_command_runs_in_the_distro_not_on_windows() {
        assert_eq!(
            launch("Ubuntu", "make test && echo ok", &[], "").args,
            [
                "-d",
                "Ubuntu",
                "--cd",
                "~",
                "--",
                "sh",
                "-c",
                "make test && echo ok"
            ]
        );
        assert_eq!(
            launch("Ubuntu", "htop", &["-d".to_string(), "10".to_string()], "").args,
            ["-d", "Ubuntu", "--cd", "~", "--", "htop", "-d", "10"]
        );
    }

    #[test]
    fn wslenv_adds_the_terminal_variables_once_and_keeps_the_users() {
        let env = wslenv("");
        assert_eq!(env, FORWARDED_ENV.join(":"));
        // The user's own WSLENV cannot carry the instance's credentials in.
        let env = wslenv("AGENTMUX_AUTH_KEY:AGENTMUX_LOCAL_URL/u:PATH/l:AGENTMUX_TABID");
        assert!(
            !env.contains("AGENTMUX_AUTH_KEY") && !env.contains("AGENTMUX_LOCAL_URL"),
            "{env}"
        );
        assert!(env.starts_with("PATH/l:AGENTMUX_TABID:"), "{env}");
        assert!(!wslenv_with("AGENTMUX_AUTH_KEY/p", []).contains("AGENTMUX_AUTH_KEY"));
        let env = wslenv("USERPROFILE/p:TERM/u");
        assert!(env.starts_with("USERPROFILE/p:TERM/u:"), "{env}");
        let names: Vec<&str> = env
            .split(':')
            .map(|p| p.split('/').next().unwrap())
            .collect();
        assert_eq!(
            names.iter().filter(|n| **n == "TERM").count(),
            1,
            "TERM once: {env}"
        );
        assert!(!env.contains("AGENTMUX_AUTH_KEY") && !env.contains("AGENTMUX_LOCAL_URL"));
    }

    #[test]
    fn an_agent_command_carries_its_own_env_and_the_gh_guard_but_no_agentmux_secret() {
        let env = wslenv_with(
            "GH_CONFIG_DIR/u:USERPROFILE/p",
            [
                "RUST_LOG",
                "AGENTMUX_AUTH_KEY",
                "AGENTMUX_LOCAL_URL",
                "AGENTMUX_JEKT_KEY",
                "AGENTMUX_BLOCKID",
                "AGENTMUX_AGENT_ID",
                "GH_CONFIG_DIR",
                "WSLENV",
                "BAD:NAME",
                "",
            ],
        );
        let entries: Vec<&str> = env.split(':').collect();
        assert!(entries.contains(&"RUST_LOG"), "{env}");
        assert!(
            entries.contains(&"AGENTMUX_AGENT_ID"),
            "a pane's configured identity crosses: {env}"
        );
        assert!(
            entries.contains(&"USERPROFILE/p"),
            "the user's own entries stay: {env}"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.starts_with("GH_CONFIG_DIR"))
                .collect::<Vec<_>>(),
            [&"GH_CONFIG_DIR/p"],
            "the guard crosses as a distro path, whatever the user's flag was: {env}"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.starts_with("AGENTMUX_BLOCKID"))
                .count(),
            1
        );
        for secret in [
            "AGENTMUX_AUTH_KEY",
            "AGENTMUX_LOCAL_URL",
            "AGENTMUX_JEKT_KEY",
            "WSLENV",
            "BAD",
        ] {
            assert!(
                !entries.iter().any(|e| e.starts_with(secret)),
                "{secret} in {env}"
            );
        }
    }
}
