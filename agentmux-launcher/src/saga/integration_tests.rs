// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CPD-3 — coordinator + host pipe integration tests.
//
// Verifies the CPD-3 wire-up end-to-end: a saga's `IssueCmd::Host`
// dispatches through `HostPipe::send_command()` (the pre-CPD-3
// log-only branch), and a host-pipe send failure (read-half closed,
// simulating host crash) terminates the saga with `SagaFailed`
// instead of leaving it stuck in_flight.
//
// See `docs/specs/SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md` §4 PR3
// + §6 acceptance criteria.

use std::sync::Arc;

use agentmux_common::ipc::{Command, Event, HostFrame};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, Mutex};

use super::{run_coordinator, SagaCoordinator};
use crate::host_pipe::{make_shared_writer, BoxedWriter, HostPipe};
use crate::state::State;

fn make_state() -> Arc<Mutex<State>> {
    Arc::new(Mutex::new(State::default()))
}

/// Spin up a coordinator wired to a HostPipe that holds a duplex
/// stream's writer half. Returns the coordinator, the broadcast
/// sender for synthetic events, the broadcast witness, and the
/// duplex read half so callers can assert on dispatched frames or
/// drop the read half to simulate a host crash.
async fn make_coord_with_host_pipe() -> (
    Arc<SagaCoordinator>,
    broadcast::Sender<Event>,
    broadcast::Receiver<Event>,
    tokio::io::DuplexStream,
) {
    let (events_tx, _) = broadcast::channel::<Event>(64);
    let state = make_state();
    let host_pipe = Arc::new(HostPipe::new(events_tx.clone(), Arc::clone(&state)));

    // Wire a duplex pair as the host's "write half"; the read half is
    // returned so the test can either consume framed Commands or drop
    // it to force the next write to fail.
    let (a, b) = tokio::io::duplex(16 * 1024);
    let boxed: BoxedWriter = Box::new(a);
    let _session = host_pipe.set_writer(make_shared_writer(boxed)).await;

    let coord = Arc::new(
        SagaCoordinator::new(events_tx.clone(), state)
            .with_host_pipe(Arc::clone(&host_pipe)),
    );
    let witness = events_tx.subscribe();
    let coord_rx = events_tx.subscribe();
    tokio::spawn(run_coordinator(Arc::clone(&coord), coord_rx));
    tokio::task::yield_now().await;
    (coord, events_tx, witness, b)
}

/// CPD-3 happy path: triggering F.5's pool-respawn saga issues a
/// `Command::SpawnPoolWindow { saga_id }` over the host pipe with the
/// real saga id injected. Verifies `apply_action` for `IssueCmd::Host`
/// is no longer log-only.
#[tokio::test]
async fn pool_respawn_saga_dispatches_spawn_pool_window_to_host_pipe() {
    let (_coord, events_tx, _witness, reader) = make_coord_with_host_pipe().await;

    // Fire the trigger.
    let _ = events_tx.send(Event::PoolWindowPromoted {
        label: "window-pool-abc".into(),
        version: 1,
    });

    // Read one HostFrame off the wire.
    let mut bufr = BufReader::new(reader);
    let mut line = String::new();
    let read = tokio::time::timeout(std::time::Duration::from_secs(2), bufr.read_line(&mut line))
        .await
        .expect("HostPipe write should arrive within 2s")
        .expect("read_line should succeed");
    assert!(read > 0, "expected at least one HostFrame on the wire");
    let frame: HostFrame =
        serde_json::from_str(line.trim_end()).expect("frame parses as HostFrame");

    match frame {
        HostFrame::Command(Command::SpawnPoolWindow { saga_id }) => {
            assert!(
                saga_id > 0,
                "coordinator should inject a real saga_id (>0), got {}",
                saga_id
            );
        }
        other => panic!("expected SpawnPoolWindow frame, got {:?}", other),
    }
}

/// CPD-3 failure path: when the host pipe's write fails (read half
/// closed — simulates host crash mid-saga), the saga terminates as
/// `SagaFailed` and is removed from `in_flight`. Verifies the
/// `Err(...)` arm in `apply_action`'s `host_pipe.send_command()`
/// branch.
#[tokio::test]
async fn host_pipe_send_failure_terminates_saga() {
    let (coord, events_tx, mut witness, reader) = make_coord_with_host_pipe().await;

    // Drop the read half — subsequent writes will fail with
    // BrokenPipe. The host-pipe wrapper translates this into
    // `HostPipeError::WriteFailed`.
    drop(reader);
    // Give the OS a moment to actually tear down the duplex peer.
    tokio::task::yield_now().await;

    // Fire the trigger. The saga's start() emits IssueCmd::Host;
    // apply_action calls host_pipe.send_command() which fails.
    let _ = events_tx.send(Event::PoolWindowPromoted {
        label: "window-pool-zzz".into(),
        version: 1,
    });

    // Round 7: host-send-failure no longer terminates the saga
    // immediately (round 4 design — the buffered retry path or the
    // saga's own timeout handles definitive failure to avoid
    // double-SagaFailed emission per reagent P1 round 6). PoolRespawn
    // uses the default 5s saga timeout; wait past that for the
    // deadline-task SagaFailed to fire.
    let mut saw_started: Option<u64> = None;
    let mut saw_failed_for: Option<u64> = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(7);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(100), witness.recv()).await {
            Ok(Ok(Event::SagaStarted { saga_id, name, .. })) => {
                if name == "pool_respawn_on_promote" {
                    saw_started = Some(saga_id);
                }
            }
            Ok(Ok(Event::SagaFailed { saga_id, .. })) => {
                if Some(saga_id) == saw_started {
                    saw_failed_for = Some(saga_id);
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }

    let saga_id = saw_started.expect("expected SagaStarted for pool_respawn_on_promote");
    assert_eq!(
        saw_failed_for,
        Some(saga_id),
        "expected SagaFailed for the started saga (pipe error or timeout)"
    );

    // Saga is no longer in_flight.
    let registry = coord.in_flight.lock().await;
    assert!(
        !registry.contains_key(&saga_id),
        "saga_id={} should be removed from in_flight after SagaFailed",
        saga_id
    );
}

/// CPD-3 — `Event::SagaActionFailed` (host-emitted via
/// `Command::ReportSagaActionFailed`) terminates the matching saga
/// even if no Report* event arrives. Avoids hung sagas waiting
/// forever on a Report* the host already gave up on.
#[tokio::test]
async fn saga_action_failed_event_terminates_matching_saga() {
    let (coord, events_tx, mut witness, _reader) = make_coord_with_host_pipe().await;

    // Start a saga.
    let _ = events_tx.send(Event::PoolWindowPromoted {
        label: "window-pool-q".into(),
        version: 1,
    });

    // Capture saga_id from SagaStarted.
    let saga_id = {
        let mut id = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(100), witness.recv())
                .await
            {
                Ok(Ok(Event::SagaStarted {
                    saga_id,
                    name,
                    ..
                })) if name == "pool_respawn_on_promote" => {
                    id = Some(saga_id);
                    break;
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        id.expect("expected SagaStarted for pool_respawn_on_promote")
    };

    // Host reports the action failed.
    let _ = events_tx.send(Event::SagaActionFailed {
        saga_id,
        reason: "spawn_pool_window: window not found".into(),
        version: 99,
    });

    // Expect SagaFailed for this saga.
    let mut saw_failed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(100), witness.recv()).await {
            Ok(Ok(Event::SagaFailed { saga_id: id, .. })) if id == saga_id => {
                saw_failed = true;
                break;
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }
    assert!(saw_failed, "expected SagaFailed for saga_id={}", saga_id);

    // Saga removed from in_flight.
    let registry = coord.in_flight.lock().await;
    assert!(
        !registry.contains_key(&saga_id),
        "saga_id={} should be removed from in_flight after SagaActionFailed",
        saga_id
    );
}
