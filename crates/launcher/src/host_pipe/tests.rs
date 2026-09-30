// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CPD-2 — HostPipe unit tests.
//
// Uses `tokio::io::duplex` for an in-memory bidirectional stream
// that quacks like a real named-pipe writer. The HostPipe holds the
// write half; the test harness reads back from the read half line by
// line and asserts the framing + drain order match expectations.

use std::sync::Arc;
use std::time::Duration;

use agentmux_common::ipc::{Command, Event};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, Mutex};

use super::{
    make_shared_writer, BoxedWriter, HostFrame, HostPipe, HostPipeError, SharedWriter,
    DISCONNECT_TIMEOUT, PENDING_BUFFER_CAP,
};

// ---- harness ---------------------------------------------------------------

/// Build a (HostPipe, events_rx, mock_reader_factory). The reader
/// factory returns a fresh DuplexStream read half each time —
/// emulating a fresh host reconnect.
fn make_pipe() -> (HostPipe, broadcast::Receiver<Event>) {
    let (events_tx, events_rx) = broadcast::channel::<Event>(64);
    let state = Arc::new(Mutex::new(crate::state::State::default()));
    (HostPipe::new(events_tx, state), events_rx)
}

/// Build an in-memory duplex pair sized for normal-traffic tests.
/// 16 KiB is comfortable for ~64 framed Commands.
fn make_duplex() -> (SharedWriter, tokio::io::DuplexStream) {
    let (a, b) = tokio::io::duplex(16 * 1024);
    let boxed: BoxedWriter = Box::new(a);
    (make_shared_writer(boxed), b)
}

/// Read up to `n` newline-delimited lines and parse each as a
/// `HostFrame` if it has the `kind` discriminator, else as a raw
/// `Event` (round-4 wire format: events bypass HostFrame for host
/// parser compat).
async fn read_n_frames(reader: tokio::io::DuplexStream, n: usize) -> Vec<HostFrame> {
    let mut bufr = BufReader::new(reader);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let mut line = String::new();
        let read = bufr
            .read_line(&mut line)
            .await
            .expect("read_line should succeed");
        if read == 0 {
            break;
        }
        let trimmed = line.trim_end();
        // Try HostFrame envelope (commands) first; on failure parse
        // as raw Event (events go raw post-round-4).
        let frame = match serde_json::from_str::<HostFrame>(trimmed) {
            Ok(f) => f,
            Err(_) => match serde_json::from_str::<Event>(trimmed) {
                Ok(e) => HostFrame::Event(e),
                Err(e) => panic!("line {:?} parses as neither HostFrame nor Event: {}", trimmed, e),
            },
        };
        out.push(frame);
    }
    out
}

fn ping_event(version: u64) -> Event {
    Event::Pong { nonce: 1, version }
}

// ---- happy: connected → send_event writes JSON line ------------------------

#[tokio::test]
async fn connected_send_event_writes_line() {
    let (pipe, _) = make_pipe();
    let (writer, reader) = make_duplex();
    pipe.set_writer(writer).await;
    assert!(pipe.is_connected().await);

    let evt = ping_event(7);
    pipe.send_event(&evt).await.expect("send_event ok");

    let frames = read_n_frames(reader, 1).await;
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        HostFrame::Event(e) => assert_eq!(e, &evt),
        other => panic!("expected Event frame, got {:?}", other),
    }
}

// ---- happy: connected → send_command writes JSON line ----------------------

#[tokio::test]
async fn connected_send_command_writes_line() {
    let (pipe, _) = make_pipe();
    let (writer, reader) = make_duplex();
    pipe.set_writer(writer).await;

    let cmd = Command::SpawnPoolWindow { saga_id: 0 };
    pipe.send_command(&cmd).await.expect("send_command ok");

    let frames = read_n_frames(reader, 1).await;
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        HostFrame::Command(_) => {}
        other => panic!("expected Command frame, got {:?}", other),
    }
}

// ---- disconnected → send_command buffers -----------------------------------

#[tokio::test]
async fn disconnected_send_command_buffers() {
    let (pipe, _) = make_pipe();
    // No set_writer call — pipe starts in disconnected state.
    assert!(!pipe.is_connected().await);
    assert_eq!(pipe.pending_len().await, 0);

    pipe.send_command(&Command::SpawnPoolWindow { saga_id: 0 }).await.unwrap();
    pipe.send_command(&Command::ReapPanes {
        label: "main".into(),
        saga_id: 0,
    })
    .await
    .unwrap();

    assert_eq!(pipe.pending_len().await, 2);
    assert!(!pipe.is_connected().await);
}

// ---- reconnect → drain in FIFO order --------------------------------------
// Round 4: events bypass HostPipe (raw wire format, no pending buffer);
// only Commands participate in pending_buffer drain on reconnect.

#[tokio::test]
async fn reconnect_drains_in_fifo_order() {
    let (pipe, _) = make_pipe();
    // Two buffered Command frames.
    pipe.send_command(&Command::SpawnPoolWindow { saga_id: 0 }).await.unwrap();
    pipe.send_command(&Command::ReapPanes {
        label: "main".into(),
        saga_id: 0,
    })
    .await
    .unwrap();
    assert_eq!(pipe.pending_len().await, 2);

    let (writer, reader) = make_duplex();
    pipe.set_writer(writer).await;
    assert_eq!(pipe.pending_len().await, 0);
    assert!(pipe.is_connected().await);

    let frames = read_n_frames(reader, 2).await;
    assert_eq!(frames.len(), 2);
    // FIFO: SpawnPoolWindow, ReapPanes
    match &frames[0] {
        HostFrame::Command(Command::SpawnPoolWindow { saga_id: 0 }) => {}
        other => panic!("frame[0] mismatch: {:?}", other),
    }
    match &frames[1] {
        HostFrame::Command(Command::ReapPanes { label, .. }) => assert_eq!(label, "main"),
        other => panic!("frame[1] mismatch: {:?}", other),
    }
}

// ---- overflow: 65 buffered → oldest dropped --------------------------------

#[tokio::test]
async fn overflow_drops_oldest_command_emits_failed() {
    // The current saga_id_of() returns None for all variants until
    // CPD-1 lands. This test verifies the OVERFLOW PATH itself —
    // oldest is dropped, length stays at the cap. CPD-1 + CPD-3 will
    // extend this test to assert SagaFailed is emitted on the bus
    // once Command variants carry saga_id fields.
    let (pipe, _events_rx) = make_pipe();

    for _ in 0..PENDING_BUFFER_CAP {
        pipe.send_command(&Command::SpawnPoolWindow { saga_id: 0 }).await.unwrap();
    }
    assert_eq!(pipe.pending_len().await, PENDING_BUFFER_CAP);

    // 65th — should evict oldest. Buffer length stays at cap.
    pipe.send_command(&Command::ReapPanes {
        label: "overflow-trigger".into(),
        saga_id: 0,
    })
    .await
    .unwrap();
    assert_eq!(pipe.pending_len().await, PENDING_BUFFER_CAP);

    // Reconnect and verify the LAST frame in is the overflow trigger
    // (proves FIFO eviction worked: oldest was popped, newest is at
    // the tail).
    let (writer, reader) = make_duplex();
    pipe.set_writer(writer).await;
    let frames = read_n_frames(reader, PENDING_BUFFER_CAP).await;
    assert_eq!(frames.len(), PENDING_BUFFER_CAP);
    match frames.last().unwrap() {
        HostFrame::Command(Command::ReapPanes { label, .. }) => {
            assert_eq!(label, "overflow-trigger");
        }
        other => panic!("expected ReapPanes overflow-trigger at tail, got {:?}", other),
    }
}

// ---- 30s disconnect → all buffered dropped --------------------------------

#[tokio::test]
async fn disconnect_timeout_drops_all_pending() {
    let (pipe, _events_rx) = make_pipe();
    // First, connect + disconnect to arm the timer.
    let (writer, _reader) = make_duplex();
    pipe.set_writer(writer).await;
    pipe.clear_writer().await;
    assert!(!pipe.is_connected().await);

    // Buffer some Command frames (events bypass HostPipe — round 4).
    pipe.send_command(&Command::SpawnPoolWindow { saga_id: 0 }).await.unwrap();
    pipe.send_command(&Command::ReapPanes {
        label: "a".into(),
        saga_id: 0,
    })
    .await
    .unwrap();
    assert_eq!(pipe.pending_len().await, 2);

    // Rewind the timer past 30s, then trigger a fresh send to invoke
    // the expiry check.
    pipe.rewind_disconnect_timer(DISCONNECT_TIMEOUT + Duration::from_secs(1))
        .await;
    pipe.send_command(&Command::SpawnPoolWindow { saga_id: 0 }).await.unwrap();

    // After expiry: prior 2 are dropped; fresh send is rebuffered
    // (timer was cleared by expire path; subsequent send sees a
    // None disconnected_at and proceeds to buffer normally).
    assert_eq!(pipe.pending_len().await, 1);
}

// ---- write failure → re-arms disconnect, re-buffers -----------------------

#[tokio::test]
async fn write_failure_clears_writer_and_rebuffers() {
    let (pipe, _) = make_pipe();
    let (writer, reader) = make_duplex();
    // Drop the read half — subsequent writes should fail with
    // BrokenPipe / ConnectionAborted.
    drop(reader);
    pipe.set_writer(writer).await;

    // Round 4: events bypass HostPipe pending-buffer semantics, so use
    // a Command to exercise the rebuffer-on-write-failure path.
    let result = pipe
        .send_command(&Command::SpawnPoolWindow { saga_id: 0 })
        .await;
    assert!(result.is_err(), "send_command should fail when peer dropped");
    // After a write failure the pipe should have flipped to
    // disconnected and re-buffered the frame.
    assert!(!pipe.is_connected().await);
    assert_eq!(pipe.pending_len().await, 1);
}

// ---- HostFrame envelope round-trips through serde --------------------------

#[test]
fn host_frame_roundtrip_command_and_event() {
    let cmd_frame = HostFrame::Command(Command::SpawnPoolWindow { saga_id: 0 });
    let json = serde_json::to_string(&cmd_frame).unwrap();
    assert!(json.contains("\"kind\":\"command\""));
    let back: HostFrame = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, HostFrame::Command(Command::SpawnPoolWindow { saga_id: 0 })));

    let evt_frame = HostFrame::Event(ping_event(42));
    let json = serde_json::to_string(&evt_frame).unwrap();
    assert!(json.contains("\"kind\":\"event\""));
    let back: HostFrame = serde_json::from_str(&json).unwrap();
    if let HostFrame::Event(Event::Pong { version, .. }) = back {
        assert_eq!(version, 42);
    } else {
        panic!("wrong frame after round-trip");
    }
}

// ---- send_event during connected state surfaces serialize errors as Err ----

#[tokio::test]
async fn send_event_via_connected_writer_completes_writeflushed() {
    // Sanity: send_event drives flush() so a slow reader can still
    // pick up the frame on its next read. Round 4: events go on the
    // wire as raw Event JSON (no HostFrame envelope) for host-parser
    // compat — verify each line parses directly as Event.
    let (pipe, _) = make_pipe();
    let (writer, reader) = make_duplex();
    pipe.set_writer(writer).await;

    pipe.send_event(&ping_event(1)).await.unwrap();
    pipe.send_event(&ping_event(2)).await.unwrap();
    pipe.send_event(&ping_event(3)).await.unwrap();

    // Drop the writer (clear) to close the read side cleanly.
    pipe.clear_writer().await;

    let events = collect_events_until_eof(reader).await;
    assert_eq!(events.len(), 3);
    for (i, e) in events.iter().enumerate() {
        match e {
            Event::Pong { version, .. } => {
                assert_eq!(*version, (i + 1) as u64);
            }
            other => panic!("unexpected event at index {}: {:?}", i, other),
        }
    }
}

async fn collect_events_until_eof(reader: tokio::io::DuplexStream) -> Vec<Event> {
    let mut bufr = BufReader::new(reader);
    let mut out = Vec::new();
    loop {
        let mut line = String::new();
        let n = bufr.read_line(&mut line).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        if let Ok(e) = serde_json::from_str::<Event>(line.trim_end()) {
            out.push(e);
        }
    }
    out
}

// ---- clear_writer is idempotent --------------------------------------------

#[tokio::test]
async fn clear_writer_idempotent() {
    let (pipe, _) = make_pipe();
    pipe.clear_writer().await; // no writer set; should be a no-op
    assert!(!pipe.is_connected().await);
    assert_eq!(pipe.pending_len().await, 0);

    let (writer, _reader) = make_duplex();
    pipe.set_writer(writer).await;
    pipe.clear_writer().await;
    pipe.clear_writer().await;
    assert!(!pipe.is_connected().await);
}

// ---- writes during reconnect are visible to the new reader -----------------

#[tokio::test]
async fn frames_buffered_during_disconnect_arrive_after_reconnect() {
    // Round 4: events bypass HostPipe pending-buffer semantics; only
    // Commands buffer + drain on reconnect. Test the reconnect-drain
    // path with two distinct Commands.
    let (pipe, _) = make_pipe();
    // Connect, disconnect.
    let (w1, _r1) = make_duplex();
    pipe.set_writer(w1).await;
    pipe.clear_writer().await;

    // Buffer two Commands.
    pipe.send_command(&Command::SpawnPoolWindow { saga_id: 10 })
        .await
        .unwrap();
    pipe.send_command(&Command::SpawnPoolWindow { saga_id: 11 })
        .await
        .unwrap();

    // Reconnect with a fresh duplex.
    let (w2, r2) = make_duplex();
    pipe.set_writer(w2).await;

    let frames = read_n_frames(r2, 2).await;
    assert_eq!(frames.len(), 2);
    match (&frames[0], &frames[1]) {
        (
            HostFrame::Command(Command::SpawnPoolWindow { saga_id: s1 }),
            HostFrame::Command(Command::SpawnPoolWindow { saga_id: s2 }),
        ) => {
            assert_eq!(*s1, 10);
            assert_eq!(*s2, 11);
        }
        other => panic!("expected two SpawnPoolWindow frames, got {:?}", other),
    }
}

// ---- shared writer holds arbitrary async-write trait objects -------------
// (sanity: this is just a shape check that the trait object holds a
// raw byte writer for non-windows test execution.)

#[tokio::test]
async fn shared_writer_holds_arbitrary_async_write() {
    let (pipe, _) = make_pipe();
    let (raw_a, mut raw_b) = tokio::io::duplex(1024);
    let boxed: BoxedWriter = Box::new(raw_a);
    pipe.set_writer(make_shared_writer(boxed)).await;
    pipe.send_event(&ping_event(0)).await.unwrap();

    let mut buf = vec![0u8; 256];
    let n = tokio::io::AsyncReadExt::read(&mut raw_b, &mut buf)
        .await
        .unwrap();
    assert!(n > 0, "expected at least one frame on the wire");
    let line = std::str::from_utf8(&buf[..n]).unwrap().trim_end();
    // Round 4: events are written as raw Event JSON (no HostFrame
    // envelope) for host parser compat. Either format must parse.
    let parsed_as_event = serde_json::from_str::<Event>(line).is_ok();
    let parsed_as_host_frame = serde_json::from_str::<HostFrame>(line).is_ok();
    assert!(
        parsed_as_event || parsed_as_host_frame,
        "line should parse as Event or HostFrame: {:?}",
        line
    );
}

// ---- fail-fast (no-buffer) sends -----------------------------------------
// SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1: the UI-liveness prober must never
// have its probe buffered while the host is down — a buffered probe either
// expires unreported (no saga_id) or reaches a FUTURE host, and either way
// the prober's outstanding-nonce bookkeeping would age into a false
// "UI thread did not pump" miss (reagent P1, PR #2106 round 2).

#[tokio::test]
async fn try_send_no_buffer_fails_fast_and_buffers_nothing_while_disconnected() {
    let (pipe, _) = make_pipe();
    assert!(!pipe.is_connected().await);

    let res = pipe
        .try_send_command_no_buffer(&Command::ProbeUiThread { nonce: 7 })
        .await;
    assert!(
        matches!(res, Err(HostPipeError::HostNotConnected)),
        "disconnected fail-fast send must error, got {:?}",
        res
    );
    assert_eq!(
        pipe.pending_len().await,
        0,
        "fail-fast send must not leave a frame in the pending buffer"
    );
}

#[tokio::test]
async fn try_send_no_buffer_delivers_while_connected() {
    let (pipe, _) = make_pipe();
    let (writer, reader) = make_duplex();
    pipe.set_writer(writer).await;

    pipe.try_send_command_no_buffer(&Command::ProbeUiThread { nonce: 9 })
        .await
        .expect("connected fail-fast send must succeed");
    assert_eq!(pipe.pending_len().await, 0);

    let frames = read_n_frames(reader, 1).await;
    assert!(
        matches!(frames.as_slice(), [HostFrame::Command(Command::ProbeUiThread { nonce: 9 })]),
        "probe must arrive on the wire, got {:?}",
        frames
    );
}

#[tokio::test]
async fn try_send_no_buffer_write_failure_purges_the_rebuffered_probe() {
    // reagent P1 (PR #2106 round 3): a write failure on an INSTALLED writer
    // takes send_frame's error branch, which re-buffers the frame as part
    // of clear-writer handling (proven by write_failure_clears_writer_and_
    // rebuffers above, pending_len == 1). The no-buffer contract requires
    // the opposite for probes: Err AND an empty buffer — a stale probe must
    // never drain into a future host.
    let (pipe, _) = make_pipe();
    let (writer, reader) = make_duplex();
    drop(reader); // writes will fail with BrokenPipe/ConnectionAborted
    pipe.set_writer(writer).await;

    let result = pipe
        .try_send_command_no_buffer(&Command::ProbeUiThread { nonce: 11 })
        .await;
    assert!(result.is_err(), "probe send must fail when peer dropped");
    assert!(!pipe.is_connected().await);
    assert_eq!(
        pipe.pending_len().await,
        0,
        "the rebuffered probe frame must be purged — stale probes never reach a future host"
    );
}
