// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Screen mode: a Browser pane whose browser is an ordinary, visible Chrome on a virtual X screen, with nothing attached to it. The pane
//! gets the same frames and sends the same messages as the streamed browser (`pane.rs`, `protocol.rs`), but a frame is the whole screen
//! (the browser's own tab strip and address bar included) and input goes to the screen as a person's mouse and keyboard would, so the browser
//! does not report itself as remote-controlled (Chromium turns that report on for a debugging pipe, an automation or headless switch).
//! A prototype for the decision in task 48354: no agent driving, no windows list, a US keyboard, a fixed screen size.

mod frames;
mod held;
mod keymap;
mod policy;
mod session;
mod watch;
mod wm;
mod x;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use serde_json::json;
use tokio::sync::mpsc;

use super::pane::Start;
use super::protocol::{self, Button, ClientMsg, Go, KeyKind, MouseKind};
use crate::server::local_session_routes::WS_PROTOCOL;

/// `1` in the environment turns screen mode on (with the streamed browser on, and the pane asking for `mode=screen`).
pub const ENABLE_ENV: &str = "AGENTMUX_STREAMED_BROWSER_SCREEN";
/// The setting that allows more than one screen session at once, up to `MAX_SESSIONS` (a test host).
pub const MAX_ENV: &str = "AGENTMUX_STREAMED_BROWSER_SCREEN_MAX";
/// At most this many screen panes at once: each is a browser and a screen of its own.
const MAX_SESSIONS: usize = 4;
/// Frames per second at most; a screen that did not change makes none.
const FPS: u64 = 15;
/// How long to wait after Ctrl+L before typing an address.
const ADDRESS_BAR_FOCUS_MS: u64 = 250;
/// The longest paste that is typed key by key.
const MAX_TYPED: usize = 4000;

static OPEN: AtomicUsize = AtomicUsize::new(0);

/// A place among the open sessions, given back when it is dropped (however the session ended).
struct Place;

impl Place {
    fn take() -> Option<Self> {
        if OPEN.fetch_add(1, Ordering::SeqCst) >= limit() {
            OPEN.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Place)
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Screen mode holds its browser's policy in the folder of the container (or dedicated machine) it runs in, so a server with screen mode on serves
/// no streamed browser (the two would share one policy file) and, by default, one screen session: a hosted task holds one browser. A test host
/// may allow more with `MAX_ENV`.
pub fn limit() -> usize {
    std::env::var(MAX_ENV).ok().and_then(|v| v.parse::<usize>().ok()).map_or(1, |n| n.clamp(1, MAX_SESSIONS))
}

pub fn enabled() -> bool {
    std::env::var(ENABLE_ENV).is_ok_and(|v| v == "1")
}

/// Upgrade to the pane's WebSocket and run a screen session until it ends (or `ended` does).
pub fn upgrade(ws: WebSocketUpgrade, start: Start, ended: impl std::future::Future<Output = ()> + Send + 'static) -> Response {
    ws.protocols([WS_PROTOCOL]).on_upgrade(move |mut socket| async move {
        let Some(_place) = Place::take() else {
            let _ = socket.send(error(&format!("the server's browser is busy ({} panes are open)", limit()))).await;
            return;
        };
        run(socket, start, ended).await;
    })
}

fn error(message: &str) -> Message {
    Message::Text(json!({ "t": "error", "message": message }).to_string().into())
}

/// The screen's frames, made on a thread of their own (reading and encoding a screen is work that must not hold up input).
fn capture(display: &str, tx: mpsc::Sender<Vec<u8>>) -> Result<(), String> {
    let screen = x::Screen::connect(display)?;
    let (w, h) = (u32::from(screen.w), u32::from(screen.h));
    std::thread::spawn(move || {
        let mut changes = frames::Changes::default();
        let every = Duration::from_millis(1000 / FPS);
        while !tx.is_closed() {
            let started = Instant::now();
            let Ok(pixels) = screen.grab() else { return };
            if changes.differs(&pixels) {
                match frames::frame_message(&pixels, w, h) {
                    // A frame the pane is too slow to take is dropped, and the screen is not remembered as sent: the next tick tries again
                    // with a newer screen, so the pane never stays on a picture that is out of date.
                    Ok(frame) => match tx.try_send(frame) {
                        Ok(()) => changes.remember(&pixels),
                        Err(mpsc::error::TrySendError::Closed(_)) => return,
                        Err(mpsc::error::TrySendError::Full(_)) => {}
                    },
                    Err(_) => return,
                }
            }
            std::thread::sleep(every.saturating_sub(started.elapsed()));
        }
    });
    Ok(())
}

/// Run one pane until its socket closes, its screen goes, or the sign-in behind it ends.
async fn run(mut socket: WebSocket, start: Start, ended: impl std::future::Future<Output = ()>) {
    let session = match session::start(start.w, start.h, &start.url, OPEN.load(Ordering::SeqCst) <= 1).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "screen browser: a pane could not be opened");
            let _ = socket.send(error(&e)).await;
            return;
        }
    };
    // The browser takes a moment to put its window up: the screen is reachable at once.
    let input = match x::Screen::connect(&session.display) {
        Ok(s) => s,
        Err(e) => {
            let _ = socket.send(error(&e)).await;
            return;
        }
    };
    let (tx, mut frames) = mpsc::channel::<Vec<u8>>(2);
    if let Err(e) = capture(&session.display, tx) {
        let _ = socket.send(error(&e)).await;
        return;
    }
    let (wtx, mut counts) = mpsc::unbounded_channel::<usize>();
    if let Err(e) = watch_windows(&session.display, wtx) {
        let _ = socket.send(error(&e)).await;
        return;
    }
    let mut watch = watch::Watch::default();
    let _ = socket.send(Message::Text(json!({ "t": "size", "w": start.w, "h": start.h }).to_string().into())).await;
    let mut held = held::Held::default();
    tokio::pin!(ended);
    loop {
        let keep = tokio::select! {
            _ = &mut ended => false,
            frame = frames.recv() => match frame {
                Some(f) => socket.send(Message::Binary(f.into())).await.is_ok(),
                None => false,
            },
            count = counts.recv() => match count {
                Some(n) => match watch.seen(n, Instant::now()) {
                    Some(notice) => socket.send(Message::Text(json!({ "t": "notice", "message": notice }).to_string().into())).await.is_ok(),
                    None => true,
                },
                None => false,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    if let Some(said) = on_client(&input, t.as_str(), &start, watch.keys_held(Instant::now()), &mut held).await {
                        socket.send(said).await.is_ok()
                    } else {
                        true
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => false,
                Some(Ok(_)) => true,
            },
        };
        if !keep {
            break;
        }
    }
    drop(session);
}

/// How many windows the screen shows, told whenever it changes (a look every quarter second, on a thread of its own).
fn watch_windows(display: &str, tx: mpsc::UnboundedSender<usize>) -> Result<(), String> {
    let screen = x::Screen::connect(display)?;
    std::thread::spawn(move || {
        let mut last = usize::MAX;
        while !tx.is_closed() {
            match screen.visible_windows() {
                Ok(n) if n != last => {
                    last = n;
                    if tx.send(n).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    Ok(())
}

/// One message from the pane, sent to the screen. A word for the pane when there is one to say. While `keys_held`, a key press or a
/// paste is dropped (a window just opened that the person did not choose); a release still goes, so no key is left down.
async fn on_client(screen: &x::Screen, text: &str, start: &Start, keys_held: bool, held: &mut held::Held) -> Option<Message> {
    let msg = match protocol::parse(text) {
        Ok(m) => m,
        Err(e) => return Some(error(&e)),
    };
    if keys_held && matches!(msg, ClientMsg::Key { kind: KeyKind::Down, .. } | ClientMsg::Paste { .. }) {
        return None;
    }
    if let ClientMsg::Key { kind, code, .. } = &msg {
        held.note(code, matches!(kind, KeyKind::Down));
    }
    let sent = match msg {
        // The paste is typed with the modifiers let go (Ctrl+V is the paste itself), and they are pressed again after.
        ClientMsg::Paste { .. } => send_without_modifiers(screen, &msg, held.keys()).await,
        _ => send_input(screen, &msg).await,
    };
    match sent {
        Ok(()) => match msg {
            // The screen keeps its size: say so, as a resize that was answered.
            ClientMsg::Resize { .. } => Some(Message::Text(json!({ "t": "size", "w": start.w, "h": start.h }).to_string().into())),
            _ => None,
        },
        Err(e) => Some(error(&e)),
    }
}

/// Send `msg` with the modifier keys in `held` let go for the while, then pressed again, so what the person still holds stays held.
async fn send_without_modifiers(screen: &x::Screen, msg: &ClientMsg, held: &[u8]) -> Result<(), String> {
    for k in held {
        screen.key(*k, false)?;
    }
    let sent = send_input(screen, msg).await;
    for k in held {
        screen.key(*k, true)?;
    }
    sent
}

fn button_number(b: Button) -> Option<u8> {
    match b {
        Button::Left => Some(1),
        Button::Middle => Some(2),
        Button::Right => Some(3),
        Button::None => None,
    }
}

/// How many wheel notches a scroll of `delta` pixels is (the pane sends 100 to 120 a notch).
fn notches(delta: f64) -> u32 {
    if delta.abs() < 1.0 {
        0
    } else {
        ((delta.abs() / 100.0).round() as u32).clamp(1, 10)
    }
}

async fn send_input(screen: &x::Screen, msg: &ClientMsg) -> Result<(), String> {
    match msg {
        ClientMsg::Mouse { kind, x, y, button, .. } => {
            screen.move_to(*x, *y)?;
            if let (Some(n), MouseKind::Down | MouseKind::Up) = (button_number(*button), kind) {
                screen.button(n, matches!(kind, MouseKind::Down))?;
            }
        }
        ClientMsg::Wheel { x, y, dx, dy, .. } => {
            screen.move_to(*x, *y)?;
            for (delta, negative, positive) in [(*dy, 4, 5), (*dx, 6, 7)] {
                for _ in 0..notches(delta) {
                    let b = if delta < 0.0 { negative } else { positive };
                    screen.button(b, true)?;
                    screen.button(b, false)?;
                }
            }
        }
        ClientMsg::Key { kind, code, text, .. } => {
            let down = matches!(kind, KeyKind::Down);
            if let Some(k) = keymap::keycode(code) {
                screen.key(k, down)?;
            } else if down && text.chars().count() == 1 {
                // A key whose place is not known (another layout) that types one character: typed as the US key that makes it.
                type_text(screen, text).await?;
            }
        }
        ClientMsg::Paste { text } => type_text(screen, text).await?,
        ClientMsg::Nav { go: Go::Url, url } => {
            // The checked, normalised address is what is typed: the raw string could carry a tab or a newline that moves the typing elsewhere.
            let url = protocol::check_url(url)?;
            navigate(screen, Go::Url, &url).await?
        }
        ClientMsg::Nav { go, url } => navigate(screen, *go, url).await?,
        ClientMsg::Resize { .. } | ClientMsg::Window { .. } => {}
    }
    Ok(())
}

/// Press and release a key, with Shift when it needs it.
fn tap(screen: &x::Screen, key: u8, shift: bool) -> Result<(), String> {
    if shift {
        screen.key(keymap::SHIFT, true)?;
    }
    screen.key(key, true)?;
    screen.key(key, false)?;
    if shift {
        screen.key(keymap::SHIFT, false)?;
    }
    Ok(())
}

/// Type text key by key. A character with no US key is skipped; the text is cut at `MAX_TYPED` characters.
async fn type_text(screen: &x::Screen, text: &str) -> Result<(), String> {
    for (i, c) in text.chars().take(MAX_TYPED).enumerate() {
        if let Some((key, shift)) = keymap::ascii(c) {
            tap(screen, key, shift)?;
        }
        if i % 64 == 63 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    Ok(())
}

/// Back, forward, reload and stop are the browser's own keys; an address is typed into its address bar.
async fn navigate(screen: &x::Screen, go: Go, url: &str) -> Result<(), String> {
    let chord = |modifier: u8, code: &str| -> Result<(), String> {
        let key = keymap::keycode(code).ok_or("no such key")?;
        screen.key(modifier, true)?;
        tap(screen, key, false)?;
        screen.key(modifier, false)
    };
    match go {
        Go::Back => chord(keymap::ALT, "ArrowLeft"),
        Go::Forward => chord(keymap::ALT, "ArrowRight"),
        Go::Reload => tap(screen, keymap::keycode("F5").ok_or("no such key")?, false),
        Go::Stop => tap(screen, keymap::keycode("Escape").ok_or("no such key")?, false),
        Go::Url => {
            // `parse` has already checked the address: only http, https or about:blank.
            chord(keymap::CONTROL, "KeyL")?;
            // The address bar takes the focus a moment after the chord: typing at once loses the first letters (found on a real run).
            tokio::time::sleep(Duration::from_millis(ADDRESS_BAR_FOCUS_MS)).await;
            type_text(screen, url).await?;
            tap(screen, keymap::keycode("Enter").ok_or("no such key")?, false)
        }
    }
}

#[cfg(test)]
mod tests;
