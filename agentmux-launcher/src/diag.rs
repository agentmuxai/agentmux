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
// Phase D.1 — `Command::GetSnapshot` is sent right after Register
// to capture the launcher's canonical state in one round-trip; the
// reply (`Event::Snapshot`) is printed prominently before the live
// event stream. Phase D.2 + D.3 will add a persisted event log + a
// `since: u64` parameter for delta replay; until then a snapshot is
// "as-of-now" only.

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

    // Phase D.3 — request a replay of all retained events
    // (`since: 0` means everything in the in-memory ring). Useful
    // for operators to see the full event history of the running
    // launcher without having to wait for new activity. The reply
    // is an `Event::EventList`; rendered in the summary below.
    let mut buf = serde_json::to_vec(&Command::GetEvents { since: 0 })
        .map_err(|e| format!("serialize GetEvents: {}", e))?;
    buf.push(b'\n');
    write_half.write_all(&buf).await
        .map_err(|e| format!("send GetEvents: {}", e))?;
    write_half.flush().await
        .map_err(|e| format!("flush GetEvents: {}", e))?;

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

/// Phase E.7 — `agentmux.exe --diag srv` Tool client.
///
/// Connects to the srv pipe (`\\.\pipe\agentmux-{hash}\srv-command`),
/// registers as a Tool client, sends `GetSrvSnapshot` + `GetEvents`,
/// captures events for the same 2 s observation window as
/// `--diag wrr`, prints a human-readable summary, exits.
///
/// Provides operator visibility into the srv reducer's canonical
/// state (workspaces / tabs / blocks / windows / saga lifecycle) +
/// recent event activity. Mirrors `run_wrr_diag` for the launcher;
/// the differences are the pipe path, the snapshot variant
/// (`Event::SrvSnapshot`), and the formatter for srv-specific event
/// kinds.
#[cfg(target_os = "windows")]
pub async fn run_srv_diag(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    let version = env!("CARGO_PKG_VERSION");
    let is_dev = cfg!(debug_assertions);

    let paths = crate::data_dir::resolve_paths(launcher_exe_dir, version, is_dev)
        .map_err(|e| format!("path resolution failed: {}", e))?;
    let dir_hash = crate::hash::data_dir_hash16(&paths.data_dir);
    let pipe_path = crate::ipc::srv_pipe_name(&dir_hash);

    println!("AgentMux srv diagnostic — connecting to {}", pipe_path);
    println!("Data dir: {}", paths.data_dir.display());

    let client = ClientOptions::new().open(&pipe_path).map_err(|e| {
        format!(
            "could not open srv pipe {}: {} (is AgentMux running for this data dir? \
             srv may not be bound in `task dev` mode)",
            pipe_path, e
        )
    })?;
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
    write_half
        .write_all(&buf)
        .await
        .map_err(|e| format!("send Register: {}", e))?;
    write_half
        .flush()
        .await
        .map_err(|e| format!("flush Register: {}", e))?;

    // Phase E.1b — request the srv reducer's canonical state.
    let mut buf = serde_json::to_vec(&Command::GetSrvSnapshot)
        .map_err(|e| format!("serialize GetSrvSnapshot: {}", e))?;
    buf.push(b'\n');
    write_half
        .write_all(&buf)
        .await
        .map_err(|e| format!("send GetSrvSnapshot: {}", e))?;
    write_half
        .flush()
        .await
        .map_err(|e| format!("flush GetSrvSnapshot: {}", e))?;

    // Phase D.3 / E.7 — request a replay of all retained events from
    // the srv side's in-memory ring + on-disk event log. Useful for
    // operators wanting recent srv reducer activity (saga lifecycle
    // events, workspace/tab/block mutations) without having to wait
    // for new activity.
    let mut buf = serde_json::to_vec(&Command::GetEvents { since: 0 })
        .map_err(|e| format!("serialize GetEvents: {}", e))?;
    buf.push(b'\n');
    write_half
        .write_all(&buf)
        .await
        .map_err(|e| format!("send GetEvents: {}", e))?;
    write_half
        .flush()
        .await
        .map_err(|e| format!("flush GetEvents: {}", e))?;

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
            Ok(Ok(Some(line))) => match serde_json::from_str::<Event>(&line) {
                Ok(evt) => events.push(evt),
                Err(e) => eprintln!("[warn] could not parse event: {} ({})", e, line),
            },
            Ok(Ok(None)) => {
                eprintln!("[warn] srv pipe closed before observation window elapsed");
                break;
            }
            Ok(Err(e)) => {
                return Err(format!("read error: {}", e));
            }
            Err(_) => break,
        }
    }

    // Best-effort Goodbye so the srv server marks our PID Exited
    // (same rationale as run_wrr_diag).
    let goodbye = match serde_json::to_vec(&Command::Goodbye) {
        Ok(mut b) => {
            b.push(b'\n');
            b
        }
        Err(_) => Vec::new(),
    };
    if !goodbye.is_empty() {
        let _ = write_half.write_all(&goodbye).await;
        let _ = write_half.flush().await;
    }

    print_srv_summary(&events);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub async fn run_srv_diag(_launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    Err("--diag srv is Windows-only (Phase 7 will add Unix domain socket parity)".to_string())
}

#[cfg(target_os = "windows")]
fn print_srv_summary(events: &[Event]) {
    let snapshot: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::SrvSnapshot { .. }))
        .collect();
    let replay: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::EventList { .. }))
        .collect();
    let stream: Vec<&Event> = events
        .iter()
        .filter(|e| !matches!(e, Event::SrvSnapshot { .. } | Event::EventList { .. }))
        .collect();

    if let Some(Event::SrvSnapshot {
        version,
        lifecycle,
        workspaces,
        tabs,
        active_tabs,
        blocks,
    }) = snapshot.last().copied()
    {
        println!("=== SrvSnapshot (event_version={}) ===", version);
        println!("Lifecycle: {:?}", lifecycle);
        println!();
        println!("Workspaces ({}):", workspaces.len());
        if workspaces.is_empty() {
            println!("  (none)");
        } else {
            for (id, name) in workspaces {
                let active = active_tabs
                    .iter()
                    .find(|(ws, _)| ws == id)
                    .map(|(_, t)| t.as_str())
                    .unwrap_or("—");
                let tab_count = tabs.iter().filter(|(_, ws, _)| ws == id).count();
                println!(
                    "  {:36} name={:<20} tabs={} active={}",
                    id, name, tab_count, active
                );
            }
        }
        println!();
        println!("Tabs ({}):", tabs.len());
        if tabs.is_empty() {
            println!("  (none)");
        } else {
            for (tab_id, ws_id, name) in tabs {
                let block_count = blocks.iter().filter(|(_, t)| t == tab_id).count();
                println!(
                    "  {:36} ws={:36} name={:<16} blocks={}",
                    tab_id, ws_id, name, block_count
                );
            }
        }
        println!();
        println!("Blocks ({}):", blocks.len());
        if blocks.is_empty() {
            println!("  (none)");
        } else {
            for (block_id, tab_id) in blocks {
                println!("  {:36} tab={}", block_id, tab_id);
            }
        }
        println!();
    } else {
        println!("(SrvSnapshot not received — srv pipe may not be bound, or older srv build)");
        println!();
    }

    if let Some(Event::EventList {
        events: replay_events,
        version,
    }) = replay.last().copied()
    {
        println!(
            "=== EventList replay (event_version={}, {} event(s)) ===",
            version,
            replay_events.len()
        );
        if replay_events.is_empty() {
            println!("(empty — srv ring + event log contained no events)");
        } else {
            // Show the last 20 events to keep output manageable.
            let to_show = replay_events.iter().rev().take(20).collect::<Vec<_>>();
            let n = replay_events.len();
            let skipped = n.saturating_sub(to_show.len());
            if skipped > 0 {
                println!("(showing last 20 of {})", n);
            }
            for (i, evt) in to_show.iter().rev().enumerate() {
                println!("  [{}] {}", skipped + i, format_srv_event(evt));
            }
        }
        println!();
    }

    // Saga activity is the most operator-relevant signal here.
    use std::collections::BTreeMap;
    let mut saga_counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let stream_or_replay = events
        .iter()
        .filter(|e| !matches!(e, Event::SrvSnapshot { .. }))
        .flat_map(|e| match e {
            Event::EventList { events: inner, .. } => inner.iter().collect::<Vec<_>>(),
            other => vec![other],
        });
    for evt in stream_or_replay {
        match evt {
            Event::SagaStarted { .. } => *saga_counts.entry("SagaStarted").or_insert(0) += 1,
            Event::SagaCompleted { .. } => {
                *saga_counts.entry("SagaCompleted").or_insert(0) += 1
            }
            Event::SagaFailed { .. } => *saga_counts.entry("SagaFailed").or_insert(0) += 1,
            _ => {}
        }
    }
    if !saga_counts.is_empty() {
        println!("=== Saga lifecycle (across snapshot + replay + stream) ===");
        for (kind, count) in &saga_counts {
            println!("  {:>4}× {}", count, kind);
        }
        println!();
    }

    println!(
        "=== Live stream observed in {}s ===",
        OBSERVATION_WINDOW.as_secs()
    );
    if stream.is_empty() {
        println!("(no events — srv reducer is idle.)");
        return;
    }
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    for evt in &stream {
        *counts.entry(srv_event_kind_name(evt)).or_insert(0) += 1;
    }
    println!("By kind:");
    for (kind, count) in &counts {
        println!("  {:>4}× {}", count, kind);
    }
    println!();
    println!("Full stream (oldest first):");
    for (i, evt) in stream.iter().enumerate() {
        println!("  [{}] {}", i, format_srv_event(evt));
    }
}

#[cfg(target_os = "windows")]
fn srv_event_kind_name(e: &Event) -> &'static str {
    match e {
        Event::Registered { .. } => "Registered",
        Event::ProcessSpawned { .. } => "ProcessSpawned",
        Event::ProcessExited { .. } => "ProcessExited",
        Event::LifecyclePhaseChanged { .. } => "LifecyclePhaseChanged",
        Event::SagaStarted { .. } => "SagaStarted",
        Event::SagaCompleted { .. } => "SagaCompleted",
        Event::SagaFailed { .. } => "SagaFailed",
        Event::WorkspaceCreated { .. } => "WorkspaceCreated",
        Event::WorkspaceDeleted { .. } => "WorkspaceDeleted",
        Event::WorkspaceRenamed { .. } => "WorkspaceRenamed",
        Event::WorkspaceMetaUpdated { .. } => "WorkspaceMetaUpdated",
        Event::TabCreated { .. } => "TabCreated",
        Event::TabDeleted { .. } => "TabDeleted",
        Event::TabRenamed { .. } => "TabRenamed",
        Event::TabReordered { .. } => "TabReordered",
        Event::TabsReorderedBulk { .. } => "TabsReorderedBulk",
        Event::TabMoved { .. } => "TabMoved",
        Event::TabMetaUpdated { .. } => "TabMetaUpdated",
        Event::ActiveTabChanged { .. } => "ActiveTabChanged",
        Event::BlockCreated { .. } => "BlockCreated",
        Event::BlockDeleted { .. } => "BlockDeleted",
        Event::BlockMoved { .. } => "BlockMoved",
        Event::BlockMetaUpdated { .. } => "BlockMetaUpdated",
        Event::SrvWindowOpened { .. } => "SrvWindowOpened",
        Event::SrvWindowClosed { .. } => "SrvWindowClosed",
        Event::SrvWindowWorkspaceChanged { .. } => "SrvWindowWorkspaceChanged",
        Event::SrvSnapshot { .. } => "SrvSnapshot",
        Event::EventList { .. } => "EventList",
        Event::Error { .. } => "Error",
        _ => "Other",
    }
}

#[cfg(target_os = "windows")]
fn format_srv_event(e: &Event) -> String {
    match e {
        Event::SagaStarted { saga_id, name, version } => {
            format!("v={:>3} SagaStarted        id={} name={}", version, saga_id, name)
        }
        Event::SagaCompleted { saga_id, version } => {
            format!("v={:>3} SagaCompleted      id={}", version, saga_id)
        }
        Event::SagaFailed { saga_id, reason, version } => {
            format!("v={:>3} SagaFailed         id={} reason={}", version, saga_id, reason)
        }
        Event::WorkspaceCreated { workspace_id, name, version } => {
            format!("v={:>3} WorkspaceCreated   id={} name={}", version, workspace_id, name)
        }
        Event::WorkspaceDeleted { workspace_id, version } => {
            format!("v={:>3} WorkspaceDeleted   id={}", version, workspace_id)
        }
        Event::TabCreated { workspace_id, tab_id, name, version } => {
            format!("v={:>3} TabCreated         tab={} ws={} name={}", version, tab_id, workspace_id, name)
        }
        Event::TabMoved { tab_id, src_workspace_id, dst_workspace_id, dst_index, .. } => format!(
            "v=??? TabMoved           tab={} {} → {} idx={}",
            tab_id, src_workspace_id, dst_workspace_id, dst_index
        ),
        Event::BlockMoved { block_id, src_tab_id, dst_tab_id, dst_index, version } => format!(
            "v={:>3} BlockMoved         blk={} {} → {} idx={}",
            version, block_id, src_tab_id, dst_tab_id, dst_index
        ),
        Event::Error { code, message, version, .. } => {
            format!("v={:>3} Error              code={:?} msg={}", version, code, message)
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| format!("{:?}", other)),
    }
}

#[cfg(target_os = "windows")]
fn print_summary(events: &[Event]) {
    // Phase D.1 — pull the Snapshot out and print it first as the
    // canonical "state now" view.
    // Phase D.3 — pull the EventList replay out next; what's left
    // is the live broadcast stream observed during the 2s window.
    let snapshot: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::Snapshot { .. }))
        .collect();
    let replay: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::EventList { .. }))
        .collect();
    let stream: Vec<&Event> = events
        .iter()
        .filter(|e| {
            !matches!(
                e,
                Event::Snapshot { .. } | Event::EventList { .. }
            )
        })
        .collect();

    // Pick the LATEST snapshot in the captured set. The IPC server
    // broadcasts snapshots to all subscribers, so concurrent diag/Tool
    // clients can produce multiple Snapshot events in this window;
    // using `.first()` would show the oldest (potentially another
    // client's stale view) rather than ours. `.last()` biases toward
    // freshness — our own snapshot reply is monotonically the latest
    // among captured events on the broadcast bus. (codex P2 PR #607.)
    if let Some(Event::Snapshot {
        version,
        lifecycle,
        windows,
        pool,
        instance_registry,
        backend_window_ids,
        monitors,
    }) = snapshot.last().copied()
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

    // Phase D.3 — replay slice from the launcher's in-memory event
    // ring. Useful for operators wanting recent history; for
    // resyncing subscribers, this is everything since their last
    // seen version.
    if let Some(Event::EventList { events: replay_events, version }) = replay.last().copied() {
        println!("=== EventList replay (event_version={}, {} event(s)) ===", version, replay_events.len());
        if replay_events.is_empty() {
            println!("(empty — launcher's in-memory ring contained no events with version > 0)");
        } else {
            for (i, evt) in replay_events.iter().enumerate() {
                println!("  [{}] {}", i, format_event(evt));
            }
        }
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
        Event::EventList { .. } => "EventList",
        Event::SagaStarted { .. } => "SagaStarted",
        Event::SagaCompleted { .. } => "SagaCompleted",
        Event::SagaFailed { .. } => "SagaFailed",
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
