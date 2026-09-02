// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.2: launcher-owned named-pipe IPC server.
//
// Per `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §3.2 and §5,
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
    // PURE — no filesystem mutation. `ipc_socket_dir_path` returns
    // the path string without creating or validating the directory.
    // Callers that need the directory to actually exist + be safe
    // (only the launcher's startup path needs that) must call
    // `ensure_ipc_socket_dir()` separately before binding/connecting.
    //
    // Reagent P2 on PR #1288: pipe_name on Windows is a pure string
    // formatter; making the Unix variant filesystem-mutating + able
    // to std::process::exit was a footgun for any future read-only
    // inspector (e.g. the planned Linux `--diag` port) that calls
    // pipe_name purely to locate the socket.
    format!(
        "{}/{}.sock",
        ipc_socket_dir_path().display(),
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
    // PURE — see comment on `pipe_name` above.
    format!(
        "{}/{}-srv.sock",
        ipc_socket_dir_path().display(),
        data_dir_hash16
    )
}

/// Pure computation of the IPC socket directory path. No I/O, no
/// process-exit. Used by `pipe_name` / `srv_pipe_name` so they
/// behave like their Windows counterparts (pure string formatters).
///
/// Resolution order (A1.1 of SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_
/// 2026_06_05):
///   1. `$XDG_RUNTIME_DIR/agentmux/` — preferred.
///   2. `/tmp/agentmux-{uid}/` — fallback.
#[cfg(unix)]
pub fn ipc_socket_dir_path() -> std::path::PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let mut p = std::path::PathBuf::from(runtime);
        p.push("agentmux");
        p
    } else {
        let uid = unsafe { libc::getuid() };
        std::path::PathBuf::from(format!("/tmp/agentmux-{}", uid))
    }
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
/// Security (codex P1 + reagent P1 on #1288). The `/tmp` fallback path
/// is reachable by every local user. The resolver must close every
/// TOCTOU window:
///
///   * If the dir doesn't exist, create it NON-RECURSIVELY (so a race
///     between our stat and our create fails with `AlreadyExists`,
///     not silently succeeds the way `create_dir_all` would).
///   * Immediately after a successful create, RE-STAT and verify
///     ownership + mode. A `create_dir(0700)` syscall under a 0022
///     umask still produces a 0700 dir; this re-stat is belt-and-
///     suspenders against the unlikely case where the dir we just
///     created has been replaced by an attacker between mkdir and
///     re-stat.
///   * If the dir already exists, `symlink_metadata` (NOT `metadata`
///     — we must not follow symlinks) and refuse to proceed unless
///     it's a real directory owned by our uid with mode masking 0700.
///   * Refusal = `std::process::exit(2)` with a clear error. We do
///     NOT try to recover by picking a different path; that would
///     enlarge the trust boundary.
/// Ensure the IPC socket directory exists with safe ownership + mode.
///
/// This is the SIDE-EFFECTING half of the path/ensure split — callers
/// that need the directory to actually exist (only the launcher's
/// startup path) call this BEFORE binding/connecting. Read-only
/// inspectors (e.g. future `--diag` tools) use `ipc_socket_dir_path`
/// directly and never reach this code.
///
/// Returns the validated directory path on success. Calls
/// `std::process::exit(2)` on any verification failure — we do NOT
/// recover by picking a different path; that would enlarge the trust
/// boundary.
///
/// (Reagent P2 on PR #1288: previously this logic was inside the
/// `ipc_socket_dir` function which `pipe_name` called, making
/// `pipe_name` a filesystem-mutating + process-exiting function on
/// Unix while it's a pure string formatter on Windows. Split into a
/// pure `ipc_socket_dir_path` + this ensure-step.)
#[cfg(unix)]
pub fn ensure_ipc_socket_dir() -> std::path::PathBuf {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};

    let dir = ipc_socket_dir_path();

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

    // Validate an existing directory (whether we found it via the
    // initial stat or via a same-user-race AlreadyExists from create).
    // Returns Ok on safe-to-use, exits(2) with a clear error otherwise.
    // Side-effect: tightens mode to 0700 if it's looser.
    let validate_existing = |meta: std::fs::Metadata| {
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
    };

    match std::fs::symlink_metadata(&dir) {
        Ok(meta) => validate_existing(meta),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Non-recursive create with mode 0700. `recursive(true)`
            // (= create_dir_all semantics) would return Ok even if an
            // attacker pre-created the directory in the race window
            // between our `symlink_metadata` NotFound and this call —
            // we'd then bind our socket inside attacker-controlled
            // space, exactly the squatting attack this resolver is
            // here to prevent. Non-recursive create fails with
            // `AlreadyExists` in that race; we treat it as fatal.
            //
            // For the XDG path (`$XDG_RUNTIME_DIR/agentmux`), the
            // parent (`/run/user/{uid}`) is created by systemd-logind
            // and always exists. For the `/tmp` fallback, `/tmp` is
            // a standard system directory and always exists. So a
            // non-recursive create is safe; it only fails when the
            // parent is missing (which is itself a sign something
            // weird is going on and we should abort).
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(false).mode(0o700);
            match builder.create(&dir) {
                Ok(()) => {
                    // Belt-and-suspenders post-create verification.
                    // mkdir(2) is atomic; the dir we just created is
                    // OURS at this instant. Re-stat to defend against
                    // any future refactor that loosens the flags.
                    match std::fs::symlink_metadata(&dir) {
                        Ok(m) => validate_existing(m),
                        Err(stat_err) => {
                            eprintln!(
                                "AgentMux refusing to start: post-create stat of {} failed: {}.",
                                dir.display(),
                                stat_err
                            );
                            std::process::exit(2);
                        }
                    }
                }
                Err(create_err)
                    if create_err.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    // Legitimate same-user race (codex P2 on PR #1288):
                    // a concurrent launcher created the dir between our
                    // initial NotFound and this create. NOT necessarily
                    // a squatting attack — re-stat + run the same
                    // owner/mode/symlink validation we'd run on a
                    // pre-existing dir. If validation passes, the
                    // concurrent launcher created a safe dir for us
                    // both and we can proceed.
                    match std::fs::symlink_metadata(&dir) {
                        Ok(m) => validate_existing(m),
                        Err(stat_err) => {
                            eprintln!(
                                "AgentMux refusing to start: re-stat after AlreadyExists race on {} failed: {}.",
                                dir.display(),
                                stat_err
                            );
                            std::process::exit(2);
                        }
                    }
                }
                Err(create_err) => {
                    eprintln!(
                        "AgentMux refusing to start: failed to create IPC socket dir {} (mode 0700, non-recursive): {}.",
                        dir.display(),
                        create_err
                    );
                    std::process::exit(2);
                }
            }
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
