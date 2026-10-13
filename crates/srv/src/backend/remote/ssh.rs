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
///
/// On Windows a native client that can force askpass (OpenSSH 8.4+) comes
/// first, before an MSYS2 or Cygwin build on `PATH` (Git for Windows' own
/// `usr/bin/ssh.exe`, there when AgentMux is started from Git Bash): that
/// build resolves names with its own POSIX resolver, which can't find a
/// machine on the local network by name ("Could not resolve hostname"), and
/// doesn't use Windows' ssh-agent. A native client older than 8.4 is not
/// preferred: an agent's ssh relies on `SSH_ASKPASS_REQUIRE=force`
/// (askpass.rs) to keep its prompts out of the agent's terminal. With no such
/// client, the order is the plain one above. For anything an agent can drive;
/// the user's own pane uses [`binary_for_user`].
pub fn binary() -> Option<PathBuf> {
    pick(on_path(), windows_client(), |p| {
        !cfg!(windows) || (!is_posix_layer_build(p) && forces_askpass(p))
    })
}

/// [`binary`] for the user's own interactive ssh pane, whose prompts are the
/// user's to answer in it: a native client is preferred whatever its version
/// (Windows 10's inbox OpenSSH is 8.1), still for its name resolution.
pub fn binary_for_user() -> Option<PathBuf> {
    pick(on_path(), windows_client(), |p| !cfg!(windows) || !is_posix_layer_build(p))
}

fn on_path() -> impl Iterator<Item = PathBuf> {
    which::which_all("ssh").into_iter().flatten()
}

/// Windows' own OpenSSH client, if installed; `None` elsewhere.
fn windows_client() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Some(Path::new(&root).join(r"System32\OpenSSH\ssh.exe")).filter(|p| p.is_file())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// [`binary`]'s order: the first `preferred` ssh on `PATH`, then Windows'
/// own client if preferred, else the first on `PATH`, else Windows' client.
fn pick(
    on_path: impl IntoIterator<Item = PathBuf>,
    windows_client: Option<PathBuf>,
    preferred: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let on_path: Vec<PathBuf> = on_path.into_iter().collect();
    if let Some(p) = on_path.iter().find(|p| preferred(p)) {
        return Some(p.clone());
    }
    if let Some(w) = windows_client.as_ref().filter(|w| preferred(w)) {
        return Some(w.clone());
    }
    on_path.into_iter().next().or(windows_client)
}

/// Whether `ssh` is OpenSSH 8.4 or newer, which honours
/// `SSH_ASKPASS_REQUIRE=force`; asked once per binary (`ssh -V`).
fn forces_askpass(ssh: &Path) -> bool {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, bool>>> =
        std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(&ok) = seen.lock().unwrap_or_else(|e| e.into_inner()).get(ssh) {
        return ok;
    }
    let mut cmd = std::process::Command::new(ssh);
    cmd.arg("-V").stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        cmd.no_window();
    }
    let ok = cmd
        .output()
        .ok()
        .and_then(|out| openssh_version(&String::from_utf8_lossy(&out.stderr)))
        .is_some_and(|v| v >= (8, 4));
    seen.lock().unwrap_or_else(|e| e.into_inner()).insert(ssh.to_path_buf(), ok);
    ok
}

/// `(major, minor)` from `ssh -V`'s "OpenSSH_9.5p2, …" or
/// "OpenSSH_for_Windows_8.1p1, …".
fn openssh_version(banner: &str) -> Option<(u32, u32)> {
    let rest = &banner[banner.find("OpenSSH")?..];
    let start = rest.find(|c: char| c.is_ascii_digit())?;
    let mut parts = rest[start..].split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// An `ssh.exe` built for MSYS2 or Cygwin: its runtime DLL sits beside it.
/// Never true off Windows.
fn is_posix_layer_build(ssh: &Path) -> bool {
    cfg!(windows)
        && ssh
            .parent()
            .is_some_and(|dir| dir.join("msys-2.0.dll").is_file() || dir.join("cygwin1.dll").is_file())
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
/// connection-sharing sockets live (`ControlMaster`), so a second pane on the
/// same connection reuses the first one's and asks for nothing.
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
    // Which AgentMux this is (`pane_env::build_info`) rides along with
    // TERM_PROGRAM. sshd passes a SetEnv name on only where its AcceptEnv
    // lists it (`AcceptEnv AGENTMUX_*`), and drops it quietly otherwise. Not
    // prefixed to the remote command instead: durable panes send their own
    // control commands through here, to hosts that may not run a POSIX shell.
    let build_info = crate::backend::pane_env::build_info();
    let set_env = std::iter::once("TERM_PROGRAM=agentmux".to_string())
        .chain(build_info.iter().map(|(k, v)| format!("{k}={v}")))
        .collect::<Vec<_>>()
        .join(" ");
    opt(format!("SetEnv={set_env}"));
    if let Some(dir) = control_dir {
        opt("ControlMaster=auto".into());
        opt(control_path_option(dir, dest));
        opt("ControlPersist=10m".into());
    }
    push_destination(&mut args, dest);
    if let Some(remote) = remote_command(cmd, cmd_args, cwd) {
        args.push(remote);
    }
    args
}

/// Name the pane in [`launch`]'s `SetEnv` too (`AGENTMUX_BLOCKID`), so a host
/// that accepts it (`AcceptEnv AGENTMUX_*`) lets the pane's stats badge find
/// its shell there (`PaneProcs`). Dropped quietly by any other host, like the
/// rest of that `SetEnv`. A block id is AgentMux's own UUID; anything with
/// another character is left out rather than quoted.
pub fn set_pane_env(args: &mut [String], block_id: &str) {
    if block_id.is_empty() || !block_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return;
    }
    if let Some(set_env) = args.iter_mut().find(|a| a.starts_with("SetEnv=")) {
        set_env.push_str(&format!(" AGENTMUX_BLOCKID={block_id}"));
    }
}

/// The port, if any, then `--` and the destination: `--` ends ssh's options,
/// so the destination can never be read as one (conn.rs also refuses a
/// leading `-`).
fn push_destination(args: &mut Vec<String>, dest: &SshDest) {
    if let Some(port) = dest.port {
        args.push("-p".into());
        args.push(port.to_string());
    }
    args.push("--".into());
    args.push(dest.destination.clone());
}

/// Hex characters of the connection name's hash in a socket's name.
const NAME_HASH_HEX: usize = 8;

/// The longest socket name ssh binds in a control dir: the name's hash, `-`,
/// the 40-hex `%C`, and while binding `.` plus 16 random characters.
#[cfg(any(unix, test))]
const SOCKET_NAME_MAX: usize = NAME_HASH_HEX + 1 + 40 + 17;

/// `ControlPath=` for `dest`'s shared connection in `dir`: `<name hash>-%C`.
/// ssh's `%C` hashes the user, host and port ssh resolves, not the name it was
/// given, so two aliases for one account (one with a key, one password-only)
/// shared whichever connection logged in first, and the password-only one got
/// in without asking (#4513). The connection's own name keeps them apart; `%C`
/// stays so an alias pointed elsewhere gets a new connection.
fn control_path_option(dir: &Path, dest: &SshDest) -> String {
    use sha2::{Digest, Sha256};
    let name = hex::encode(&Sha256::digest(dest.name().as_bytes())[..NAME_HASH_HEX / 2]);
    format!("ControlPath={}", dir.join(format!("{name}-%C")).display())
}

/// What to ask a shared connection (`ssh -O`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Answers (exit 0) while the connection is up, which it is only once the
    /// user has logged in: at a password prompt there is none yet.
    Check,
    /// Close it: every session on it ends, with ssh's exit 255.
    Exit,
}

/// The `ssh` arguments asking `dest`'s shared connection in `dir` to `op`:
/// the same socket a pane's [`launch`] shares.
pub fn control(dest: &SshDest, dir: &Path, op: Control) -> Vec<String> {
    let word = match op {
        Control::Check => "check",
        Control::Exit => "exit",
    };
    let mut args = vec!["-o".into(), control_path_option(dir, dest), "-O".into(), word.into()];
    push_destination(&mut args, dest);
    args
}

/// The `ssh` arguments for one command run without a terminal (an agent's
/// `Shell`): [`launch`]'s, with `-T` in place of `-tt`, since its output is
/// read, not a session shown.
pub fn launch_exec(
    dest: &SshDest,
    cmd: &str,
    cwd: &str,
    control_dir: Option<&Path>,
) -> Vec<String> {
    let mut args = launch(dest, cmd, &[], cwd, control_dir);
    args[0] = "-T".into();
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

/// Whether `ssh` ending with `code` means it had no connection, or lost it,
/// rather than that the remote shell ended (`exit` included). A session on a
/// shared connection that was closed or died ends this way too.
pub fn connection_lost(code: u32) -> bool {
    code == EXIT_CONNECTION_FAILED
}

/// Whether a pane's `ssh`, ended with `code`, lost its connection (or never
/// had one). `stopped_by_srv`: srv ended it itself (the pane closed, or
/// restarted), and ssh then exits 255 too (closing its pty does, and a session
/// on a shared connection does on SIGTERM), which is no lost connection.
pub fn pane_lost_connection(code: Option<u32>, stopped_by_srv: bool) -> bool {
    !stopped_by_srv && code.is_some_and(connection_lost)
}

/// What the connection's status says when a pane's `ssh` lost it.
pub fn lost_message(dest: &str) -> String {
    format!("ssh could not connect to {dest}, or the connection dropped (see the terminal for ssh's own message)")
}

/// The line a plain SSH pane shows, under ssh's own reason, when it has no
/// connection: the pane stays, and Enter starts ssh again.
pub fn lost_note(conn: &str) -> String {
    format!("No connection to {conn}: it dropped, or could not be made. Press Enter to try again.")
}

/// Where `ControlMaster` sockets go (macOS and Linux; Windows' OpenSSH has no
/// connection sharing): a private, owner-only directory, since anyone who can
/// open a socket there can use the connection. `None` (no sharing; ssh still
/// connects) if it can't be made so, or a socket in it would be too long.
///
/// Not under `config_home`: a channel's config dir is deep
/// (`/Users/<name>/.agentmux/channels/stable/config`), and a Unix socket path
/// has to fit in `sun_path`, 104 bytes on macOS and 108 on Linux. Under it,
/// ssh failed with "too long for Unix domain socket" right after logging in,
/// on every macOS install. So: the launcher's short runtime dir
/// (`$XDG_RUNTIME_DIR/agentmux`, else `/tmp/agentmux-<uid>`, as
/// `launcher::ipc::ipc_socket_dir_path`) and one `ssh-<hash>` per channel.
#[cfg(unix)]
pub fn control_dir(config_home: &Path) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .map(|d| d.join("agentmux"))
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/agentmux-{}", unsafe { libc::getuid() })));
    control_dir_in(&base, config_home)
}

#[cfg(not(unix))]
pub fn control_dir(_config_home: &Path) -> Option<PathBuf> {
    None
}

/// [`control_dir`] for this srv's channel: where every `ssh` it runs to a host
/// (panes, agents' commands, the helper, files) shares one connection. `None`
/// where ssh can't share connections, and then there is nothing to ask `-O`.
pub fn shared_control_dir() -> Option<PathBuf> {
    control_dir(&crate::backend::base::get_mux_config_dir())
}

/// [`control_dir`] under `base`: `base/ssh-<8 hex of config_home>`, each
/// level owner-only. A test can pass its own `base`.
#[cfg(unix)]
fn control_dir_in(base: &Path, config_home: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let channel = hex::encode(&Sha256::digest(config_home.as_os_str().as_encoded_bytes())[..4]);
    let dir = base.join(format!("ssh-{channel}"));
    private_dir(base)?;
    private_dir(&dir)?;
    socket_fits(&dir).then_some(dir)
}

/// Make `dir` (mode `0700`), or accept it if it already exists as a real
/// directory owned by this user that no one else can enter. `/tmp` is shared:
/// someone else could have made the directory, or a symlink in its place.
#[cfg(unix)]
fn private_dir(dir: &Path) -> Option<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let meta = std::fs::symlink_metadata(dir).ok()?;
    let ours = meta.file_type().is_dir() && meta.uid() == unsafe { libc::getuid() };
    (ours && meta.permissions().mode() & 0o077 == 0).then_some(())
}

/// Whether ssh can bind a socket in `dir`: the longest path it binds is
/// `dir/<name hash>-<40-hex %C>.<16 random>` ([`SOCKET_NAME_MAX`]), and it has
/// to fit in `sun_path` with its NUL.
#[cfg(any(unix, test))]
fn socket_fits(dir: &Path) -> bool {
    let sun_path = if cfg!(target_os = "linux") { 108 } else { 104 };
    dir.as_os_str().len() + 1 + SOCKET_NAME_MAX < sun_path
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preferred ssh on PATH first, then Windows' client if preferred;
    /// with neither, the first on PATH, as before.
    #[test]
    fn a_preferred_ssh_comes_first_and_otherwise_the_order_is_the_plain_one() {
        let p = |s: &str| PathBuf::from(s);
        let preferred = |path: &Path| path.starts_with("/native");
        assert_eq!(
            pick([p("/git/ssh"), p("/native/ssh")], Some(p("/win/ssh")), preferred),
            Some(p("/native/ssh"))
        );
        let win_ok = |path: &Path| path.starts_with("/win");
        assert_eq!(pick([p("/git/ssh")], Some(p("/win/ssh")), win_ok), Some(p("/win/ssh")));
        // An old native client (no forced askpass) isn't preferred: the
        // first on PATH, as before this change.
        let none = |_: &Path| false;
        assert_eq!(pick([p("/git/ssh"), p("/old/ssh")], Some(p("/win/ssh")), none), Some(p("/git/ssh")));
        assert_eq!(pick(Vec::<PathBuf>::new(), Some(p("/win/ssh")), none), Some(p("/win/ssh")));
        assert_eq!(pick(Vec::<PathBuf>::new(), None, none), None);
    }

    #[test]
    fn openssh_versions_are_read_from_the_banner() {
        assert_eq!(openssh_version("OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2"), Some((9, 5)));
        assert_eq!(openssh_version("OpenSSH_for_Windows_8.1p1, LibreSSL 3.0.2"), Some((8, 1)));
        assert_eq!(openssh_version("OpenSSH_10.2p1, OpenSSL 3.5.4 30 Sep 2025"), Some((10, 2)));
        assert_eq!(openssh_version("Dropbear v2022.83"), None);
        assert!((8, 1) < (8, 4) && (10, 2) >= (8, 4));
    }

    /// Git for Windows' and Cygwin's ssh are passed over for the native one.
    #[cfg(windows)]
    #[test]
    fn an_msys_or_cygwin_ssh_is_told_from_a_native_one() {
        let dir = tempfile::tempdir().unwrap();
        let (msys, cygwin, native) = (dir.path().join("msys"), dir.path().join("cygwin"), dir.path().join("native"));
        for d in [&msys, &cygwin, &native] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("ssh.exe"), b"").unwrap();
        }
        std::fs::write(msys.join("msys-2.0.dll"), b"").unwrap();
        std::fs::write(cygwin.join("cygwin1.dll"), b"").unwrap();
        assert!(is_posix_layer_build(&msys.join("ssh.exe")));
        assert!(is_posix_layer_build(&cygwin.join("ssh.exe")));
        assert!(!is_posix_layer_build(&native.join("ssh.exe")));
    }

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
                build_set_env().as_str(),
                "--",
                "asaf@area54"
            ]
        );
    }

    fn build_set_env() -> String {
        let mut s = "SetEnv=TERM_PROGRAM=agentmux".to_string();
        for (k, v) in crate::backend::pane_env::build_info() {
            s.push_str(&format!(" {k}={v}"));
        }
        s
    }

    #[test]
    fn the_build_rides_along_without_touching_the_remote_command() {
        let args = launch(&dest("area54", None), "claude", &[], "", None);
        let set_env = args.iter().find(|a| a.starts_with("SetEnv=")).unwrap();
        assert!(set_env.starts_with("SetEnv=TERM_PROGRAM=agentmux "));
        assert!(set_env.contains(&format!("AGENTMUX_VERSION={}", env!("CARGO_PKG_VERSION"))));
        assert_eq!(args.last().unwrap(), "claude", "durable panes' control commands go through here unchanged");
    }

    #[test]
    fn a_plain_pane_names_itself_in_set_env() {
        let mut args = launch(&dest("area54", None), "", &[], "", None);
        let before = args.clone();
        set_pane_env(&mut args, "6f1c2b9e-0a4d-4c3e-9b1a-2f5e7d8c9a01");
        let set_env = args.iter().find(|a| a.starts_with("SetEnv=")).unwrap();
        assert!(set_env.ends_with(" AGENTMUX_BLOCKID=6f1c2b9e-0a4d-4c3e-9b1a-2f5e7d8c9a01"), "{set_env}");
        assert_eq!(args.len(), before.len(), "no argument added, only the SetEnv value");
        let mut odd = before.clone();
        set_pane_env(&mut odd, "x y=z");
        assert_eq!(odd, before, "an id with any other character is left out");
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

    fn control_path_opt(args: &[String]) -> &str {
        args.iter().find(|a| a.starts_with("ControlPath=")).expect("a ControlPath")
    }

    /// Two `Host` aliases for one account (one with a key, one password-only)
    /// resolve to the same user, host and port, which is all ssh's `%C`
    /// hashes: they shared whichever connection logged in first, so the
    /// password-only alias got in without asking (#4513, finding 3). The
    /// connection's own name keeps them apart.
    #[test]
    fn two_aliases_for_one_account_get_their_own_shared_connection() {
        let dir = Path::new("/tmp/agentmux-501/ssh-1a2b3c4d");
        let path = |d: &SshDest| control_path_opt(&launch(d, "", &[], "", Some(dir))).to_string();
        let (key, pw) = (dest("exp-key", None), dest("exp-pw", None));
        assert_ne!(path(&key), path(&pw));
        assert_eq!(path(&key), path(&dest("exp-key", None)), "one name, one connection");
        assert_ne!(path(&dest("u@h", Some(22))), path(&dest("u@h", Some(2222))), "the port is part of the name");
        // ssh's own %C stays in it: an alias pointed somewhere else gets a new one.
        assert!(path(&key).ends_with("-%C"), "{}", path(&key));
    }

    /// `ssh -O` reaches the very connection a pane's ssh shares: same socket,
    /// same destination and port.
    #[test]
    fn control_asks_the_connection_a_pane_shares() {
        let dir = Path::new("/tmp/agentmux-501/ssh-1a2b3c4d");
        let d = dest("tester@area54", Some(2222));
        let pane = launch(&d, "", &[], "", Some(dir));
        let check = control(&d, dir, Control::Check);
        assert_eq!(control_path_opt(&check), control_path_opt(&pane));
        assert_eq!(&check[check.len() - 6..], ["-O", "check", "-p", "2222", "--", "tester@area54"]);
        let exit = control(&d, dir, Control::Exit);
        assert_eq!(&exit[exit.len() - 6..], ["-O", "exit", "-p", "2222", "--", "tester@area54"]);
    }

    /// Exit 255 is ssh's own failure (no connection, or it dropped: also a
    /// session on a shared connection that was closed); any other code is the
    /// remote shell's, `exit` included.
    #[test]
    fn only_ssh_s_own_failure_is_a_lost_connection() {
        assert!(connection_lost(255));
        for code in [0, 1, 2, 127, 130] {
            assert!(!connection_lost(code), "{code}");
        }
        let note = lost_note("area54");
        assert!(note.contains("area54") && note.contains("Press Enter"), "{note}");
    }

    /// The longest socket path ssh binds in `dir`: `/`, the connection's 8-hex
    /// name, `-`, the 40-hex `%C`, and while binding `.` plus 16 random
    /// characters. It must stay under the OS's `sun_path` (104 bytes on macOS,
    /// 108 on Linux, NUL included).
    #[cfg(unix)]
    fn bind_path_len(dir: &Path) -> usize {
        dir.as_os_str().len() + 1 + 8 + 1 + 40 + 17
    }

    #[cfg(unix)]
    #[test]
    fn a_deep_config_dir_still_gets_a_socket_path_ssh_can_bind() {
        // A channel's config dir is deep: the stable one on macOS is
        // `/Users/<name>/.agentmux/channels/stable/config`, and every socket
        // under it was too long, so ssh failed right after logging in.
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("users-name/.agentmux/channels/local-main-b28b7a-51e8e6a3/config");
        let dir = control_dir(&deep).expect("a control dir");
        let max = if cfg!(target_os = "linux") { 108 } else { 104 };
        assert!(bind_path_len(&dir) < max, "{} bytes: {}", bind_path_len(&dir), dir.display());
        // It's in the real runtime dir: don't leave one behind per run.
        let _ = std::fs::remove_dir(&dir);
    }

    /// Under `/tmp`: macOS's own temp dir (`/var/folders/…`) is itself too
    /// long for a socket, so a dir there is refused for its length alone.
    #[cfg(unix)]
    fn short_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("am").tempdir_in("/tmp").unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn each_channel_gets_its_own_owner_only_dir() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = short_tempdir();
        let base = tmp.path().join("b");
        let (stable, local) = (Path::new("/u/.agentmux/channels/stable/config"), Path::new("/u/.agentmux/channels/local-x/config"));
        let a = control_dir_in(&base, stable).unwrap();
        assert_eq!(control_dir_in(&base, stable), Some(a.clone()));
        assert_ne!(control_dir_in(&base, local), Some(a.clone()));
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&base), mode(&a)), (0o700, 0o700));
    }

    #[cfg(unix)]
    #[test]
    fn a_base_others_can_enter_or_a_symlink_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = short_tempdir();
        let config = Path::new("/u/config");
        let open = tmp.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(control_dir_in(&open, config), None);
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(control_dir_in(&link, config), None);
    }

    #[test]
    fn a_socket_fits_only_below_sun_path() {
        let sun_path = if cfg!(target_os = "linux") { 108 } else { 104 };
        let dir = |len: usize| PathBuf::from(format!("/{}", "d".repeat(len - 1)));
        let longest = sun_path - 1 - (1 + 8 + 1 + 40 + 17);
        assert!(socket_fits(&dir(longest)));
        assert!(!socket_fits(&dir(longest + 1)));
        // The stable channel's old dir on macOS, for an 8-letter user name.
        assert!(!socket_fits(Path::new("/Users/asafebgi/.agentmux/channels/stable/config/ssh")) || cfg!(target_os = "linux"));
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
    fn a_command_without_a_terminal_is_dash_capital_t() {
        let args = launch_exec(&dest("area54", Some(22)), "make test", "~/proj", None);
        assert_eq!(args[0], "-T");
        assert!(!args.iter().any(|a| a == "-tt"));
        assert_eq!(
            &args[args.len() - 3..],
            ["--", "area54", "cd ~/proj; make test"]
        );
    }

    #[test]
    fn only_a_connection_failure_is_an_error() {
        assert!(lost_message("area54").contains("could not connect to area54"));
        assert!(pane_lost_connection(Some(255), false));
        for code in [Some(0), Some(1), Some(130), None] {
            assert!(!pane_lost_connection(code, false), "{code:?}");
        }
    }

    /// srv ending a pane's ssh itself (the pane closed, or restarted) makes it
    /// exit 255 too: closing its pty does at once, and a session on a shared
    /// connection exits 255 on SIGTERM (measured, OpenSSH 9). That is not a
    /// lost connection: no line in the pane, no error on the host.
    #[test]
    fn ssh_ended_by_srv_is_not_a_lost_connection() {
        assert!(!pane_lost_connection(Some(255), true));
    }

    /// `~` is the remote home: left for the remote shell to expand.
    #[test]
    fn a_tilde_cwd_is_the_remote_home() {
        assert_eq!(quote_path("~"), "~");
        assert_eq!(quote_path("~/"), "~/");
        assert_eq!(quote_path("~/my proj"), "~/'my proj'");
        assert_eq!(quote_path("~/src"), "~/src");
        assert_eq!(
            quote_path("/a/~b"),
            "'/a/~b'",
            "a tilde inside a path stays literal"
        );
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
