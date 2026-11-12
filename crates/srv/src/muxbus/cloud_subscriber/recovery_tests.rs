// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The cloud subscriber through the faults of 2026-11-11, when two hosts stopped
//! receiving cloud messages for hours while their status said connected: a token
//! request that never answers, a relay that stops answering on an open socket,
//! and a loop that ends or stalls.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use super::supervisor::supervise;
use super::*;

/// A WebSocket relay: accepts one client, reads whatever it says, and sends a pong
/// every `pong_every` (never, for `None`).
async fn relay(pong_every: Option<Duration>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut tick = tokio::time::interval(pong_every.unwrap_or(Duration::from_secs(3600)));
        loop {
            tokio::select! {
                msg = ws.next() => {
                    if msg.is_none() {
                        break;
                    }
                }
                _ = tick.tick(), if pong_every.is_some() => {
                    let _ = ws.send(Message::Text(r#"{"type":"pong"}"#.into())).await;
                }
            }
        }
    });
    (url, task)
}

const SHORT: SessionLimits = SessionLimits {
    connect: Duration::from_secs(5),
    silence: Duration::from_millis(400),
    check_every: Duration::from_millis(100),
};

async fn session(ws_url: &str, limits: SessionLimits) -> Result<(), String> {
    std::env::set_var("AGENTMUX_MUXBUS_REST_URL", "http://127.0.0.1:9");
    std::env::set_var("AGENTMUX_MUXBUS_WS_URL", ws_url);
    let agents = Arc::new(Mutex::new(HashSet::new()));
    let (_ctrl_tx, mut ctrl_rx) = tokio::sync::mpsc::unbounded_channel::<CtrlMsg>();
    let mstore = Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    connect_and_run("test-token", agents, &mut ctrl_rx, &mstore, &reqwest::Client::new(), limits).await
}

#[tokio::test]
async fn a_token_request_that_never_answers_ends_in_its_time() {
    // A server that accepts the connection and never says a word.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hold = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    let creds = crate::muxbus::MuxBusCredentials {
        cognito_domain: format!("http://{addr}"),
        client_id: "client".into(),
        access_token: String::new(),
        refresh_token: "refresh".into(),
        id_token: String::new(),
        expires_at: 1,
        user_email: "a@example.test".into(),
        user_sub: String::new(),
    };
    let started = Instant::now();
    let result = crate::muxbus::pkce::refresh_token_within(&creds, &reqwest::Client::new(), Duration::from_millis(300)).await;
    assert!(
        matches!(result.err(), Some(crate::muxbus::pkce::RefreshTokenError::Network(_))),
        "a refresh with no answer is a network failure, which the broker retries"
    );
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    hold.abort();
}

#[tokio::test]
async fn a_relay_that_stops_answering_is_given_up_on_and_the_session_ends() {
    let _serial = crate::muxbus::pkce::TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (url, server) = relay(None).await;
    let ended = tokio::time::timeout(Duration::from_secs(5), session(&url, SHORT)).await.expect("the watchdog ends it");
    let why = ended.unwrap_err();
    assert!(why.contains("no word from the relay"), "{why}");
    server.abort();
    std::env::remove_var("AGENTMUX_MUXBUS_REST_URL");
    std::env::remove_var("AGENTMUX_MUXBUS_WS_URL");
}

#[tokio::test]
async fn a_relay_that_keeps_answering_is_kept() {
    let _serial = crate::muxbus::pkce::TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (url, server) = relay(Some(Duration::from_millis(100))).await;
    let still_open = tokio::time::timeout(Duration::from_millis(1500), session(&url, SHORT)).await;
    assert!(still_open.is_err(), "pongs inside the silence limit keep the session open: {still_open:?}");
    server.abort();
    std::env::remove_var("AGENTMUX_MUXBUS_REST_URL");
    std::env::remove_var("AGENTMUX_MUXBUS_WS_URL");
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_loop_that_panics_is_replaced() {
    let starts = Arc::new(AtomicUsize::new(0));
    let beat = Arc::new(AtomicI64::new(0));
    let supervisor = tokio::spawn({
        let (starts, beat) = (starts.clone(), beat.clone());
        async move {
            supervise(
                move || {
                    let n = starts.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async move {
                        if n == 0 {
                            panic!("the loop panicked (a test)");
                        }
                        std::future::pending::<()>().await
                    })
                },
                &beat,
                Duration::from_millis(50),
                Duration::from_secs(60),
                Duration::from_millis(10),
            )
            .await
        }
    });
    until("a second loop", || starts.load(Ordering::SeqCst) >= 2).await;
    supervisor.abort();
}

#[tokio::test]
async fn a_loop_that_ends_is_replaced() {
    let starts = Arc::new(AtomicUsize::new(0));
    let beat = Arc::new(AtomicI64::new(0));
    let supervisor = tokio::spawn({
        let (starts, beat) = (starts.clone(), beat.clone());
        async move {
            supervise(
                move || {
                    starts.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async {})
                },
                &beat,
                Duration::from_millis(50),
                Duration::from_secs(60),
                Duration::from_millis(10),
            )
            .await
        }
    });
    until("a third loop", || starts.load(Ordering::SeqCst) >= 3).await;
    supervisor.abort();
}

/// Sets a flag when dropped, which is when an aborted task's future goes.
struct OnDrop(Arc<AtomicBool>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_loop_that_stops_stamping_is_stopped_and_replaced() {
    let starts = Arc::new(AtomicUsize::new(0));
    let first_stopped = Arc::new(AtomicBool::new(false));
    let beat = Arc::new(AtomicI64::new(0));
    let supervisor = tokio::spawn({
        let (starts, beat, first_stopped) = (starts.clone(), beat.clone(), first_stopped.clone());
        async move {
            supervise(
                move || {
                    let n = starts.fetch_add(1, Ordering::SeqCst);
                    let flag = first_stopped.clone();
                    // Stuck for good, and never stamps: the stall the supervisor is for.
                    tokio::spawn(async move {
                        let _stopped = (n == 0).then(|| OnDrop(flag));
                        std::future::pending::<()>().await
                    })
                },
                &beat,
                Duration::from_millis(50),
                Duration::from_millis(300),
                Duration::from_millis(10),
            )
            .await
        }
    });
    until("a second loop", || starts.load(Ordering::SeqCst) >= 2).await;
    until("the stuck loop to be stopped", || first_stopped.load(Ordering::SeqCst)).await;
    supervisor.abort();
}

#[tokio::test]
async fn a_loop_that_keeps_stamping_is_left_alone() {
    let starts = Arc::new(AtomicUsize::new(0));
    let beat = Arc::new(AtomicI64::new(0));
    let supervisor = tokio::spawn({
        let (starts, beat) = (starts.clone(), beat.clone());
        let stamped = beat.clone();
        async move {
            supervise(
                move || {
                    starts.fetch_add(1, Ordering::SeqCst);
                    let beat = stamped.clone();
                    tokio::spawn(async move {
                        loop {
                            beat.store(agentmux_common::time::now_ms(), Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    })
                },
                &beat,
                Duration::from_millis(50),
                Duration::from_millis(300),
                Duration::from_millis(10),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1, "a loop that is making progress is not restarted");
    supervisor.abort();
}
