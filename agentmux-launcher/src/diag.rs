// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.8 — `agentmux.exe --diag wrr` Tool client.
//
// Connects to a running AgentMux instance's IPC pipe as a `Tool`
// client, registers, captures events for a short observation
// window, prints a human-readable summary to stdout, and exits.
//
// Operator visibility into the launcher's reducer state without
// needing a debugger or log scraping. The output IS the diagnosis
// surface for "what does the launcher think is happening right now."
//
// Why Tool kind: doesn't drive the reducer's lifecycle (Host
// drives Starting → Running), doesn't trigger OrphanInstance saga,
// gets all events broadcast on the pipe without affecting the
// running instance.
//
// Phase D will add a `Command::GetSnapshot` RPC for a structured
// reply (current windows + pool + drift state) — until then, this
// captures the live event stream over a 2s window and infers
// state from observed events.

use std::time::Duration;

use agentmux_common::ipc::{ClientKind, Command, Event};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(target_os = "windows")]
use tokio::net::windows::named_pipe::ClientOptions;

const OBSERVATION_WINDOW: Duration = Duration::from_secs(2);

/// Entry point for diag mode. Returns `Ok(())` on success (printed
/// summary, exit 0) or `Err(message)` for the caller to surface and
/// exit non-zero.
#[cfg(target_os = "windows")]
pub async fn run_wrr_diag(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    let version = env!("CARGO_PKG_VERSION");
    let is_dev = cfg!(debug_assertions);

    let paths = crate::data_dir::resolve_paths(launcher_exe_dir, version, is_dev)
        .map_err(|e| format!("path resolution failed: {}", e))?;
    let dir_hash = crate::hash::data_dir_hash16(&paths.data_dir);
    let pipe_path = crate::ipc::pipe_name(&dir_hash);

    println!("AgentMux diagnostic — connecting to {}", pipe_path);
    println!("Data dir: {}", paths.data_dir.display());

    let client = ClientOptions::new()
        .open(&pipe_path)
        .map_err(|e| format!(
            "could not open pipe {}: {} (is AgentMux running for this data dir?)",
            pipe_path, e
        ))?;
    println!("Connected. Registering as Tool client...\n");

    let (read_half, mut write_half) = tokio::io::split(client);

    let register = Command::Register {
        kind: ClientKind::Tool,
        pid: std::process::id(),
        version: version.to_string(),
    };
    let mut buf = serde_json::to_vec(&register)
        .map_err(|e| format!("serialize Register: {}", e))?;
    buf.push(b'\n');
    write_half.write_all(&buf).await
        .map_err(|e| format!("send Register: {}", e))?;
    write_half.flush().await
        .map_err(|e| format!("flush Register: {}", e))?;

    // Phase D.1 — request a state snapshot. Reply arrives on the
    // broadcast bus alongside the Register reply; we filter for it
    // in the event collection below.
    let mut buf = serde_json::to_vec(&Command::GetSnapshot)
        .map_err(|e| format!("serialize GetSnapshot: {}", e))?;
    buf.push(b'\n');
    write_half.write_all(&buf).await
        .map_err(|e| format!("send GetSnapshot: {}", e))?;
    write_half.flush().await
        .map_err(|e| format!("flush GetSnapshot: {}", e))?;

    // Read events for OBSERVATION_WINDOW. The launcher's IPC server
    // (post-B.8) broadcasts every reducer event on a server-wide
    // bus; this connection's fanout writes them to us. We collect
    // everything that arrives in the window and print a summary.
    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();
    let mut events: Vec<Event> = Vec::new();
    let deadline = tokio::time::Instant::now() + OBSERVATION_WINDOW;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, lines.next_line()).await {
            Ok(Ok(Some(line))) if line.trim().is_empty() => continue,
            Ok(Ok(Some(line))) => {
                match serde_json::from_str::<Event>(&line) {
                    Ok(evt) => events.push(evt),
                    Err(e) => eprintln!("[warn] could not parse event: {} ({})", e, line),
                }
            }
            Ok(Ok(None)) => {
                eprintln!("[warn] pipe closed before observation window elapsed");
                break;
            }
            Ok(Err(e)) => {
                return Err(format!("read error: {}", e));
            }
            Err(_) => break, // timeout → observation window closed
        }
    }

    // Phase B.8 — send Goodbye so the server emits ProcessExited and
    // marks our PID record as Exited. Without this, repeated
    // `--diag wrr` invocations accumulate stale `Running` records;
    // when Windows reuses a PID, the next Register fails with
    // `AlreadyRegistered`. Best-effort: errors are non-fatal because
    // we're about to exit anyway. (codex P2 PR #605.)
    let goodbye = match serde_json::to_vec(&Command::Goodbye) {
        Ok(mut b) => { b.push(b'\n'); b }
        Err(_) => Vec::new(),
    };
    if !goodbye.is_empty() {
        let _ = write_half.write_all(&goodbye).await;
        let _ = write_half.flush().await;
    }

    print_summary(&events);
    Ok(())
}

/// Stub for non-Windows platforms. Phase 7 cross-platform IPC will
/// implement Unix domain sockets; until then `--diag wrr` is
/// Windows-only.
#[cfg(not(target_os = "windows"))]
pub async fn run_wrr_diag(_launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    Err("--diag wrr is Windows-only (Phase 7 will add Unix domain socket parity)".to_string())
}

#[cfg(target_os = "windows")]
fn print_summary(events: &[Event]) {
    // Phase D.1 — pull the Snapshot out and print it first as the
    // canonical "state now" view; remaining events are the live
    // stream observed during the 2s window.
    let (snapshot, stream): (Vec<_>, Vec<_>) = events
        .iter()
        .partition(|e| matches!(e, Event::Snapshot { .. }));

    if let Some(Event::Snapshot {
        version,
        lifecycle,
        windows,
        pool,
        instance_registry,
        backend_window_ids,
        monitors,
    }) = snapshot.first().copied()
    {
        println!("=== Snapshot (event_version={}) ===", version);
        println!("Lifecycle: {:?}", lifecycle);
        println!("Monitors:  {} ({:?})", monitors.len(), monitors);
        println!();
        println!("Windows ({}):", windows.len());
        if windows.is_empty() {
            println!("  (none)");
        } else {
            for w in windows {
                let inst = instance_registry
                    .iter()
                    .find(|(l, _)| l == &w.label)
                    .map(|(_, n)| format!("#{}", n))
                    .unwrap_or_else(|| "—".to_string());
                let backend = backend_window_ids
                    .iter()
                    .find(|(l, _)| l == &w.label)
                    .map(|(_, b)| b.as_str())
                    .unwrap_or("—");
                println!(
                    "  {:>4} {:30} kind={:?} hwnd={:?} visible={} iconic={} fg_seen={} backend={}",
                    inst, w.label, w.kind, w.hwnd, w.visible, w.iconic,
                    w.foregrounded_since_open, backend
                );
            }
        }
        println!();
        println!("Pool ({}): {:?}", pool.len(), pool);
        println!();
    } else {
        println!("(snapshot not received — server may be older than D.1)");
        println!();
    }

    println!("=== Live stream observed in {}s ===", OBSERVATION_WINDOW.as_secs());
    if stream.is_empty() {
        println!("(no events — instance is idle.)");
        return;
    }
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    for evt in &stream {
        *counts.entry(event_kind_name(evt)).or_insert(0) += 1;
    }
    println!("By kind:");
    for (kind, count) in &counts {
        println!("  {:>4}× {}", count, kind);
    }
    println!();
    println!("Full stream (oldest first):");
    for (i, evt) in stream.iter().enumerate() {
        println!("  [{}] {}", i, format_event(evt));
    }
}

#[cfg(target_os = "windows")]
fn event_kind_name(e: &Event) -> &'static str {
    match e {
        Event::Registered { .. } => "Registered",
        Event::Pong { .. } => "Pong",
        Event::ProcessSpawned { .. } => "ProcessSpawned",
        Event::ProcessExited { .. } => "ProcessExited",
        Event::LifecyclePhaseChanged { .. } => "LifecyclePhaseChanged",
        Event::WindowOpened { .. } => "WindowOpened",
        Event::WindowClosed { .. } => "WindowClosed",
        Event::WindowInstanceAssigned { .. } => "WindowInstanceAssigned",
        Event::WindowInstanceReleased { .. } => "WindowInstanceReleased",
        Event::PoolWindowAdded { .. } => "PoolWindowAdded",
        Event::PoolWindowRemoved { .. } => "PoolWindowRemoved",
        Event::BackendWindowIdRegistered { .. } => "BackendWindowIdRegistered",
        Event::BackendWindowIdUnregistered { .. } => "BackendWindowIdUnregistered",
        Event::DriftDetected { .. } => "DriftDetected",
        Event::HwndDriftDetected { .. } => "HwndDriftDetected",
        Event::CorrectiveWindowMove { .. } => "CorrectiveWindowMove",
        Event::HostShouldQuit { .. } => "HostShouldQuit",
        Event::Snapshot { .. } => "Snapshot",
        _ => "Other",
    }
}

#[cfg(target_os = "windows")]
fn format_event(e: &Event) -> String {
    // Compact one-liner per event. Falls back to JSON for variants
    // we don't have a custom formatter for so the operator still
    // sees the data.
    match e {
        Event::WindowOpened { label, kind, parent_label, version } => format!(
            "v={:>3} WindowOpened       label={} kind={:?} parent={:?}",
            version, label, kind, parent_label
        ),
        Event::WindowClosed { label, version } => format!(
            "v={:>3} WindowClosed       label={}", version, label
        ),
        Event::WindowInstanceAssigned { label, num, version } => format!(
            "v={:>3} InstanceAssigned   label={} num={}", version, label, num
        ),
        Event::HwndDriftDetected { kind, label, hwnd, severity, detail, version } => format!(
            "v={:>3} HwndDriftDetected  kind={:?} label={:?} hwnd={:?} severity={:?} — {}",
            version, kind, label, hwnd, severity, detail
        ),
        Event::HostShouldQuit { version } => format!(
            "v={:>3} HostShouldQuit", version
        ),
        other => serde_json::to_string(other).unwrap_or_else(|_| format!("{:?}", other)),
    }
}
