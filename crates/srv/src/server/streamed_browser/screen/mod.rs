// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Screen mode: a Browser pane whose browser is an ordinary, visible Chrome on a virtual X screen, with nothing attached to it. The pane
//! gets the same frames and sends the same messages as the streamed browser (`pane.rs`, `protocol.rs`), but a frame is the whole screen
//! (the browser's own tab strip and address bar included) and input goes to the screen as a person's mouse and keyboard would, so the browser
//! does not report itself as remote-controlled (Chromium turns that report on for a debugging pipe, an automation or headless switch).
//! A prototype for the decision in task 48354: no agent driving, no windows list, a US keyboard. The screen follows the pane's size (`randr`).

mod clipboard;
mod ewmh;
mod frames;
mod held;
mod keymap;
mod pace;
mod policy;
mod randr;
mod session;
mod watch;
mod wm;
mod x;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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

fn size_message(w: u16, h: u16) -> Message {
    Message::Text(json!({ "t": "size", "w": w, "h": h }).to_string().into())
}

fn error(message: &str) -> Message {
    Message::Text(json!({ "t": "error", "message": message }).to_string().into())
}

/// The screen's frames, made on a thread of their own (reading and encoding a screen is work that must not hold up input). The screen says
/// when a part of it changed: nothing is read while it is still, so a page that is not moving costs next to nothing. A change is read where
/// it happened and written into the picture of the whole screen, and a frame is made only if some pixel really differs. Frames are at most
/// `FPS` a second, but come at once and quickly just after the person's input (`pace`).
fn capture(display: &str, tx: mpsc::Sender<Vec<u8>>, activity: pace::Activity) -> Result<(), String> {
    let screen = x::Screen::connect(display)?;
    // A screen that changes size is read at its new size, whole.
    screen.follow_size()?;
    let damage = screen.watch_damage()?;
    let (mut w, mut h) = screen.size();
    std::thread::spawn(move || {
        let steady = Duration::from_millis(1000 / FPS);
        let mut picture = frames::Picture::new(w, h);
        // The first picture is the whole screen, and is sent whatever it holds.
        let mut pending = Some(frames::Rect::whole(w, h));
        let mut unsent = true;
        let mut last_frame: Option<Instant> = None;
        while !tx.is_closed() {
            if pending.is_none() && !unsent {
                // Waits until the screen changes; ends with the screen (the thread goes when the browser's screen does).
                match screen.wait_damage() {
                    Ok(Some(at)) => pending = Some(at),
                    // The screen changed size: handled below, with whatever else changed.
                    Ok(None) => {}
                    Err(_) => return,
                }
            }
            // The gap runs from the start of the last frame, so making a frame (which takes time) is part of the gap and not on top of it.
            if let Some(at) = last_frame {
                std::thread::sleep(pace::gap(activity.since(), steady).saturating_sub(at.elapsed()));
            }
            let began = Instant::now();
            match screen.take_damage() {
                Ok(Some(more)) => pending = Some(pending.map_or(more, |p| p.union(more))),
                Ok(None) => {}
                Err(_) => return,
            }
            if screen.take_resized() {
                (w, h) = screen.size();
                picture = frames::Picture::new(w, h);
                pending = Some(frames::Rect::whole(w, h));
                unsent = true;
            }
            if let Some(at) = pending.take() {
                // Taken in before it is read, so a change made while it is read is not lost.
                if screen.clear_damage(damage).is_err() {
                    return;
                }
                match screen.grab_rect(at) {
                    Ok(part) => unsent |= picture.update(at, &part),
                    // A read that fails because the screen changed size just then is made again, whole, at the new size; any other is the end.
                    Err(_) if screen.refresh_size() => unsent = true,
                    Err(_) => return,
                }
            }
            if unsent {
                let Ok(frame) = frames::frame_message(picture.pixels(), u32::from(w), u32::from(h)) else { return };
                match tx.try_send(frame) {
                    Ok(()) => unsent = false,
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                    // A frame the pane is too slow to take is dropped: the picture is still unsent, so it is tried again after the gap,
                    // from the newest screen, and the pane never stays on a picture that is out of date.
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                }
                last_frame = Some(began);
            }
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
    let activity = pace::Activity::default();
    if let Err(e) = capture(&session.display, tx, activity.clone()) {
        let _ = socket.send(error(&e)).await;
        return;
    }
    // The screen's clipboard, for an exact paste and a copy on the person's request; without it a paste is typed key by key.
    let clip = clipboard::Clipboard::connect(&session.display).map(Arc::new).map_err(|e| tracing::warn!(error = %e, "screen browser: no clipboard")).ok();
    let (wtx, mut counts) = mpsc::unbounded_channel::<usize>();
    if let Err(e) = watch_windows(&session.display, wtx, input.opened.clone()) {
        let _ = socket.send(error(&e)).await;
        return;
    }
    // The browser's window fills the screen once it is up, and (maximised) follows the screen when the pane is resized (`ewmh`).
    let display = session.display.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = x::Screen::connect(&display).and_then(|s| s.maximize_first_within(Duration::from_secs(15))) {
            tracing::warn!(error = %e, "screen browser: the window was not maximised");
        }
    });
    let mut watch = watch::Watch::default();
    let (sw, sh) = input.size();
    let _ = socket.send(size_message(sw, sh)).await;
    let mut held = held::Held::default();
    let mut settle = randr::Settle::default();
    tokio::pin!(ended);
    loop {
        let settled = settle.due_at().map(tokio::time::Instant::from_std);
        let keep = tokio::select! {
            // A window opening is handled before the next thing the person typed, so the key hold starts first; what the person sends is
            // read before the next frame is sent, so a slow link that keeps the frame channel full never starves the person's input.
            biased;
            _ = &mut ended => false,
            count = counts.recv() => match count {
                Some(n) => match watch.seen(n, Instant::now()) {
                    Some(notice) => socket.send(Message::Text(json!({ "t": "notice", "message": notice }).to_string().into())).await.is_ok(),
                    None => true,
                },
                None => false,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    if let Some(said) = on_client(&input, t.as_str(), &mut settle, watch.keys_held(Instant::now()), &mut held, &activity, clip.as_ref()).await {
                        socket.send(said).await.is_ok()
                    } else {
                        true
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => false,
                Some(Ok(_)) => true,
            },
            // The pane's size has stayed the same for a moment: the screen is made that size, and the pane is told.
            () = async { tokio::time::sleep_until(settled.unwrap_or_else(tokio::time::Instant::now)).await }, if settled.is_some() => match settle.take(Instant::now()) {
                Some((w, h)) => match input.resize(w, h) {
                    Ok(()) => socket.send(size_message(w, h)).await.is_ok(),
                    Err(e) => socket.send(error(&e)).await.is_ok(),
                },
                None => true,
            },
            frame = frames.recv() => match frame {
                Some(f) => socket.send(Message::Binary(f.into())).await.is_ok(),
                None => false,
            },
        };
        if !keep {
            break;
        }
    }
    drop(session);
}

/// How many windows the screen shows, told whenever it changes: at the moment a window is mapped or unmapped (the screen's own events, on a
/// thread of its own), so what is typed after a window opens is held from that moment.
fn watch_windows(display: &str, tx: mpsc::UnboundedSender<usize>, opened: Arc<AtomicUsize>) -> Result<(), String> {
    let screen = x::Screen::connect(display)?;
    screen.follow_windows()?;
    std::thread::spawn(move || {
        let mut last = usize::MAX;
        loop {
            match screen.visible_windows() {
                Ok(n) if n != last => {
                    // Before the person's next key is read, anything being typed is told a window opened.
                    if last != usize::MAX && watch::opened_one(last, n) {
                        opened.fetch_add(1, Ordering::SeqCst);
                    }
                    last = n;
                    if tx.send(n).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
            // Blocks until the screen says a window changed; ends with the screen (the thread goes when the browser's screen does).
            if screen.next_window_event().is_err() {
                return;
            }
        }
    });
    Ok(())
}

/// One message from the pane, sent to the screen. A word for the pane when there is one to say. While `keys_held`, a key press or a
/// paste is dropped (a window just opened that the person did not choose); a release still goes, so no key is left down.
async fn on_client(screen: &x::Screen, text: &str, settle: &mut randr::Settle, keys_held: bool, held: &mut held::Held, activity: &pace::Activity, clip: Option<&Arc<clipboard::Clipboard>>) -> Option<Message> {
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
    // What the person does is seen as it happens: frames come quickly for a moment (a pointer that only moves does not count).
    if !matches!(msg, ClientMsg::Mouse { kind: MouseKind::Move, .. } | ClientMsg::Resize { .. }) {
        activity.touch();
    }
    if matches!(msg, ClientMsg::Copy) {
        return copy_out(clip).await;
    }
    // The pane's size is followed once it has stayed the same for a moment (`randr::Settle`), and the pane is told then.
    if let ClientMsg::Resize { w, h } = msg {
        settle.ask(randr::clamp(w, h), Instant::now());
        return None;
    }
    let sent = match msg {
        // The text goes to the screen's clipboard and Ctrl+V is pressed, with the other modifiers let go; without a clipboard it is typed key by key.
        ClientMsg::Paste { ref text } if clip.is_some() => paste_exact(screen, clip, text, held.keys()).await,
        // The paste is typed with the modifiers let go (Ctrl+V is the paste itself), and they are pressed again after.
        ClientMsg::Paste { .. } => send_without_modifiers(screen, &msg, held.keys()).await,
        _ => send_input(screen, &msg).await,
    };
    sent.err().map(|e| error(&e))
}

/// Paste `text` exactly: offer it as the screen's clipboard and press Ctrl+V (the other modifiers the person holds are let go for the while, and
/// pressed again). The offer ends a moment later, so nothing stays on the screen's clipboard for the browser to read on its own.
async fn paste_exact(screen: &x::Screen, clip: Option<&Arc<clipboard::Clipboard>>, text: &str, held: &[u8]) -> Result<(), String> {
    let clip = clip.ok_or("no clipboard")?;
    let generation = clip.offer(text)?;
    let v = keymap::keycode("KeyV").ok_or("no such key")?;
    for k in held {
        screen.key(*k, false)?;
    }
    screen.key(keymap::CONTROL, true)?;
    let tapped = tap(screen, v, false);
    screen.key(keymap::CONTROL, false)?;
    for k in held {
        screen.key(*k, true)?;
    }
    let clip = clip.clone();
    tokio::spawn(async move {
        tokio::time::sleep(clipboard::OFFER_HOLD).await;
        clip.end(generation);
    });
    tapped
}

/// What the browser copied, for the pane (which puts it on the person's clipboard): asked for by the person (Ctrl+C or Ctrl+X in the pane).
/// None when the browser has copied nothing.
async fn copy_out(clip: Option<&Arc<clipboard::Clipboard>>) -> Option<Message> {
    let clip = clip?.clone();
    match tokio::task::spawn_blocking(move || clip.fetch()).await {
        Ok(Ok(Some(text))) => Some(Message::Text(json!({ "t": "clipboard", "text": text }).to_string().into())),
        Ok(Ok(None)) => None,
        Ok(Err(e)) => Some(error(&e)),
        Err(_) => Some(error("the remote browser's clipboard could not be read")),
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
        ClientMsg::Paste { text } => {
            type_text(screen, text).await?;
        }
        ClientMsg::Nav { go: Go::Url, url } => {
            // The checked, normalised address is what is typed: the raw string could carry a tab or a newline that moves the typing elsewhere.
            let url = protocol::check_url(url)?;
            navigate(screen, Go::Url, &url).await?
        }
        ClientMsg::Nav { go, url } => navigate(screen, *go, url).await?,
        ClientMsg::Resize { .. } | ClientMsg::Window { .. } | ClientMsg::Copy => {}
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

/// Type text key by key. A character with no US key is skipped; the text is cut at `MAX_TYPED` characters. When a window opens while it is
/// being typed, the rest is dropped: it was meant for the window the person was looking at.
async fn type_text(screen: &x::Screen, text: &str) -> Result<Typed, String> {
    let windows = screen.opened.load(Ordering::SeqCst);
    type_chars(text, || screen.opened.load(Ordering::SeqCst) != windows, |key, shift| tap(screen, key, shift)).await
}

/// How typing ended: all of the text was typed, or it stopped because the person's screen changed under it.
#[derive(Debug, PartialEq, Eq)]
enum Typed {
    Done,
    Interrupted,
}

/// The typing itself: `stop` is asked before every character, `press` types one key (with Shift or not).
async fn type_chars(text: &str, stop: impl Fn() -> bool, mut press: impl FnMut(u8, bool) -> Result<(), String>) -> Result<Typed, String> {
    for (i, c) in text.chars().take(MAX_TYPED).enumerate() {
        if stop() {
            return Ok(Typed::Interrupted);
        }
        if let Some((key, shift)) = keymap::ascii(c) {
            press(key, shift)?;
        }
        if i % 64 == 63 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    Ok(Typed::Done)
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
            // A window that opened while the address was typed took what was left of it: Enter would send half an address (or go to the new window).
            if type_text(screen, url).await? == Typed::Interrupted {
                return Ok(());
            }
            tap(screen, keymap::keycode("Enter").ok_or("no such key")?, false)
        }
    }
}

#[cfg(test)]
mod tests;
