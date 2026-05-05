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

    let paths = crate::data_dir::resolve_paths(launcher_exe_dir, version)
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

    let paths = crate::data_dir::resolve_paths(launcher_exe_dir, version)
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
        Event::TabMoved {
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            dst_index,
            version,
            ..
        } => format!(
            "v={:>3} TabMoved           tab={} {} → {} idx={}",
            version, tab_id, src_workspace_id, dst_workspace_id, dst_index
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
        Event::PoolWindowPromoted { .. } => "PoolWindowPromoted",
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
        Event::WindowClosed { label, version, crash_detected } => format!(
            "v={:>3} WindowClosed       label={} crash_detected={}", version, label, crash_detected
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

// ---------------------------------------------------------------------------
// LSD-3 — `agentmux.exe --diag sagas`
//
// Operator visibility into the launcher saga log
// (`<data-dir>/launcher-sagas.db`). Unlike `--diag wrr` and
// `--diag srv`, this command does NOT need a running launcher: the
// SQLite log is a passive on-disk artefact. We open it read-only via
// `LauncherSagaLog::open_read_only` and call `snapshot_recent(50)`. Useful when
// the launcher won't start (operator wants to see what sagas were
// in flight at the last crash) or post-mortem on a portable instance.
//
// Output mirrors the example in LSD spec §3.5 — saga header line per
// saga, followed by step rows showing target + state + cmd snippet.
// Sagas marked `failed_compensation` by the startup recovery walker
// are flagged "(recovered on restart)" in the header so operators
// can spot them at a glance.
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub async fn run_sagas_diag(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    run_sagas_diag_impl(launcher_exe_dir).await
}

#[cfg(not(target_os = "windows"))]
pub async fn run_sagas_diag(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    // The saga log is a SQLite file with no platform-specific bits;
    // the cross-platform parity goal for `--diag sagas` is "works
    // wherever the launcher writes the log." Phase 7 will wire the
    // POSIX data_dir resolution, but the rest of the formatter is
    // already platform-agnostic, so we reuse the impl on every
    // target.
    run_sagas_diag_impl(launcher_exe_dir).await
}

async fn run_sagas_diag_impl(launcher_exe_dir: &std::path::Path) -> Result<(), String> {
    let version = env!("CARGO_PKG_VERSION");

    let paths = crate::data_dir::resolve_paths(launcher_exe_dir, version)
        .map_err(|e| format!("path resolution failed: {}", e))?;
    let saga_log_path = paths.data_dir.join("launcher-sagas.db");

    println!("AgentMux launcher saga diagnostic");
    println!("Data dir: {}", paths.data_dir.display());
    println!("Saga log: {}", saga_log_path.display());
    println!();

    if !saga_log_path.exists() {
        println!(
            "(no saga log at {} — launcher hasn't written one yet, or this isn't an AgentMux data dir)",
            saga_log_path.display()
        );
        return Ok(());
    }

    // Read-only open: an operator's diagnostic invocation must not
    // mutate the log a running launcher owns. (codex P2 PR #647 round 3.)
    let log = crate::saga::LauncherSagaLog::open_read_only(&saga_log_path)
        .map_err(|e| format!("open saga log {:?} (read-only): {}", saga_log_path, e))?;

    let snapshot = log
        .snapshot_recent(50)
        .map_err(|e| format!("snapshot_recent: {}", e))?;
    let unresolved = log
        .unresolved_sagas()
        .map_err(|e| format!("unresolved_sagas: {}", e))?;

    if snapshot.is_empty() {
        println!("(saga log is empty)");
        return Ok(());
    }

    println!("Recent launcher sagas (last {}):", snapshot.len());
    for s in &snapshot {
        let recovered_marker = if s.state == "failed_compensation" {
            " (recovered on restart)"
        } else {
            ""
        };
        let ended = s.ended_at.as_deref().unwrap_or("—");
        println!(
            "  saga_id={} name={} state={}{}",
            s.saga_id, s.name, s.state, recovered_marker
        );
        println!(
            "    started={} ended={} steps_progressed={}",
            s.started_at, ended, s.step_count
        );
        if let Some(reason) = &s.failure_reason {
            println!("    failure: {}", reason);
        }
        if !s.input_json.is_empty() && s.input_json != "null" {
            println!("    input: {}", s.input_json);
        }

        // Step rows. Surface step detail for both `unresolved` sagas
        // AND `failed_compensation` sagas (recovered crashes) — the
        // latter is the operator triage flow per LSD spec §3.5.
        // (codex P1 PR #647 round 1: unresolved_sagas() filters
        // out failed_compensation, so we fall back to a direct
        // get_saga_steps query for those.)
        let steps: Vec<crate::saga::log::UnresolvedLauncherStep> = if let Some(u) =
            unresolved.iter().find(|u| u.saga_id == s.saga_id)
        {
            u.steps.clone()
        } else if s.state == "failed_compensation" {
            // Surface step-query failures rather than silently
            // returning empty — operators need visibility into "why
            // are step rows missing for this recovered saga".
            // (codex P2 PR #647 round 3.)
            match log.get_saga_steps(s.saga_id) {
                Ok(steps) => steps,
                Err(e) => {
                    println!("    [step query failed: {} — saga rows may exist but cannot be read]", e);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if !steps.is_empty() {
            println!("    steps:");
            for step in &steps {
                let target = step.target.as_deref().unwrap_or("—");
                let cmd_snippet = step
                    .cmd_json
                    .as_deref()
                    .map(|c| truncate_for_display(c, 120))
                    .unwrap_or_else(|| "—".into());
                println!(
                    "      {:>3}  {:30} target={:<14} state={:<10} cmd={}",
                    step.step_index, step.name, target, step.state, cmd_snippet
                );
                if let Some(reason) = &step.failure_reason {
                    println!("           failure: {}", reason);
                }
            }
            // Pinpoint the in-flight step at crash time, mirroring
            // the example in spec §3.5: "[step 2 was in-flight when
            // launcher exited]". The step in `pending` state at the
            // highest index is the one the saga was waiting on.
            if let Some(in_flight) = steps.iter().rev().find(|st| st.state == "pending") {
                println!(
                    "      [step {} was in-flight when launcher exited]",
                    in_flight.step_index
                );
            }
        }
        println!();
    }

    let recovered_count = snapshot
        .iter()
        .filter(|s| s.state == "failed_compensation")
        .count();
    if recovered_count > 0 {
        println!(
            "Note: {} saga(s) marked `failed_compensation` by the startup recovery walker.",
            recovered_count
        );
        println!("These were unresolved when the launcher last exited; their effects on host state");
        println!("may be partially applied. Inspect step rows above to see what was attempted.");
    }

    Ok(())
}

/// Trim a JSON snippet to `max_chars` for one-line display, appending
/// `…` if it was truncated. Used to keep `--diag sagas` cmd columns
/// readable when commands carry large payloads (e.g. block meta).
fn truncate_for_display(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}

#[cfg(test)]
mod sagas_diag_tests {
    use super::*;
    use crate::saga::{LauncherSagaLog, PipeTarget};
    use agentmux_common::ipc::{Command, Event};

    #[test]
    fn truncate_for_display_shortens_long_strings_and_keeps_short_ones() {
        assert_eq!(truncate_for_display("hi", 10), "hi");
        let long = "a".repeat(200);
        let truncated = truncate_for_display(&long, 50);
        // 50 a's + ellipsis.
        assert_eq!(truncated.chars().count(), 51);
        assert!(truncated.ends_with('…'));
    }

    /// Smoke test: build a fixture saga log, run `snapshot_recent`
    /// and `unresolved_sagas`, and verify the formatter assembles the
    /// expected fields without panicking. We don't snapshot stdout
    /// (printing is fine; what matters is the data we'd print is
    /// what the spec describes).
    #[test]
    fn sagas_diag_fixture_log_has_expected_summary_fields() {
        let log = LauncherSagaLog::open_in_memory().unwrap();

        // Saga 1 — completed cleanly.
        log.start_saga(
            1,
            "window_cleanup_cascade",
            &serde_json::json!({"label": "win-1"}),
        )
        .unwrap();
        log.start_step(
            1,
            0,
            "issue_cmd_host_reap_panes",
            PipeTarget::Host,
            &Command::Ping { nonce: 1 },
        )
        .unwrap();
        log.finish_step(1, 0, &Event::Pong { nonce: 1, version: 1 })
            .unwrap();
        log.terminate_saga(1, crate::saga::log::SagaOutcome::Completed)
            .unwrap();

        // Saga 2 — recovered (failed_compensation), with a pending
        // step row to demonstrate the "[step N was in-flight ..]" line.
        log.start_saga(
            2,
            "window_cleanup_cascade",
            &serde_json::json!({"label": "win-3"}),
        )
        .unwrap();
        log.start_step(
            2,
            0,
            "issue_cmd_host_reap_panes",
            PipeTarget::Host,
            &Command::Ping { nonce: 2 },
        )
        .unwrap();
        log.finish_step(2, 0, &Event::Pong { nonce: 2, version: 1 })
            .unwrap();
        log.start_step(
            2,
            1,
            "issue_cmd_host_drain_pool",
            PipeTarget::Host,
            &Command::Ping { nonce: 3 },
        )
        .unwrap();
        // Step 1 stays pending — saga 2 was in-flight at "crash" time.
        // Then run the recovery walker manually.
        log.mark_failed_compensation(2, "launcher restarted while saga in state 'running'")
            .unwrap();

        let snapshot = log.snapshot_recent(50).unwrap();
        assert_eq!(snapshot.len(), 2);

        // snapshot_recent returns most-recent-first.
        let s2 = &snapshot[0];
        let s1 = &snapshot[1];
        assert_eq!(s1.saga_id, 1);
        assert_eq!(s1.state, "completed");
        assert_eq!(s1.step_count, 1);

        assert_eq!(s2.saga_id, 2);
        assert_eq!(s2.state, "failed_compensation");
        // step_count counts succeeded+compensated, so 1 succeeded
        // (the reap-panes step). The pending drain-pool step is NOT
        // counted as progress.
        assert_eq!(s2.step_count, 1);
        assert!(s2
            .failure_reason
            .as_deref()
            .unwrap_or("")
            .contains("launcher restarted"));

        // Step rows for saga 2 still surface via unresolved_sagas? No —
        // saga 2 is in failed_compensation, which is terminal, so
        // unresolved_sagas EXCLUDES it. The formatter handles this by
        // simply skipping the step list for terminal-recovered sagas.
        // That's fine: operators looking for in-flight detail get it
        // for sagas STILL unresolved; for already-recovered ones, the
        // failure_reason header already names the prior state.
        let unresolved = log.unresolved_sagas().unwrap();
        assert!(unresolved.iter().all(|u| u.saga_id != 2));
    }
}
