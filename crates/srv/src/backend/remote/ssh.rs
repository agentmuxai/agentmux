// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SSH terminals (P2 of SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! A pane on an SSH connection runs the system `ssh` in its PTY (spec §3.1):
//! the user's own `~/.ssh/config`, keys, agent and known hosts apply, and
//! `ssh` asks for a password, a passphrase or a new host key in the terminal
//! itself, where the user answers it. The askpass bridge (§5.3), which an
//! agent's non-interactive use needs, comes later.

use std::path::{Path, PathBuf};

use super::conn::SshDest;

/// `ssh` exits 255 when it could not connect, could not authenticate, or the
/// connection dropped; any other code is the remote command's own.
pub const EXIT_CONNECTION_FAILED: u32 = 255;

/// The `ssh` to run: the first on `PATH`, then (Windows) the OpenSSH client
/// Windows installs, which an inherited `PATH` can lack.
pub fn binary() -> Option<PathBuf> {
    if let Ok(p) = which::which("ssh") {
        return Some(p);
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let p = Path::new(&root).join(r"System32\OpenSSH\ssh.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// What to say when there is no `ssh`, with how to get it on this platform.
pub fn missing_binary_message() -> String {
    let how = if cfg!(windows) {
        "install the OpenSSH Client (Settings > System > Optional features)"
    } else if cfg!(target_os = "macos") {
        "macOS includes it at /usr/bin/ssh; check that PATH reaches it"
    } else {
        "install your distribution's openssh-client package"
    };
    format!("SSH terminals need the ssh command, which was not found: {how}")
}

/// The `ssh` arguments for a pane. `control_dir`, on macOS and Linux, is where
/// connection-sharing sockets live (`ControlMaster`), so a second pane to the
/// same host reuses the first one's connection and asks for nothing.
pub fn launch(
    dest: &SshDest,
    cmd: &str,
    cmd_args: &[String],
    cwd: &str,
    control_dir: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-tt".into()];
    let mut opt = |o: String| {
        args.push("-o".into());
        args.push(o);
    };
    // A dead link ends ssh within about 45 s instead of the TCP timeout (§7.5).
    opt("ServerAliveInterval=15".into());
    opt("ServerAliveCountMax=3".into());
    opt("SetEnv=TERM_PROGRAM=agentmux".into());
    if let Some(dir) = control_dir {
        opt("ControlMaster=auto".into());
        opt(format!("ControlPath={}", dir.join("%C").display()));
        opt("ControlPersist=10m".into());
    }
    if let Some(port) = dest.port {
        args.push("-p".into());
        args.push(port.to_string());
    }
    // `--` ends ssh's options: the destination can never be read as one
    // (conn.rs also refuses a leading `-`).
    args.push("--".into());
    args.push(dest.destination.clone());
    if let Some(remote) = remote_command(cmd, cmd_args, cwd) {
        args.push(remote);
    }
    args
}

/// The command line `ssh` hands to the remote user's login shell: the pane's
/// command, run in `cwd` when one is set, or for a plain pane with a cwd, the
/// login shell started there. `None` for a plain pane with no cwd: ssh then
/// starts the login shell itself.
///
/// The remote shell parses this, whatever it is (bash, zsh, fish), so only
/// single-quoted words, a leading `~/`, `;`, `exec` and `"$SHELL"` appear, which
/// all of them read the same way.
pub fn remote_command(cmd: &str, cmd_args: &[String], cwd: &str) -> Option<String> {
    let run = if cmd.is_empty() {
        None
    } else if cmd_args.is_empty() {
        // A command line, as a local pane runs it through `sh -c`.
        Some(cmd.to_string())
    } else {
        Some(
            std::iter::once(cmd)
                .chain(cmd_args.iter().map(String::as_str))
                .map(quote)
                .collect::<Vec<_>>()
                .join(" "),
        )
    };
    // A failed `cd` prints its own error and the shell goes on, in the
    // remote home: shown, not hidden.
    let cd = (!cwd.trim().is_empty()).then(|| format!("cd {}; ", quote_path(cwd)));
    match (cd, run) {
        (None, None) => None,
        (None, Some(run)) => Some(run),
        (Some(cd), Some(run)) => Some(format!("{cd}{run}")),
        (Some(cd), None) => Some(format!("{cd}exec \"$SHELL\" -l")),
    }
}

/// [`quote`] for a remote path, leaving a leading `~` or `~/` unquoted so the
/// remote shell expands it to the remote home (OSC 7 and users write it).
pub fn quote_path(path: &str) -> String {
    if path == "~" {
        return "~".to_string();
    }
    match path.strip_prefix("~/") {
        Some("") => "~/".to_string(),
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(path),
    }
}

/// One word for a POSIX shell: single-quoted, with `'` written as `'\''`.
pub fn quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
    {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// What a pane says when its `ssh` ended: why, for a connection failure, and
/// nothing for an ordinary exit (the remote shell ended).
pub fn exit_message(dest: &str, code: u32) -> Option<String> {
    (code == EXIT_CONNECTION_FAILED).then(|| {
        format!("ssh could not connect to {dest}, or the connection dropped (see the terminal for ssh's own message)")
    })
}

/// Where `ControlMaster` sockets go (macOS and Linux; Windows' OpenSSH has no
/// connection sharing): a private directory under AgentMux's config, created
/// owner-only, since anyone who can open a socket there can use the
/// connection. `None` if it cannot be made so.
pub fn control_dir(config_home: &Path) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    let dir = config_home.join("ssh");
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dest(d: &str, port: Option<u16>) -> SshDest {
        SshDest {
            destination: d.to_string(),
            port,
        }
    }

    #[test]
    fn a_plain_pane_is_the_login_shell_with_keepalives() {
        let args = launch(&dest("asaf@area54", None), "", &[], "", None);
        assert_eq!(
            args,
            [
                "-tt",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-o",
                "SetEnv=TERM_PROGRAM=agentmux",
                "--",
                "asaf@area54"
            ]
        );
    }

    #[test]
    fn a_port_and_connection_sharing_come_before_the_destination() {
        let args = launch(
            &dest("area54", Some(2222)),
            "",
            &[],
            "",
            Some(Path::new("/home/u/.agentmux/ssh")),
        );
        let at = |s: &str| args.iter().position(|a| a == s).unwrap();
        assert!(args.iter().any(|a| a == "ControlMaster=auto"));
        assert!(args
            .iter()
            .any(|a| a.starts_with("ControlPath=") && a.ends_with("%C")));
        assert_eq!(args[at("-p") + 1], "2222");
        assert!(at("-p") < at("--") && at("--") + 1 == at("area54"));
        assert_eq!(args.last().unwrap(), "area54");
    }

    #[test]
    fn a_command_and_a_cwd_become_one_remote_command_line() {
        assert_eq!(remote_command("", &[], ""), None);
        assert_eq!(
            remote_command("make test", &[], ""),
            Some("make test".into())
        );
        assert_eq!(
            remote_command("", &[], "/srv/my app"),
            Some(r#"cd '/srv/my app'; exec "$SHELL" -l"#.into())
        );
        assert_eq!(
            remote_command("htop", &["-d".into(), "10".into()], "/tmp"),
            Some("cd /tmp; htop -d 10".into())
        );
        assert_eq!(
            remote_command("echo", &["it's $HOME".into()], ""),
            Some(r"echo 'it'\''s $HOME'".into())
        );
        let args = launch(&dest("h", None), "uptime", &[], "/tmp", None);
        assert_eq!(&args[args.len() - 3..], ["--", "h", "cd /tmp; uptime"]);
    }

    #[test]
    fn only_a_connection_failure_is_an_error() {
        assert!(exit_message("area54", 255)
            .unwrap()
            .contains("could not connect to area54"));
        assert_eq!(exit_message("area54", 0), None);
        assert_eq!(exit_message("area54", 1), None);
        assert_eq!(exit_message("area54", 130), None);
    }

    /// `~` is the remote home: left for the remote shell to expand.
    #[test]
    fn a_tilde_cwd_is_the_remote_home() {
        assert_eq!(quote_path("~"), "~");
        assert_eq!(quote_path("~/"), "~/");
        assert_eq!(quote_path("~/my proj"), "~/'my proj'");
        assert_eq!(quote_path("~/src"), "~/src");
        assert_eq!(quote_path("/a/~b"), "'/a/~b'", "a tilde inside a path stays literal");
        assert_eq!(quote_path("~bob/x"), "'~bob/x'", "only the user's own home");
        assert_eq!(
            remote_command("", &[], "~/proj"),
            Some(r#"cd ~/proj; exec "$SHELL" -l"#.into())
        );
    }

    #[test]
    fn quoting_leaves_plain_words_alone() {
        assert_eq!(quote("src/main.rs"), "src/main.rs");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("$(rm -rf ~)"), "'$(rm -rf ~)'");
    }
}
