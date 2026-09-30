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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const DUMP_DIR: &str = r"C:\CrashDumps\agentmuxsrv";
/// Unix Domain Socket path used for crash-handler IPC.
const SOCKET_PATH: &str = r"C:\CrashDumps\agentmuxsrv\monitor.sock";
/// Message kind for sending the crashing process's PID before a dump request.
/// The monitor stores this PID and uses it in the dump filename.
const MSG_KIND_CRASH_PID: u32 = 0;

// ─── Monitor process (server side) ───────────────────────────────────────────

/// Entry point for the monitor process.
///
/// Called when the binary is invoked with `--crash-monitor`. Runs a blocking
/// `minidumper::Server` loop. Exits when the main process disconnects (either
/// cleanly on shutdown, or after the crash dump is written).
pub fn run_monitor() {
    let dump_dir = PathBuf::from(DUMP_DIR);
    if let Err(e) = std::fs::create_dir_all(&dump_dir) {
        eprintln!("[crash-monitor] failed to create dump dir {}: {}", dump_dir.display(), e);
        // Continue — create_minidump_file will fail gracefully per dump request.
    }
    eprintln!(
        "[crash-monitor] started (pid={}), writing dumps to {}",
        std::process::id(),
        dump_dir.display()
    );

    let socket_name = minidumper::SocketName::path(SOCKET_PATH);
    let mut server = match minidumper::Server::with_name(socket_name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[crash-monitor] failed to bind socket '{}': {}", SOCKET_PATH, e);
            return;
        }
    };

    let shutdown = AtomicBool::new(false);
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
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
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

/// Spawn a crash monitor child process and attach the VEH crash handler in this process.
///
/// Returns `Some(guard)` on success. The guard must be kept alive (e.g. as a `let _` binding
/// at the top of `main`) for the handler to remain active. Returns `None` on any failure —
/// non-fatal, the process continues without the VEH handler (WER LocalDumps still works).
pub fn spawn_and_attach() -> Option<CrashHandlerGuard> {
    // Ensure dump directory exists before spawning the monitor.
    let _ = std::fs::create_dir_all(DUMP_DIR);

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[crash-handler] failed to get current exe path: {}", e);
            return None;
        }
    };

    // Spawn monitor process. Null stdin/stdout so it doesn't inherit the sidecar's
    // stdin reader (which drives the stdin-EOF watchdog in the main process).
    let child = match std::process::Command::new(&exe)
        .arg("--crash-monitor")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[crash-handler] failed to spawn crash monitor: {}", e);
            return None;
        }
    };
    eprintln!("[crash-handler] crash monitor spawned (pid={})", child.id());

    // Connect to the monitor's socket with retry — give the server time to start.
    let socket_name = minidumper::SocketName::path(SOCKET_PATH);
    let client = connect_with_retry(socket_name, 10, std::time::Duration::from_millis(25));
    let client = match client {
        Some(c) => c,
        None => {
            eprintln!("[crash-handler] could not connect to crash monitor after retries");
            return None;
        }
    };

    // Verify the connection is healthy before installing the VEH handler.
    if let Err(e) = client.ping() {
        eprintln!("[crash-handler] crash monitor ping failed: {}", e);
        return None;
    }

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
                let pid_bytes = crash_context.process_id.to_le_bytes();
                let _ = client.send_message(MSG_KIND_CRASH_PID, &pid_bytes);

                // Best-effort dump request. If the pipe is broken we can't do anything useful.
                let _ = client.request_dump(crash_context);
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
