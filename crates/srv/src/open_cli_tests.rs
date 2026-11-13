// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;
use std::time::Instant;

use super::*;
use crate::server::local_session::{CodeState, Entered, SessionStore};

/// `open` talking to a real session store.
#[derive(Default)]
struct Local {
    store: SessionStore,
    entered: Mutex<Vec<Entered>>,
}

impl Local {
    /// A browser enters the code; a spent code makes nothing.
    fn enter(&self, code: &str) {
        if let Some(e) = self.store.enter(code, Instant::now()) {
            self.entered.lock().unwrap().push(e);
        }
    }
}

impl Server for Local {
    async fn mint_code(&self) -> Result<String, String> {
        Ok(self.store.mint_code(Instant::now()))
    }
    async fn entry(&self, code: &str) -> Result<Entry, String> {
        Ok(match self.store.code_state(code, Instant::now()) {
            CodeState::Unused => Entry::Unused,
            CodeState::Entered { phrase } => Entry::Entered(phrase),
            CodeState::Gone => Entry::Gone,
        })
    }
    async fn answer(&self, code: &str, accept: bool) -> Result<bool, String> {
        Ok(self.store.decide(code, accept, Instant::now()))
    }
}

/// A terminal that types a fixed answer, and a browser that enters the code of the line it is given.
struct Script<'a> {
    server: Option<&'a Local>,
    /// Another local user read the line from the process list and enters the code first.
    thief_first: bool,
    tty: bool,
    answer: Option<&'static str>,
    said: Vec<String>,
    asked: Vec<String>,
    opened: Vec<String>,
}

impl<'a> Script<'a> {
    fn new(server: &'a Local, answer: Option<&'static str>) -> Script<'a> {
        Script { server: Some(server), thief_first: false, tty: true, answer, said: vec![], asked: vec![], opened: vec![] }
    }

    /// The person's browser is given this line (opened for them, or pasted from `--print`).
    fn browser_gets(&self, line: &str) {
        let (Some(server), Some((_, code))) = (self.server, line.split_once("/auth/enter#")) else { return };
        if self.thief_first {
            server.enter(code);
        }
        server.enter(code);
    }
}

impl Terminal for Script<'_> {
    fn is_terminal(&self) -> bool {
        self.tty
    }
    fn say(&mut self, line: &str) {
        if line.starts_with("http://") {
            self.browser_gets(line);
        }
        self.said.push(line.to_string());
    }
    async fn ask(&mut self, prompt: &str, _limit: Duration) -> Option<String> {
        self.asked.push(prompt.to_string());
        self.answer.map(str::to_string)
    }
    fn open_browser(&mut self, url: &str) -> bool {
        self.browser_gets(url);
        self.opened.push(url.to_string());
        true
    }
}

const ORIGIN: &str = "http://127.0.0.1:7380";

#[test]
fn the_url_keeps_the_code_in_the_fragment() {
    assert_eq!(sign_in_url(ORIGIN, "abc"), "http://127.0.0.1:7380/auth/enter#abc");
    assert_eq!(sign_in_url("http://127.0.0.1:8080/", "abc"), "http://127.0.0.1:8080/auth/enter#abc");
}

#[test]
fn only_y_or_yes_confirms_and_a_bare_enter_does_not() {
    for yes in ["y", "Y", "yes", "YES", " y \n", "y\r\n"] {
        assert!(is_yes(yes), "{yes:?}");
    }
    for no in ["", "\n", "n", "N", "no", "yy", "yes please", "ok", " "] {
        assert!(!is_yes(no), "{no:?}");
    }
}

#[tokio::test]
async fn confirming_signs_the_browser_in() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 0);
    assert!(t.opened[0].starts_with("http://127.0.0.1:7380/auth/enter#"));
    let phrase = server.entered.lock().unwrap()[0].phrase.clone();
    assert!(t.asked[0].contains(&format!("\"{phrase}\"")) && t.asked[0].ends_with("[y/N] "), "{:?}", t.asked);
    assert_eq!(server.store.session_count(Instant::now()), 1);
    assert!(t.said.last().unwrap().starts_with("Signed in"));
}

#[tokio::test]
async fn anything_but_a_yes_discards_the_session() {
    for answer in [Some("\n"), Some("n\n"), Some("maybe\n"), None] {
        let server = Local::default();
        let mut t = Script::new(&server, answer);
        assert_eq!(run(&server, ORIGIN, false, &mut t).await, 1, "{answer:?}");
        assert_eq!(server.store.session_count(Instant::now()), 0, "{answer:?}");
        assert!(t.said.last().unwrap().contains("--print"), "{:?}", t.said);
    }
}

/// Another browser entered the code first: `open` shows that session's phrase, which the person's
/// own browser (refused, the code is spent) never showed. Answering `n` leaves nothing live.
#[tokio::test]
async fn a_session_the_person_does_not_recognise_is_discarded_and_nothing_goes_live() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("n\n"));
    t.thief_first = true;
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 1);
    assert_eq!(server.entered.lock().unwrap().len(), 1, "the person's own entry made nothing");
    assert_eq!(server.store.session_count(Instant::now()), 0);
}

#[tokio::test]
async fn print_shows_the_line_and_opens_no_browser() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    assert_eq!(run(&server, ORIGIN, true, &mut t).await, 0);
    assert!(t.opened.is_empty());
    assert!(t.said.iter().any(|l| l.starts_with("http://127.0.0.1:7380/auth/enter#")), "{:?}", t.said);
}

#[tokio::test]
async fn no_terminal_means_no_sign_in() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    t.tty = false;
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 3);
    assert!(t.opened.is_empty() && t.asked.is_empty());
}

/// A code nobody enters: `open` gives up after the minute and says so.
#[tokio::test(start_paused = true)]
async fn a_code_nobody_enters_ends_the_wait_after_a_minute() {
    let server = Local::default();
    let mut t = Script::new(&server, Some("y\n"));
    t.server = None; // the browser never opens the line
    let before = tokio::time::Instant::now();
    assert_eq!(run(&server, ORIGIN, false, &mut t).await, 1);
    assert!(tokio::time::Instant::now() - before >= STEP_LIMIT);
    assert!(t.asked.is_empty());
    assert!(t.said.last().unwrap().contains("No browser entered"));
}

/// The typed origin is the one `--public-origin` accepts, or nothing: a line carrying a live code is
/// never opened at another page. It is judged before the key file is read.
#[test]
fn the_origin_is_held_to_the_rule_of_the_servers_public_origin() {
    let missing = std::path::PathBuf::from("no/such/key-file");
    for bad in ["https://127.0.0.1:8080", "http://localhost:8080", "http://evil.test", "http://127.0.0.1", "http://192.0.2.1:80", "http://127.0.0.1:8080/x"] {
        let err = prepare(None, 7380, Some(missing.clone()), Some(bad.to_string())).err().expect(bad);
        assert!(err.contains("--origin"), "{bad}: {err}");
    }
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k");
    std::fs::write(&key, "secret\n").unwrap();
    let t = prepare(None, 7380, Some(key.clone()), Some("http://127.1:8080/".into())).unwrap();
    assert_eq!(t.origin, "http://127.0.0.1:8080");
    let t = prepare(None, 7380, Some(key), None).unwrap();
    assert_eq!(t.origin, "http://127.0.0.1:7380");
}
