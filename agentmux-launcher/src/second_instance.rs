// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

#[cfg(not(target_os = "windows"))]
use crate::logging::log;

/// Phase B.6 (post-fix) — forward an `open_new_window` request to
/// the already-running host and let this launcher exit 0.
///
/// The host writes `<data-dir>/ipc-port` after CEF init as
/// `port:token`. We open a TCP connection to 127.0.0.1:port, send a
/// minimal HTTP/1.1 POST to /ipc with the bearer token and a JSON
/// body, and bail. We deliberately do NOT pull in reqwest: the
/// launcher binary should stay tiny (~325 KB) and the protocol is
/// fixed, so a hand-rolled request is the right tool.
///
/// Failure classification (codex P2 PR #598):
/// - `Transient` — port file missing / unreadable / malformed.
///   The host is alive (pipe held) but mid-startup; caller exits
///   0 silently so the user isn't punished for double-clicking
///   quickly.
/// - `Fatal` — port file is readable, but the HTTP path failed
///   (connect refused, write failed, timeout). Either a hung
///   host or a non-running-instance source of
///   `ERROR_ACCESS_DENIED` (namespace conflict, security
///   descriptor failure). Caller surfaces the dialog so the user
///   sees a real problem rather than a silent no-op.
pub(crate) enum ForwardError {
    Transient(String),
    Fatal(String),
}

/// Forward an arbitrary host IPC command over the same authenticated
/// localhost channel `forward_open_new_window` uses.
///
/// Extracted (issue #2977 Workstream 1) so the tray can also send `quit_app`
/// without a second copy of the port-file read, bearer-token handshake, and
/// the read-the-response subtlety documented below — all of which are easy to
/// get subtly wrong and none of which are specific to opening a window.
/// `forward_open_new_window` is now a thin wrapper, so the long-standing
/// second-instance path keeps its exact behavior.
pub(crate) fn forward_host_cmd(
    data_dir: &std::path::Path,
    dir_hash: &str,
    cmd: &str,
) -> Result<(), ForwardError> {
    forward_host_cmd_impl(data_dir, dir_hash, cmd)
}

pub(crate) fn forward_open_new_window(
    data_dir: &std::path::Path,
    dir_hash: &str,
) -> Result<(), ForwardError> {
    forward_host_cmd_impl(data_dir, dir_hash, "open_new_window")
}

fn forward_host_cmd_impl(
    data_dir: &std::path::Path,
    dir_hash: &str,
    cmd: &str,
) -> Result<(), ForwardError> {
    // Read the version-scoped port file so we reach THIS version's host,
    // not a concurrent release's host that may have overwritten "ipc-port".
    let port_file_name = format!("ipc-port-{}", dir_hash);
    let port_file = data_dir.join(&port_file_name);
    let contents = std::fs::read_to_string(&port_file).map_err(|e| {
        ForwardError::Transient(format!("read {}: {}", port_file.display(), e))
    })?;
    let trimmed = contents.trim();
    let (port_str, token) = trimmed.split_once(':').ok_or_else(|| {
        ForwardError::Transient(format!(
            "malformed port file (expected port:token): {}",
            trimmed
        ))
    })?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| ForwardError::Transient(format!("invalid port {:?}: {}", port_str, e)))?;

    // From here on the file was readable: any failure is a fatal
    // forward (the host got far enough to publish but isn't
    // serving the IPC port).
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2))
        .map_err(|e| ForwardError::Fatal(format!("connect 127.0.0.1:{}: {}", port, e)))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .ok();

    // `cmd` is a fixed internal identifier chosen by call sites in this
    // crate, never user input, so a plain format is safe here — there is no
    // untrusted string to escape.
    let body = format!(r#"{{"cmd":"{}"}}"#, cmd);
    let body = body.as_str();
    let req = format!(
        "POST /ipc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        token,
        body.len(),
        body
    );
    use std::io::{Read, Write};
    stream
        .write_all(req.as_bytes())
        .map_err(|e| ForwardError::Fatal(format!("write request: {}", e)))?;
    // CRITICAL: read at least the status line. The host's axum
    // handler is async — if the launcher closes the TCP socket
    // before axum has finished parsing + dispatching to
    // `open_new_window`, the request can be dropped (smoke caught
    // exactly this on v0.33.481: the launcher logged "forwarded"
    // but no second window appeared because the process exited
    // before axum ran the handler). We don't care about the body
    // — `Connection: close` lets the server drop the socket once
    // the response is written, so a single short read is enough
    // to keep the connection alive past handler dispatch.
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .ok();
    let mut sink = [0u8; 64];
    let _ = stream.read(&mut sink);
    Ok(())
}

/// Best-effort `open_new_window` forward for the unix second-instance path.
/// Unlike the Windows path (which pops a dialog on a fatal forward), unix just
/// logs and lets the caller exit 0: the existing instance is alive (its socket
/// answered our connect probe), so a transient/fatal forward failure shouldn't
/// block — at worst the relaunch is a silent no-op instead of a new window.
/// SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md.
#[cfg(not(target_os = "windows"))]
pub(crate) fn forward_open_new_window_or_log(data_dir: &std::path::Path, dir_hash: &str) {
    match forward_open_new_window(data_dir, dir_hash) {
        Ok(()) => log("forwarded open_new_window to existing instance"),
        Err(ForwardError::Transient(reason)) => {
            log(&format!("open_new_window forward transient (host mid-startup?): {}", reason))
        }
        Err(ForwardError::Fatal(reason)) => {
            log(&format!("open_new_window forward failed: {}", reason))
        }
    }
}

/// Bind the launcher's IPC socket with single-instance enforcement +
/// crash-safe stale-socket recovery, serialized across concurrent
/// launchers via `flock(2)`.
///
/// Returns a bound `UnixListener` on success. Calls `std::process::exit`
/// on:
///   * second-instance detection (exit code 0)
///   * a hard bind failure that isn't `EADDRINUSE` (exit code 2)
///   * unable to acquire the recovery lock (exit code 2)
///
/// Why the lockfile (codex P1 + reagent P1 on PR #1288): see the call-
/// site comment. Two-launcher concurrent stale-cleanup would otherwise
/// produce two live launchers for one data dir.
#[cfg(not(target_os = "windows"))]
pub(crate) fn bind_socket_with_recovery(
    socket_path: &str,
    data_dir: &std::path::Path,
    dir_hash: &str,
    channel: &str,
) -> tokio::net::UnixListener {
    use std::os::unix::io::AsRawFd as _;

    // Fast path: bind without contention.
    match crate::ipc::server::bind_first_unix_socket(socket_path) {
        Ok(l) => return l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => { /* slow path below */ }
        Err(e) => {
            log(&format!("FATAL: bind {} failed: {}", socket_path, e));
            eprintln!(
                "AgentMux failed to start: could not bind IPC socket.\n\nSocket: {}\nError: {}",
                socket_path, e
            );
            std::process::exit(2);
        }
    }

    // Slow path: contention. Acquire the recovery lock so only one
    // launcher at a time does the connect-probe + unlink + rebind.
    let lock_path = format!("{}.lock", socket_path);
    let lock_file = match std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            log(&format!(
                "FATAL: could not open recovery lockfile {}: {}",
                lock_path, e
            ));
            eprintln!(
                "AgentMux failed to start: could not open IPC recovery lockfile.\n\nLockfile: {}\nError: {}",
                lock_path, e
            );
            std::process::exit(2);
        }
    };
    // Block until we have the lock — another launcher's recovery
    // window is bounded by a single bind + a single connect-probe;
    // we won't wait long. flock(2) is auto-released on close (when
    // the OS reaps the launcher process), so a SIGKILL'd holder
    // doesn't leak the lock.
    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        let errno = std::io::Error::last_os_error();
        log(&format!(
            "FATAL: flock({}, LOCK_EX) failed: {}",
            lock_path, errno
        ));
        eprintln!(
            "AgentMux failed to start: could not acquire IPC recovery lock.\n\nLockfile: {}\nError: {}",
            lock_path, errno
        );
        std::process::exit(2);
    }

    // Retry the bind under the lock — another launcher may have
    // already cleaned up the stale file while we were waiting on
    // flock, leaving us free to bind directly.
    match crate::ipc::server::bind_first_unix_socket(socket_path) {
        Ok(l) => return l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => { /* probe below */ }
        Err(e) => {
            log(&format!(
                "FATAL: post-lock bind {} failed: {}",
                socket_path, e
            ));
            eprintln!(
                "AgentMux failed to start: post-lock IPC bind failed.\n\nSocket: {}\nError: {}",
                socket_path, e
            );
            std::process::exit(2);
        }
    }

    // Disambiguate: is the existing socket a real running launcher,
    // or a stale file?
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => {
            // Real second-instance. Forward an `open_new_window` request to the
            // already-running launcher's host (Windows-parity — main.rs:1292),
            // then exit cleanly. SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md.
            if channel.starts_with("dev-") {
                eprintln!(
                    "AgentMux dev instance already running (channel: {}).\n\
                     Use `task dev:local` to launch a second isolated session.\n\
                     Socket: {}",
                    channel, socket_path
                );
            } else {
                eprintln!(
                    "AgentMux is already running for this data directory.\n\nSocket: {}",
                    socket_path
                );
            }
            forward_open_new_window_or_log(data_dir, dir_hash);
            log(&format!(
                "[ipc] second-instance detected — existing launcher owns {}",
                socket_path
            ));
            std::process::exit(0);
        }
        Err(connect_err)
            if connect_err.kind() == std::io::ErrorKind::ConnectionRefused
                || connect_err.raw_os_error() == Some(libc::ENOENT) =>
        {
            // Stale socket file from a crashed launcher. Unlink and
            // rebind. The lock serializes us against other launchers
            // ALSO doing recovery, but it does NOT block a fresh
            // launcher taking the fast-path bind() above — that
            // launcher is lock-free and can win the socket in the
            // microsecond window between our `remove_file` and our
            // `bind`. If that happens, AddrInUse means a real
            // launcher just claimed the socket and WE are now the
            // losing second instance, not a failed start.
            // (Reagent P2 on PR #1288.)
            log(&format!(
                "[ipc] stale socket file at {} — unlinking and rebinding (under recovery lock)",
                socket_path
            ));
            let _ = std::fs::remove_file(socket_path);
            match crate::ipc::server::bind_first_unix_socket(socket_path) {
                Ok(l) => l,
                Err(retry_e) if retry_e.kind() == std::io::ErrorKind::AddrInUse => {
                    if channel.starts_with("dev-") {
                        eprintln!(
                            "AgentMux dev instance already running (channel: {}).\n\
                             Use `task dev:local` to launch a second isolated session.\n\
                             Socket: {}",
                            channel, socket_path
                        );
                    } else {
                        eprintln!(
                            "AgentMux is already running for this data directory.\n\nSocket: {}",
                            socket_path
                        );
                    }
                    forward_open_new_window_or_log(data_dir, dir_hash);
                    log(&format!(
                        "[ipc] post-recovery bind lost the race to a fresh launcher on {} — exiting as second instance",
                        socket_path
                    ));
                    std::process::exit(0);
                }
                Err(retry_e) => {
                    log(&format!(
                        "FATAL: bind retry after stale-socket unlink failed: {}",
                        retry_e
                    ));
                    eprintln!(
                        "AgentMux failed to start: IPC rebind after stale cleanup failed.\n\nSocket: {}\nError: {}",
                        socket_path, retry_e
                    );
                    std::process::exit(2);
                }
            }
        }
        Err(other) => {
            log(&format!(
                "[ipc] AddrInUse but connect probe failed in an unexpected way: {} — \
                 treating as second instance and exiting cleanly",
                other
            ));
            std::process::exit(0);
        }
    }
    // `lock_file` drops here; flock auto-released on close.
}
