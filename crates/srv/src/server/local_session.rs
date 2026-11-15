// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The browser sessions of a headless srv that serves its own UI on loopback: one-time codes, the
//! pending sessions a code makes, and the live sessions a person confirms.
//!
//! The flow, from the person's side: `agentmux-srv open` mints a code with the key, the browser
//! enters it, the page shows a short check phrase, `open` shows the same phrase and asks the
//! person to confirm. Only a confirmed session is live. Everything here is memory only: a restart
//! ends every code, pending session and session.
//!
//! A live session is a pair: a cookie (sent by the browser to every port of the address, so it
//! proves little alone) and a per-tab token the page keeps in `sessionStorage` (isolated by
//! origin, port included). One record holds the hash of both, and a request needs both to name it
//! ([`SessionStore::admit`]). Secrets are kept as SHA-256 digests; a code, handle, cookie or token
//! never reaches a log.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::secret_eq::secret_eq;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

/// A code can be entered for this long after it is minted.
pub(crate) const CODE_LIFE: Duration = Duration::from_secs(60);
/// A pending session waits this long, from the moment its code was entered, for the person's answer.
pub(crate) const PENDING_LIFE: Duration = Duration::from_secs(60);
/// A session ends after this long without a request.
pub(crate) const SESSION_IDLE_LIFE: Duration = Duration::from_secs(12 * 60 * 60);
/// A session ends this long after it was confirmed, however much it is used: a browser that is
/// always open signs in again (with `agentmux-srv open`) once a week.
pub(crate) const SESSION_MAX_LIFE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Most codes and most pending sessions held at once; the oldest goes first.
const MAX_HELD: usize = 64;
/// Most refusals remembered at once (they are counted over a minute).
pub(crate) const MAX_FAILURES: usize = 1000;
/// This many refusals in a minute are logged one by one; after them, one line says the rest are only counted.
pub(crate) const LOGGED_FAILURES: usize = 5;

type Digest32 = [u8; 32];

fn digest(secret: &str) -> Digest32 {
    Sha256::digest(secret.as_bytes()).into()
}

/// 256 random bits as 64 hex characters (a valid WebSocket subprotocol token and cookie value).
pub(crate) fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new().fill(&mut bytes).expect("the system random source");
    hex::encode(bytes)
}

/// Two short pronounceable words from 1024 each (about 20 bits), e.g. `kela-mivo`: easy to compare by eye.
pub(crate) fn check_phrase() -> String {
    let mut bytes = [0u8; 4];
    SystemRandom::new().fill(&mut bytes).expect("the system random source");
    let a = u16::from_le_bytes([bytes[0], bytes[1]]) & 0x3ff;
    let b = u16::from_le_bytes([bytes[2], bytes[3]]) & 0x3ff;
    format!("{}-{}", phrase_word(a), phrase_word(b))
}

/// Word `n` (0..1024): consonant, vowel, consonant, vowel from 8 consonants and 4 vowels.
fn phrase_word(n: u16) -> String {
    const CONSONANTS: [char; 8] = ['k', 'l', 'm', 'n', 'r', 's', 't', 'v'];
    const VOWELS: [char; 4] = ['a', 'e', 'i', 'o'];
    let n = usize::from(n & 0x3ff);
    [CONSONANTS[n >> 7], VOWELS[(n >> 5) & 3], CONSONANTS[(n >> 2) & 7], VOWELS[n & 3]].iter().collect()
}

/// What a request to a route may present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteClass {
    /// The cookie and the token must both name one session. Every route unless listed below.
    Both,
    /// The cookie alone (for a GET that an element loads by URL and that changes nothing).
    CookieOnly,
}

/// What a session's page was handed, once.
pub(crate) struct Grant {
    pub cookie: String,
    pub token: String,
}

enum Outcome {
    Waiting,
    /// Confirmed. `Some` until the waiting page collects it.
    Granted(Option<Grant>),
    Discarded,
}

struct Pending {
    code: Digest32,
    phrase: String,
    born: Instant,
    outcome: Outcome,
    /// The session confirming made, to end it if the page never collected its grant.
    cookie: Option<Digest32>,
}

struct Session {
    token: Digest32,
    last_seen: Instant,
    /// When the person confirmed it: [`SESSION_MAX_LIFE`] counts from here.
    born: Instant,
}

#[derive(Default)]
struct Inner {
    /// Codes minted and not yet entered.
    codes: HashMap<Digest32, Instant>,
    /// Keyed by the page's handle.
    pending: HashMap<Digest32, Pending>,
    /// Keyed by the cookie's digest.
    sessions: HashMap<Digest32, Session>,
    failures: VecDeque<Instant>,
}

pub(crate) struct SessionStore {
    inner: Mutex<Inner>,
    /// Counts the logouts, so a socket upgraded with a session hears that it ended.
    revoked: tokio::sync::watch::Sender<u64>,
}

impl Default for SessionStore {
    fn default() -> Self {
        SessionStore { inner: Mutex::new(Inner::default()), revoked: tokio::sync::watch::channel(0).0 }
    }
}

/// Which session a request came from (the digest of its cookie), carried on the request so a
/// WebSocket upgraded with it can be closed when the session ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionId(Digest32);

impl SessionId {
    pub(crate) fn of_cookie(cookie: &str) -> SessionId {
        SessionId(digest(cookie))
    }
}

/// How often a socket re-checks that its session is still live (a logout is heard at once).
const LIVE_RECHECK: Duration = Duration::from_secs(10);

/// What a code entry made.
pub(crate) struct Entered {
    pub handle: String,
    pub phrase: String,
}

/// Where a code stands, for the `open` that minted it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CodeState {
    /// Minted and nobody has entered it yet.
    Unused,
    /// Entered; the browser that did shows this phrase.
    Entered { phrase: String },
    /// Expired, answered or never minted here.
    Gone,
}

/// The waiting page's answer.
pub(crate) enum Collected {
    Waiting,
    Granted(Grant),
    Refused,
}

impl SessionStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new one-time code: 256 random bits, single use, [`CODE_LIFE`].
    pub(crate) fn mint_code(&self, now: Instant) -> String {
        let code = random_secret();
        let mut inner = self.lock();
        prune(&mut inner, now);
        if inner.codes.len() >= MAX_HELD {
            if let Some(oldest) = inner.codes.iter().min_by_key(|(_, born)| **born).map(|(k, _)| *k) {
                inner.codes.remove(&oldest);
            }
        }
        inner.codes.insert(digest(&code), now);
        code
    }

    /// Enter a code: it is spent whether or not it was valid, and a valid one makes a pending
    /// session, which holds no access. `None` for a wrong, used or expired code.
    pub(crate) fn enter(&self, code: &str, now: Instant) -> Option<Entered> {
        let mut inner = self.lock();
        prune(&mut inner, now);
        let code = digest(code);
        inner.codes.remove(&code)?;
        if inner.pending.len() >= MAX_HELD {
            if let Some(oldest) = inner.pending.iter().min_by_key(|(_, p)| p.born).map(|(k, _)| *k) {
                end_pending(&mut inner, oldest);
            }
        }
        let handle = random_secret();
        let phrase = check_phrase();
        let pending = Pending { code, phrase: phrase.clone(), born: now, outcome: Outcome::Waiting, cookie: None };
        inner.pending.insert(digest(&handle), pending);
        Some(Entered { handle, phrase })
    }

    /// Where the code `open` minted stands. It looks nothing up by anything else.
    pub(crate) fn code_state(&self, code: &str, now: Instant) -> CodeState {
        let mut inner = self.lock();
        prune(&mut inner, now);
        let code = digest(code);
        if inner.codes.contains_key(&code) {
            return CodeState::Unused;
        }
        inner
            .pending
            .values()
            .find(|p| p.code == code && matches!(p.outcome, Outcome::Waiting))
            .map_or(CodeState::Gone, |p| CodeState::Entered { phrase: p.phrase.clone() })
    }

    /// The person's answer for the session made from `code`. True when a pending session of that
    /// code was waiting. Confirming makes the live session (its hashes), and keeps the cookie and
    /// token for the waiting page to collect once.
    pub(crate) fn decide(&self, code: &str, accept: bool, now: Instant) -> bool {
        let mut inner = self.lock();
        prune(&mut inner, now);
        let code = digest(code);
        let Some(handle) = inner.pending.iter().find(|(_, p)| p.code == code && matches!(p.outcome, Outcome::Waiting)).map(|(h, _)| *h)
        else {
            return false;
        };
        if !accept {
            if let Some(p) = inner.pending.get_mut(&handle) {
                p.outcome = Outcome::Discarded;
            }
            return true;
        }
        let grant = Grant { cookie: random_secret(), token: random_secret() };
        let cookie = digest(&grant.cookie);
        inner.sessions.insert(cookie, Session { token: digest(&grant.token), last_seen: now, born: now });
        if let Some(p) = inner.pending.get_mut(&handle) {
            p.cookie = Some(cookie);
            p.outcome = Outcome::Granted(Some(grant));
        }
        true
    }

    /// The waiting page asks, with its handle. A grant is handed over exactly once; a discarded,
    /// expired, spent or unknown handle is refused.
    pub(crate) fn collect(&self, handle: &str, now: Instant) -> Collected {
        let mut inner = self.lock();
        prune(&mut inner, now);
        let handle = digest(handle);
        let Some(pending) = inner.pending.get_mut(&handle) else {
            return Collected::Refused;
        };
        match &mut pending.outcome {
            Outcome::Waiting => Collected::Waiting,
            Outcome::Granted(grant) => match grant.take() {
                Some(grant) => {
                    // Spent: the handle dies with the answer.
                    inner.pending.remove(&handle);
                    Collected::Granted(grant)
                }
                None => Collected::Refused,
            },
            Outcome::Discarded => {
                inner.pending.remove(&handle);
                Collected::Refused
            }
        }
    }

    /// Does this request name a live session? `Both` needs the token too. A use slides the idle limit.
    pub(crate) fn admit(&self, cookie: Option<&str>, token: Option<&str>, class: RouteClass, now: Instant) -> bool {
        let Some(cookie) = cookie else { return false };
        let mut inner = self.lock();
        prune(&mut inner, now);
        let Some(session) = inner.sessions.get_mut(&digest(cookie)) else {
            return false;
        };
        let token_ok = match class {
            RouteClass::CookieOnly => true,
            RouteClass::Both => token.is_some_and(|t| secret_eq(&digest(t), &session.token)),
        };
        if token_ok {
            session.last_seen = now;
        }
        token_ok
    }

    /// End every session and every pending sign-in. How many sessions there were.
    pub(crate) fn revoke_all(&self) -> usize {
        let mut inner = self.lock();
        inner.codes.clear();
        inner.pending.clear();
        let n = inner.sessions.len();
        inner.sessions.clear();
        drop(inner);
        self.revoked.send_modify(|logouts| *logouts += 1);
        n
    }

    /// End one session (this browser's): the others stay. False when it was not live. A socket that
    /// was upgraded with it hears that it ended.
    pub(crate) fn revoke(&self, id: SessionId, now: Instant) -> bool {
        let mut inner = self.lock();
        prune(&mut inner, now);
        let was_live = inner.sessions.remove(&id.0).is_some();
        drop(inner);
        if was_live {
            self.revoked.send_modify(|logouts| *logouts += 1);
        }
        was_live
    }

    /// Is this session still live? A socket that asks is in use, so it slides the idle limit.
    pub(crate) fn is_live(&self, id: SessionId, now: Instant) -> bool {
        let mut inner = self.lock();
        prune(&mut inner, now);
        inner.sessions.get_mut(&id.0).map(|s| s.last_seen = now).is_some()
    }

    /// Completes when the session has ended: at a logout, and at the first re-check after it
    /// expired. A WebSocket is authenticated once, when it is upgraded, so it waits on this.
    pub(crate) async fn ended(&self, id: SessionId) {
        let mut logouts = self.revoked.subscribe();
        while self.is_live(id, Instant::now()) {
            tokio::select! {
                _ = logouts.changed() => {}
                _ = tokio::time::sleep(LIVE_RECHECK) => {}
            }
        }
    }

    pub(crate) fn session_count(&self, now: Instant) -> usize {
        let mut inner = self.lock();
        prune(&mut inner, now);
        inner.sessions.len()
    }

    /// Count a refused sign-in; how many in the last minute (at most [`MAX_FAILURES`]), this one
    /// included. The list is capped, so a flood of refusals holds no more memory than a trickle.
    pub(crate) fn note_failure(&self, now: Instant) -> usize {
        let mut inner = self.lock();
        let minute = Duration::from_secs(60);
        while inner.failures.front().is_some_and(|t| now.duration_since(*t) > minute) {
            inner.failures.pop_front();
        }
        if inner.failures.len() >= MAX_FAILURES {
            inner.failures.pop_front();
        }
        inner.failures.push_back(now);
        inner.failures.len()
    }
}

/// Drop what has expired. A grant nobody collected ends its session too.
fn prune(inner: &mut Inner, now: Instant) {
    inner.codes.retain(|_, born| now.duration_since(*born) <= CODE_LIFE);
    let stale: Vec<Digest32> =
        inner.pending.iter().filter(|(_, p)| now.duration_since(p.born) > PENDING_LIFE).map(|(h, _)| *h).collect();
    for handle in stale {
        end_pending(inner, handle);
    }
    inner.sessions.retain(|_, s| now.duration_since(s.last_seen) <= SESSION_IDLE_LIFE && now.duration_since(s.born) <= SESSION_MAX_LIFE);
}

fn end_pending(inner: &mut Inner, handle: Digest32) {
    if let Some(p) = inner.pending.remove(&handle) {
        if let (Outcome::Granted(Some(_)), Some(cookie)) = (&p.outcome, p.cookie) {
            inner.sessions.remove(&cookie);
        }
    }
}

#[cfg(test)]
#[path = "tests/local_session_tests.rs"]
mod tests;
