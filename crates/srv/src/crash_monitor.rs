// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Out-of-process crash dump monitor for Windows.
//!
//! Architecture
//! ============
//! Two-process pattern:
//!   - Main process: installs a VEH handler via `crash-handler`. On crash, the handler
//!     sends the crash context to the monitor over a Unix Domain Socket (IPC via `minidumper`).
//!   - Monitor process: same binary, launched with `--crash-monitor`. Runs a blocking
//!     `minidumper::Server` that receives crash contexts and writes .dmp files.
//!
//! Why out-of-process?
//!   A crash may corrupt the heap or stack. Writing a minidump from inside the crashing
//!   process is unreliable. The monitor runs in a healthy isolated process and can safely
//!   call `MiniDumpWriteDump` even if the main process is badly corrupted.
//!
//! What this catches
//! =================
//! - Access violations (`0xC0000005`)
//! - Heap corruption detected by heap manager
//! - Rust panics that abort
//! - Any exception that reaches the Vectored Exception Handler
//!
//! What this does NOT catch
//! ========================
//! - `__fastfail` (`int 0x29`, exit code `0xC0000409`) — the CPU traps directly to the
//!   kernel which terminates the process before returning to user mode. VEH is bypassed.
//!   Use WER `LocalDumps` for that (already configured via `enable-wer-dumps.reg`).
//!
//! Dump location: `C:\CrashDumps\agentmuxsrv\agentmuxsrv-<unix_ts>-<pid>.dmp`
//! Socket path:   `C:\CrashDumps\agentmuxsrv\monitor.sock`

#![cfg(windows)]

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const DUMP_DIR: &str = r"C:\CrashDumps\agentmuxsrv";
/// Names another folder for the dumps and sockets (the crash monitor tests use a temporary one).
const DIR_ENV: &str = "AGENTMUX_CRASH_DIR";
/// How long a monitor waits for its srv to connect before it gives up, in ms. The environment
/// variable shortens it for the tests.
const FIRST_CLIENT_GRACE_MS: u64 = 60_000;
const GRACE_ENV: &str = "AGENTMUX_CRASH_MONITOR_GRACE_MS";
/// Message kind for sending the crashing process's PID before a dump request.
/// The monitor stores this PID and uses it in the dump filename.
const MSG_KIND_CRASH_PID: u32 = 0;
/// Set once any srv has connected to this monitor.
static CLIENT_SEEN: AtomicBool = AtomicBool::new(false);

fn dump_dir() -> PathBuf {
    std::env::var_os(DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DUMP_DIR))
}

/// The Unix Domain Socket one srv and its monitor talk over. Every srv has its own: a shared
/// path let a new monitor take the file over from a live one (`minidumper` removes the path
/// before it binds) and let an exiting monitor delete the file of a newer one.
fn socket_path(dir: &std::path::Path, srv_pid: u32) -> PathBuf {
    dir.join(format!("monitor-{srv_pid}.sock"))
}

/// Remove the sockets of monitors that were killed before they could (their srv's pid is in the
/// name; a day-old file of any other pid is no one's).
fn prune_stale_sockets(dir: &std::path::Path, own_pid: u32, max_age: std::time::Duration) {
    let own = socket_path(dir, own_pid);
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path == own || !(name.starts_with("monitor-") && name.ends_with(".sock")) {
            continue;
        }
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age >= max_age);
        if old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// ─── Monitor process (server side) ───────────────────────────────────────

/// Entry point for the monitor process.
///
/// Called when the binary is invoked with `--crash-monitor --parent-pid <srv pid>`. Runs a
/// blocking `minidumper::Server` loop on the socket of that srv. Exits when its srv disconnects
/// (cleanly on shutdown, or after the crash dump is written), or when no srv has connected after
/// [`FIRST_CLIENT_GRACE_MS`]: a srv that could not reach it in time never will, and a monitor
/// nobody uses would otherwise run for ever and hold the exe open.
pub fn run_monitor(parent_pid: Option<u32>) {
    let dump_dir = dump_dir();
    if let Err(e) = std::fs::create_dir_all(&dump_dir) {
        eprintln!("[crash-monitor] failed to create dump dir {}: {}", dump_dir.display(), e);
        // Continue — create_minidump_file will fail gracefully per dump request.
    }
    // Without a pid (an invocation from before the pid was passed) the old shared name.
    let socket = match parent_pid {
        Some(pid) => socket_path(&dump_dir, pid),
        None => dump_dir.join("monitor.sock"),
    };
    eprintln!(
        "[crash-monitor] started (pid={}), writing dumps to {}",
        std::process::id(),
        dump_dir.display()
    );

    let Some(socket_str) = socket.to_str() else {
        eprintln!("[crash-monitor] socket path is not valid UTF-8: {}", socket.display());
        return;
    };
    let socket_name = minidumper::SocketName::path(socket_str);
    let mut server = match minidumper::Server::with_name(socket_name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[crash-monitor] failed to bind socket '{}': {}", socket.display(), e);
            return;
        }
    };

    let shutdown = std::sync::Arc::new(AtomicBool::new(false));
    let grace = std::time::Duration::from_millis(
        std::env::var(GRACE_ENV).ok().and_then(|v| v.parse().ok()).unwrap_or(FIRST_CLIENT_GRACE_MS),
    );
    let stop = shutdown.clone();
    std::thread::spawn(move || {
        std::thread::sleep(grace);
        if !CLIENT_SEEN.load(Ordering::Relaxed) {
            eprintln!("[crash-monitor] no srv connected within {grace:?}; giving up");
            stop.store(true, Ordering::Relaxed);
        }
    });
    let handler = Box::new(CrashDumpHandler { dump_dir, crash_pid: AtomicU32::new(0) });

    if let Err(e) = server.run(handler, &shutdown, None) {
        eprintln!("[crash-monitor] server loop ended: {}", e);
    }

    eprintln!("[crash-monitor] exiting");
}

struct CrashDumpHandler {
    dump_dir: PathBuf,
    /// PID of the crashing process, set by `on_message` before `create_minidump_file` is called.
    /// The client sends MSG_KIND_CRASH_PID just before `request_dump` so the PID is available
    /// when the server needs to name the file.
    crash_pid: AtomicU32,
}

impl minidumper::ServerHandler for CrashDumpHandler {
    fn create_minidump_file(&self) -> Result<(std::fs::File, PathBuf), std::io::Error> {
        let ts = agentmux_common::time::now_secs_u64();
        // Use the crashing process PID sent via on_message, not std::process::id()
        // (which would give the monitor's own PID — misleading for diagnosis).
        let pid = self.crash_pid.load(Ordering::Relaxed);
        let filename = if pid != 0 {
            format!("agentmuxsrv-{}-{}.dmp", ts, pid)
        } else {
            format!("agentmuxsrv-{}-unknown.dmp", ts)
        };
        let path = self.dump_dir.join(&filename);
        eprintln!("[crash-monitor] creating dump: {}", path.display());
        let file = std::fs::File::create(&path)?;
        Ok((file, path))
    }

    fn on_minidump_created(
        &self,
        result: Result<minidumper::MinidumpBinary, minidumper::Error>,
    ) -> minidumper::LoopAction {
        match result {
            Ok(binary) => {
                eprintln!("[crash-monitor] dump written: {}", binary.path.display());
            }
            Err(e) => {
                eprintln!("[crash-monitor] failed to write dump: {}", e);
            }
        }
        // Continue — keep monitoring in case of multiple clients or future restarts.
        minidumper::LoopAction::Continue
    }

    fn on_message(&self, kind: u32, buffer: Vec<u8>) {
        if kind == MSG_KIND_CRASH_PID {
            if let Ok(bytes) = <[u8; 4]>::try_from(buffer.as_slice()) {
                let pid = u32::from_le_bytes(bytes);
                self.crash_pid.store(pid, Ordering::Relaxed);
                eprintln!("[crash-monitor] crash pid set to {}", pid);
            }
        }
    }

    fn on_client_connected(&self, _num_clients: usize) -> minidumper::LoopAction {
        CLIENT_SEEN.store(true, Ordering::Relaxed);
        minidumper::LoopAction::Continue
    }

    fn on_client_disconnected(&self, num_clients: usize) -> minidumper::LoopAction {
        // Exit when the last client (the main process) disconnects.
        if num_clients == 0 {
            minidumper::LoopAction::Exit
        } else {
            minidumper::LoopAction::Continue
        }
    }
}

// ─── Main process (client + handler side) ────────────────────────────────────

/// Guard that keeps the crash handler installed for the lifetime of the process.
/// Dropping this guard uninstalls the VEH handler.
pub struct CrashHandlerGuard {
    _handler: crash_handler::CrashHandler,
}

/// Start this exe again as the crash monitor of the srv with this pid.
fn spawn_monitor(srv_pid: u32) -> Option<std::process::Child> {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[crash-handler] failed to get current exe path: {}", e);
            return None;
        }
    };

    // Null stdin/stdout so it doesn't inherit the sidecar's stdin reader (which drives the
    // stdin-EOF watchdog in the main process).
    let mut monitor_cmd = std::process::Command::new(&exe);
    monitor_cmd
        .arg("--crash-monitor")
        .arg("--parent-pid")
        .arg(srv_pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    // srv is a console-subsystem binary launched windowless; without this flag
    // the monitor (the same exe) gets a fresh console window at startup, on top
    // of the splash. See crates/common/src/cli.rs for the same fix elsewhere.
    #[cfg(windows)]
    {
        monitor_cmd.no_window();
    }
    match monitor_cmd.spawn() {
        Ok(child) => {
            eprintln!("[crash-handler] crash monitor spawned (pid={})", child.id());
            Some(child)
        }
        Err(e) => {
            eprintln!("[crash-handler] failed to spawn crash monitor: {}", e);
            None
        }
    }
}

/// Connect to the monitor `child` on this srv's socket and check the connection works. A monitor
/// this srv can't reach would run for ever with no one to serve (and hold this exe open), so it
/// is stopped again whenever the attach fails.
fn connect_to_monitor(dir: &std::path::Path, srv_pid: u32, child: &mut std::process::Child) -> Option<minidumper::Client> {
    let client = connect_to_monitor_socket(dir, srv_pid);
    let healthy = match &client {
        Some(c) => match c.ping() {
            Ok(()) => true,
            Err(e) => {
                eprintln!("[crash-handler] crash monitor ping failed: {}", e);
                false
            }
        },
        None => false,
    };
    if healthy {
        return client;
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// Connect with retry — a cold start of the exe (a scan, a slow disk) can take well over a second.
fn connect_to_monitor_socket(dir: &std::path::Path, srv_pid: u32) -> Option<minidumper::Client> {
    let socket = socket_path(dir, srv_pid);
    let Some(socket_str) = socket.to_str() else {
        eprintln!("[crash-handler] socket path is not valid UTF-8: {}", socket.display());
        return None;
    };
    let client = connect_with_retry(minidumper::SocketName::path(socket_str), 40, std::time::Duration::from_millis(50));
    if client.is_none() {
        eprintln!("[crash-handler] could not connect to crash monitor after retries");
    }
    client
}

/// Spawn a crash monitor child process and attach the VEH crash handler in this process.
///
/// Returns `Some(guard)` on success. The guard must be kept alive (e.g. as a `let _` binding
/// at the top of `main`) for the handler to remain active. Returns `None` on any failure —
/// non-fatal, the process continues without the VEH handler (WER LocalDumps still works).
pub fn spawn_and_attach() -> Option<CrashHandlerGuard> {
    // Ensure dump directory exists before spawning the monitor.
    let dir = dump_dir();
    let _ = std::fs::create_dir_all(&dir);
    let pid = std::process::id();
    prune_stale_sockets(&dir, pid, std::time::Duration::from_secs(24 * 3600));

    let mut child = spawn_monitor(pid)?;
    let client = connect_to_monitor(&dir, pid, &mut child)?;

    // Install Vectored Exception Handler.
    //
    // Safety contract: the closure is called in an exception context (Windows VEH).
    // Only async-signal-safe operations are allowed:
    //   - `client.request_dump()` uses pre-allocated IPC buffers, no heap allocation.
    //   - No Rust std synchronisation primitives (Mutex/RwLock) are used.
    //   - No log flushing or tracing calls (those may lock).
    let handler = unsafe {
        crash_handler::CrashHandler::attach(crash_handler::make_crash_event(
            move |crash_context: &crash_handler::CrashContext| {
                // Send our PID to the monitor BEFORE requesting the dump so that
                // on_message sets crash_pid before create_minidump_file is called.
                // The send uses a fixed-size stack buffer internally — no heap allocation.
                request_dump_of_this_process(&client, crash_context);
                // Return Handled(false): let the exception continue propagating so that
                // WER still fires (necessary for __fastfail which bypasses VEH anyway,
                // and to preserve normal Windows crash reporting for other exception types).
                crash_handler::CrashEventResult::Handled(false)
            },
        ))
    };

    match handler {
        Ok(h) => {
            eprintln!("[crash-handler] VEH handler installed");
            Some(CrashHandlerGuard { _handler: h })
        }
        Err(e) => {
            eprintln!("[crash-handler] failed to install VEH handler: {}", e);
            None
        }
    }
}

/// Tells the monitor which process is crashing and asks it for a dump.
///
/// The monitor opens this process by pid to read it, and a hardened process refuses that to every
/// other process of the user: open it for the length of the request, then close it again (the
/// exception may yet be handled). Both calls only switch a security descriptor prepared at startup,
/// so nothing here allocates. The send uses a fixed-size stack buffer too.
fn request_dump_of_this_process(client: &minidumper::Client, crash_context: &crash_handler::CrashContext) {
    // Send our PID to the monitor BEFORE requesting the dump so that on_message sets crash_pid
    // before create_minidump_file is called.
    let pid_bytes = crash_context.process_id.to_le_bytes();
    let _ = client.send_message(MSG_KIND_CRASH_PID, &pid_bytes);
    agentmux_common::process_hardening::relax();
    // Best-effort dump request. If the pipe is broken we can't do anything useful.
    let _ = client.request_dump(crash_context);
    agentmux_common::process_hardening::restore();
}

/// Try to connect to the server socket up to `attempts` times, sleeping `delay` between tries.
fn connect_with_retry(
    socket_name: minidumper::SocketName<'_>,
    attempts: u32,
    delay: std::time::Duration,
) -> Option<minidumper::Client> {
    for i in 0..attempts {
        match minidumper::Client::with_name(socket_name) {
            Ok(c) => return Some(c),
            Err(e) if i + 1 < attempts => {
                eprintln!("[crash-handler] connect attempt {}/{}: {}", i + 1, attempts, e);
                std::thread::sleep(delay);
            }
            Err(e) => {
                eprintln!("[crash-handler] connect attempt {}/{} failed: {}", i + 1, attempts, e);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// Runs only as the child of the tests below: hardens itself, connects to the monitor in
    /// the test process, and reports a crash through the real handler path (a simulated
    /// exception, so the process does not die). `AGENTMUX_CM_PLAIN=1` skips the
    /// `relax`/`restore` pair, to show the hardening alone would lose the dump.
    #[test]
    #[ignore = "runs as the child of the crash monitor tests"]
    fn crash_child() {
        let Some(socket) = std::env::var_os("AGENTMUX_CM_CHILD") else { return };
        assert!(agentmux_common::process_hardening::apply(), "the child hardens itself");
        let socket = socket.into_string().expect("a utf-8 socket path");
        let client = connect_with_retry(minidumper::SocketName::path(&socket), 100, std::time::Duration::from_millis(50))
            .expect("the monitor is listening");
        let plain = std::env::var_os("AGENTMUX_CM_PLAIN").is_some();
        let handler = crash_handler::CrashHandler::attach(unsafe {
            crash_handler::make_crash_event(move |ctx: &crash_handler::CrashContext| {
                if plain {
                    let _ = client.send_message(MSG_KIND_CRASH_PID, &ctx.process_id.to_le_bytes());
                    let _ = client.request_dump(ctx);
                } else {
                    request_dump_of_this_process(&client, ctx);
                }
                crash_handler::CrashEventResult::Handled(true)
            })
        })
        .expect("handler attached");
        handler.simulate_exception(None);
    }

    /// A real second process, hardened, reports a crash; the monitor (this process) writes the dump.
    fn dump_size(plain: bool) -> u64 {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("monitor.sock");
        let dumps = dir.path().join("dumps");
        std::fs::create_dir_all(&dumps).unwrap();

        let mut server = minidumper::Server::with_name(minidumper::SocketName::path(socket.to_str().unwrap())).unwrap();
        let handler = Box::new(CrashDumpHandler { dump_dir: dumps.clone(), crash_pid: AtomicU32::new(0) });
        let shutdown = std::sync::Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let server_thread = std::thread::spawn(move || {
            let _ = server.run(handler, &stop, None);
        });

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_monitor::tests::crash_child", "--ignored", "--nocapture"])
            .env("AGENTMUX_CM_CHILD", &socket)
            .envs(plain.then_some(("AGENTMUX_CM_PLAIN", "1")))
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let status = child.wait().unwrap();
        assert!(status.success(), "the child ran to the end: {status}");
        // Give the monitor a moment to finish writing, then stop it.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        shutdown.store(true, Ordering::Relaxed);
        let _ = server_thread.join();

        std::fs::read_dir(&dumps)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "dmp"))
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn every_srv_has_its_own_socket() {
        let dir = std::path::Path::new("dumps");
        assert_ne!(socket_path(dir, 1), socket_path(dir, 2));
        assert_eq!(socket_path(dir, 7).file_name().unwrap(), "monitor-7.sock");
    }

    #[test]
    fn the_sockets_of_other_srvs_are_pruned_once_a_day_old_and_nothing_else_is() {
        let dir = tempfile::tempdir().unwrap();
        let three_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
        let make = |name: &str, old: bool| {
            let path = dir.path().join(name);
            let file = std::fs::File::create(&path).unwrap();
            if old {
                file.set_modified(three_days_ago).unwrap();
            }
            path
        };
        let gone = make("monitor-1.sock", true);
        let recent = make("monitor-2.sock", false);
        let own = make("monitor-9.sock", true);
        let other = make("notes.txt", true);
        prune_stale_sockets(dir.path(), 9, std::time::Duration::from_secs(24 * 3600));
        assert!(!gone.exists(), "an old socket of another srv is removed");
        assert!(recent.exists(), "a recent one may belong to a srv that is starting");
        assert!(own.exists(), "this srv's own socket is never pruned");
        assert!(other.exists(), "only monitor sockets are touched");
    }

    #[test]
    fn a_hardened_process_still_gets_its_crash_dump_written() {
        assert!(dump_size(false) > 0, "a dump of the hardened child was written");
    }

    #[test]
    fn without_relaxing_a_hardened_process_could_not_be_dumped() {
        if agentmux_common::debug_privilege::refusal_cannot_be_observed("without_relaxing_a_hardened_process_could_not_be_dumped") {
            return;
        }
        assert_eq!(dump_size(true), 0, "the control: hardening alone refuses the monitor");
    }
}
