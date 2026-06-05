// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.2: launcher-owned named-pipe IPC server.
//
// Per `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §3.2 and §5,
// the launcher hosts the canonical state machine and exposes it over a
// pipe per data-dir-scoped namespace. Each subscriber (host, eventually
// frontend renderers, srv) connects, sends `Command` messages, and
// receives `Event` messages back.
//
// B.2 scope (this module): just the wire — types + accept loop +
// per-connection read/write tasks. No reducer, no events emitted yet
// (B.3 wires the reducer; B.4 pipes events back). This commit makes
// the host able to register itself with the launcher and the launcher
// to log incoming Commands. Foundation for everything else in Phase B.

pub mod server;

// Wire types live in agentmux-common::ipc so the host (client) and
// launcher (server) compile against one definition. Phase F.7
// cleanup audit: the prior `pub use {Command, Event}` re-exports
// from this module had no consumers — every reference uses the
// canonical `agentmux_common::ipc` path directly. Removed to keep
// the launcher's public surface honest.
pub use server::run_ipc_server;

/// Construct the IPC endpoint path for a given data-dir hash.
///
/// Windows: a named-pipe path `\\.\pipe\agentmux-{hash16}\command`.
/// Unix:    a Unix-domain-socket path under `$XDG_RUNTIME_DIR/agentmux/`
///          (fallback `/tmp/agentmux-{uid}/`), file name
///          `{hash16}.sock`. The directory is created with 0700 perms
///          and ownership = the user, so cross-user squatting in `/tmp`
///          can't happen.
///
/// Per-data-dir scoping preserves multi-instance support per
/// `CLAUDE.md`: different portable folders / installed versions
/// → different data dirs → different hashes → distinct endpoints.
/// Two launchers pointing at the SAME data dir collide at bind time,
/// which is also the single-instance signal Phase B.6 / A1.6 use.
#[cfg(target_os = "windows")]
pub fn pipe_name(data_dir_hash16: &str) -> String {
    format!("\\\\.\\pipe\\agentmux-{}\\command", data_dir_hash16)
}

#[cfg(unix)]
pub fn pipe_name(data_dir_hash16: &str) -> String {
    format!(
        "{}/{}.sock",
        ipc_socket_dir().display(),
        data_dir_hash16
    )
}

/// Phase E.1b — srv-side pipe path. Same data-dir hash as the
/// launcher pipe (multi-instance scoping is identical), different
/// leaf name. Both pipes coexist; subscribers connect to whichever
/// reducer they need.
#[cfg(target_os = "windows")]
pub fn srv_pipe_name(data_dir_hash16: &str) -> String {
    format!("\\\\.\\pipe\\agentmux-{}\\srv-command", data_dir_hash16)
}

#[cfg(unix)]
pub fn srv_pipe_name(data_dir_hash16: &str) -> String {
    format!(
        "{}/{}-srv.sock",
        ipc_socket_dir().display(),
        data_dir_hash16
    )
}

/// Directory under which all launcher Unix sockets live. Created with
/// 0700 perms so cross-user squatting can't happen.
///
/// Resolution order (A1.1 of SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_
/// 2026_06_05):
///   1. `$XDG_RUNTIME_DIR/agentmux/` — preferred; tmpfs, per-user,
///      automatically cleaned up by systemd-logind on session end.
///   2. `/tmp/agentmux-{uid}/` — fallback for environments without
///      a systemd user manager. UID in the path so users can't
///      collide.
///
/// The directory is created on first call (idempotent). On the rare
/// failure path (read-only `/tmp`, exotic chroot) we fall back to
/// `/tmp` with the launcher pid as a discriminator. That fallback
/// is intentionally non-strict — the caller's bind will fail and
/// the launcher exits cleanly with a clear error.
#[cfg(unix)]
pub fn ipc_socket_dir() -> std::path::PathBuf {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};

    let dir = if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let mut p = std::path::PathBuf::from(runtime);
        p.push("agentmux");
        p
    } else {
        let uid = unsafe { libc::getuid() };
        std::path::PathBuf::from(format!("/tmp/agentmux-{}", uid))
    };

    // Security (codex P1 on PR #1288): the /tmp fallback is reachable
    // by any local user. If an attacker pre-creates the path with a
    // permissive mode, the naive `create_dir_all` + best-effort chmod
    // pattern silently lets us bind the launcher socket inside an
    // attacker-controlled directory, where they can connect as the
    // first IPC client and impersonate the host. Defense:
    //   1. Create the directory atomically with mode 0700.
    //   2. If the directory already exists, stat it and refuse to
    //      proceed unless: (a) it's a real directory (not a symlink),
    //      (b) it's owned by the current uid, (c) its mode masks 0700.
    //   3. On refusal, abort the launcher with a clear error rather
    //      than continuing to bind in an unsafe location.
    let our_uid = unsafe { libc::getuid() };
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) => {
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                eprintln!(
                    "AgentMux refusing to start: IPC socket dir {} is a symlink (potential squatting attack).",
                    dir.display()
                );
                std::process::exit(2);
            }
            if !file_type.is_dir() {
                eprintln!(
                    "AgentMux refusing to start: IPC socket path {} exists but is not a directory.",
                    dir.display()
                );
                std::process::exit(2);
            }
            if meta.uid() != our_uid {
                eprintln!(
                    "AgentMux refusing to start: IPC socket dir {} is owned by uid {}, not our uid {} (potential squatting attack).",
                    dir.display(),
                    meta.uid(),
                    our_uid
                );
                std::process::exit(2);
            }
            // Tighten mode if it's looser than 0700. Use the chmod
            // (rather than a fresh create) so we never widen perms.
            let mode = meta.mode() & 0o777;
            if mode & 0o077 != 0 {
                use std::os::unix::fs::PermissionsExt as _;
                if let Err(e) = std::fs::set_permissions(
                    &dir,
                    std::fs::Permissions::from_mode(0o700),
                ) {
                    eprintln!(
                        "AgentMux refusing to start: IPC socket dir {} has mode {:o} (group/other accessible) and chmod 0700 failed: {}.",
                        dir.display(),
                        mode,
                        e
                    );
                    std::process::exit(2);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Atomically create with mode 0700. `DirBuilder::mode(0700)`
            // applies via mkdir(2)'s `mode` arg under our umask, so the
            // directory's actual perms are the requested mode masked by
            // the umask. We set the mode explicitly via `set_permissions`
            // immediately after creation as a belt + suspenders against
            // any inherited umask quirk.
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            if let Err(e) = builder.create(&dir) {
                eprintln!(
                    "AgentMux refusing to start: failed to create IPC socket dir {}: {}.",
                    dir.display(),
                    e
                );
                std::process::exit(2);
            }
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Err(e) => {
            eprintln!(
                "AgentMux refusing to start: failed to stat IPC socket dir {}: {}.",
                dir.display(),
                e
            );
            std::process::exit(2);
        }
    }

    dir
}
