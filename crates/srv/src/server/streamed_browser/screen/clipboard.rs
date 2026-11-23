// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The clipboard of the virtual screen (the X selection `CLIPBOARD`), shared with the person's own on their say-so only.
//! A paste from the person is offered here as the screen's clipboard and Ctrl+V is pressed, so the browser reads the exact text (any
//! letter, any length up to the limit) instead of having it typed key by key; the offer ends a moment later. A copy is read out only when
//! the person asks for it (they pressed Ctrl+C or Ctrl+X in the pane): the text the browser put on the screen's clipboard goes to the pane,
//! which puts it on the person's. Nothing reads the screen's clipboard on its own, and no agent is involved.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent, SelectionRequestEvent, WindowClass, SELECTION_NOTIFY_EVENT};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_FROM_PARENT, CURRENT_TIME, NONE};

/// The most text that is offered or read (the same limit as a paste message).
pub const MAX_TEXT: usize = 1024 * 1024;
/// How long the browser has to read a pasted text before the offer ends.
pub const OFFER_HOLD: Duration = Duration::from_millis(1500);
/// How long a read of the browser's copy waits for the browser to answer.
const FETCH_WAIT: Duration = Duration::from_secs(1);

/// The atoms the clipboard talks in.
pub struct Atoms {
    pub clipboard: Atom,
    pub utf8: Atom,
    pub targets: Atom,
    pub plain_utf8: Atom,
    pub incr: Atom,
    pub property: Atom,
}

/// What to answer a request for `target`.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// The list of kinds of text that can be given.
    Targets(Vec<Atom>),
    /// The text itself.
    Text,
    /// Nothing: no text is offered, or a kind that is not text.
    Refuse,
}

/// The answer to a request for `target` while `offering` says whether a text is on offer.
pub fn answer(a: &Atoms, target: Atom, offering: bool) -> Answer {
    if !offering {
        Answer::Refuse
    } else if target == a.targets {
        Answer::Targets(vec![a.targets, a.utf8, a.plain_utf8])
    } else if target == a.utf8 || target == a.plain_utf8 {
        Answer::Text
    } else {
        Answer::Refuse
    }
}

/// The text on offer, and which offer it is (so an old offer's end does not end a newer one).
#[derive(Default)]
struct Offered {
    text: Option<String>,
    generation: u64,
}

impl Offered {
    fn offer(&mut self, text: String) -> u64 {
        self.generation += 1;
        self.text = Some(text);
        self.generation
    }

    /// End the offer `generation`, unless a newer one has replaced it or it already ended. True when it ended now.
    fn end(&mut self, generation: u64) -> bool {
        if self.generation != generation || self.text.is_none() {
            return false;
        }
        self.text = None;
        true
    }
}

pub struct Clipboard {
    conn: Arc<RustConnection>,
    window: u32,
    atoms: Arc<Atoms>,
    offered: Arc<Mutex<Offered>>,
    /// Where the answer to a read of the browser's copy goes (true when the browser gave text).
    reading: Arc<Mutex<Option<mpsc::Sender<bool>>>>,
}

impl Clipboard {
    /// A clipboard on display `display`: a window of its own that owns and reads the selection, and a thread that answers what the browser asks.
    pub fn connect(display: &str) -> Result<Self, String> {
        let (conn, n) = RustConnection::connect(Some(display)).map_err(|e| format!("cannot reach the virtual screen {display}: {e}"))?;
        let root = conn.setup().roots[n].root;
        let window = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(COPY_FROM_PARENT as u8, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, COPY_FROM_PARENT, &CreateWindowAux::new())
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("the clipboard's window could not be made: {e}"))?;
        let atom = |name: &[u8]| -> Result<Atom, String> {
            Ok(conn.intern_atom(false, name).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom)
        };
        let atoms = Arc::new(Atoms {
            clipboard: atom(b"CLIPBOARD")?,
            utf8: atom(b"UTF8_STRING")?,
            targets: atom(b"TARGETS")?,
            plain_utf8: atom(b"text/plain;charset=utf-8")?,
            incr: atom(b"INCR")?,
            property: atom(b"AGENTMUX_CLIPBOARD")?,
        });
        let this = Self { conn: Arc::new(conn), window, atoms, offered: Arc::default(), reading: Arc::default() };
        this.serve();
        Ok(this)
    }

    /// The thread that answers the browser's requests for the offered text and tells a read that the browser answered. It ends with the screen.
    fn serve(&self) {
        let (conn, atoms, offered, reading) = (self.conn.clone(), self.atoms.clone(), self.offered.clone(), self.reading.clone());
        std::thread::spawn(move || loop {
            let Ok(event) = conn.wait_for_event() else { return };
            match event {
                Event::SelectionRequest(r) => {
                    let _ = reply(&conn, &atoms, &offered, &r);
                }
                Event::SelectionNotify(n) => {
                    if let Some(tx) = reading.lock().ok().and_then(|mut r| r.take()) {
                        let _ = tx.send(n.property != NONE);
                    }
                }
                Event::SelectionClear(_) => {
                    if let Ok(mut o) = offered.lock() {
                        o.text = None;
                    }
                }
                _ => {}
            }
        });
    }

    /// Offer `text` as the screen's clipboard. The number says which offer it is, for `end`.
    pub fn offer(&self, text: &str) -> Result<u64, String> {
        if text.len() > MAX_TEXT || text.len() + 100 > self.conn.maximum_request_bytes() {
            return Err("that paste is too long for the remote browser's clipboard".to_string());
        }
        let generation = self.offered.lock().map_err(|_| "the clipboard is busy".to_string())?.offer(text.to_string());
        self.conn.set_selection_owner(self.window, self.atoms.clipboard, CURRENT_TIME).map_err(|e| e.to_string())?;
        self.conn.flush().map_err(|e| e.to_string())?;
        let owner = self.conn.get_selection_owner(self.atoms.clipboard).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.owner;
        if owner != self.window {
            return Err("the remote browser's clipboard could not be taken".to_string());
        }
        Ok(generation)
    }

    /// End the offer `generation` (nothing is left on the screen's clipboard for the browser to read later), unless a newer offer replaced it.
    pub fn end(&self, generation: u64) {
        if self.offered.lock().is_ok_and(|mut o| o.end(generation)) {
            let _ = self.conn.set_selection_owner(NONE, self.atoms.clipboard, CURRENT_TIME);
            let _ = self.conn.flush();
        }
    }

    /// The text the browser has put on the screen's clipboard, or none when it has put none (or not text). Waits up to a second.
    pub fn fetch(&self) -> Result<Option<String>, String> {
        let (tx, rx) = mpsc::channel();
        *self.reading.lock().map_err(|_| "the clipboard is busy".to_string())? = Some(tx);
        self.conn
            .convert_selection(self.window, self.atoms.clipboard, self.atoms.utf8, self.atoms.property, CURRENT_TIME)
            .map_err(|e| e.to_string())?;
        self.conn.flush().map_err(|e| e.to_string())?;
        match rx.recv_timeout(FETCH_WAIT) {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(_) => {
                if let Ok(mut r) = self.reading.lock() {
                    r.take();
                }
                return Err("the remote browser did not answer for its clipboard".to_string());
            }
        }
        let got = self
            .conn
            .get_property(true, self.window, self.atoms.property, AtomEnum::ANY, 0, (MAX_TEXT / 4 + 1) as u32)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        if got.type_ == self.atoms.incr {
            return Err("that copy is too long to bring over".to_string());
        }
        if got.bytes_after > 0 {
            return Err("that copy is too long to bring over".to_string());
        }
        Ok(String::from_utf8(got.value).ok().filter(|t| !t.is_empty()))
    }
}

/// Answer one request of the browser for the offered text: put the text (or the list of kinds) on the asking window, then say so.
fn reply(conn: &RustConnection, a: &Atoms, offered: &Mutex<Offered>, r: &SelectionRequestEvent) -> Result<(), String> {
    let text = offered.lock().ok().and_then(|o| o.text.clone());
    let property = if r.property == NONE { r.target } else { r.property };
    let given = if r.selection != a.clipboard {
        NONE
    } else {
        match answer(a, r.target, text.is_some()) {
            Answer::Targets(list) => {
                conn.change_property32(PropMode::REPLACE, r.requestor, property, AtomEnum::ATOM, &list).map_err(|e| e.to_string())?;
                property
            }
            Answer::Text => {
                let text = text.unwrap_or_default();
                conn.change_property8(PropMode::REPLACE, r.requestor, property, r.target, text.as_bytes()).map_err(|e| e.to_string())?;
                property
            }
            Answer::Refuse => NONE,
        }
    };
    let notify = SelectionNotifyEvent { response_type: SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property: given };
    conn.send_event(false, r.requestor, EventMask::NO_EVENT, notify).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms() -> Atoms {
        Atoms { clipboard: 10, utf8: 11, targets: 12, plain_utf8: 13, incr: 14, property: 15 }
    }

    #[test]
    fn text_is_given_in_the_kinds_the_browser_asks_for_and_nothing_else() {
        let a = atoms();
        assert_eq!(answer(&a, a.utf8, true), Answer::Text);
        assert_eq!(answer(&a, a.plain_utf8, true), Answer::Text);
        assert_eq!(answer(&a, a.targets, true), Answer::Targets(vec![12, 11, 13]));
        assert_eq!(answer(&a, 99, true), Answer::Refuse, "a kind that is not text");
    }

    #[test]
    fn nothing_is_given_when_nothing_is_on_offer() {
        let a = atoms();
        for target in [a.utf8, a.plain_utf8, a.targets, 99] {
            assert_eq!(answer(&a, target, false), Answer::Refuse);
        }
    }

    #[test]
    fn an_old_offers_end_does_not_end_a_newer_offer() {
        let mut o = Offered::default();
        let first = o.offer("one".to_string());
        let second = o.offer("two".to_string());
        assert!(!o.end(first), "the first offer was replaced");
        assert_eq!(o.text.as_deref(), Some("two"));
        assert!(o.end(second));
        assert_eq!(o.text, None);
        assert!(!o.end(second), "already ended");
    }
}
