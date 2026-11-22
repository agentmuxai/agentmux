// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The values srv holds secret (its instance key, an agent's token, a Vault value it handed to an agent's process) are
//! never written into an agent's transcript, and never served to a paired device from one: a line that carries one is
//! stored, and shown to a device, with the value replaced by [`MARK`]. A value is matched as it is and in its ordinary
//! spellings (base64, URL-encoded, hex, as JSON holds it: `vault::scrub::forms`).
//!
//! The values are registered where srv learns them (a token is minted or loaded, the instance key is made, a Vault value
//! is handed to a process). The registry holds only what can still be used:
//! - [`register`] keeps a value for the life of the process;
//! - [`hold`] keeps a value for an owner, until its expiry, until the owner is [`release`]d (its token revoked, its
//!   container removed, its device unpaired) or until the owner holds another value in its place;
//! - [`hold_until`] keeps a value that works until a given time, and keeps it until then even when a renewed one
//!   follows it: the value that was replaced still works, so it stays hidden until it expires;
//! - [`hold_kept`] keeps a value with no known expiry (a refresh token, an API key) and keeps the values an owner held
//!   before it, up to [`KEPT_PER_OWNER`], oldest dropped first;
//! - a batch of registrations ([`register_all`]) costs one rebuild of the matcher, made when a line next needs it.
//!
//! The registry is bounded and never fails open. A value up to [`MAX_POOLED_LEN`] goes into the one pattern, and the
//! registry holds at most [`MAX_TOTAL_BYTES`] of those; a larger value (a certificate bundle, a key file) is held apart
//! and matched by an exact search, up to [`MAX_VALUE_LEN`] each and [`MAX_LARGE_BYTES`] in all. Within a budget the
//! oldest values an owner holds make room first; a value that does not fit is refused, counted and logged. A pattern
//! that cannot be built is replaced by an exact search over the same values, and if even that is over its limit every
//! line is read as the mark ([`MARK`]) in place of its text. Either state is logged once with the counts and reported
//! to the hook ([`set_alert`]).

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use regex::{NoExpand, Regex, RegexBuilder};

/// What a withheld value is replaced by.
pub const MARK: &str = "[withheld]";

/// Shorter values are not registered: matching them would mangle ordinary text.
pub const MIN_LEN: usize = 8;

/// How many values with no known expiry an owner keeps with [`hold_kept`].
pub const KEPT_PER_OWNER: usize = 8;

/// The longest value the registry takes (bytes): a longer one is refused.
pub const MAX_VALUE_LEN: usize = 32 * 1024 * 1024;

/// The longest value that goes into the one pattern; a longer one is held apart and matched by an exact search.
pub const MAX_POOLED_LEN: usize = 32 * 1024;

/// The most the pattern's values hold (bytes of values, not of spellings): past it the oldest values an owner holds
/// make room.
pub const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

/// The most the values over [`MAX_POOLED_LEN`] hold in all (bytes).
pub const MAX_LARGE_BYTES: usize = 32 * 1024 * 1024;

/// The size limit of the one-pattern matcher, and of the exact search that replaces it (bytes of spellings).
const REGEX_LIMIT: usize = 64 << 20;
const EXACT_LIMIT: usize = 32 << 20;

/// How many values were refused, and how many times the one-pattern matcher could not be built.
static REFUSED: AtomicU64 = AtomicU64::new(0);
/// How many values stopped being held to make room for newer ones.
static EVICTED: AtomicU64 = AtomicU64::new(0);
static FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// How the registry matches.
#[derive(Clone)]
enum Matcher {
    /// One pattern over every value in every spelling.
    Pattern(Arc<Regex>),
    /// The same spellings, longest first, searched one at a time: slower, and with no size limit but [`EXACT_LIMIT`].
    Exact(Arc<Vec<String>>),
    /// Neither could be used: every line reads as [`MARK`].
    Closed,
}

/// What the registry matches with: the pattern (or its fallbacks) for the pooled values, and the large values apart.
#[derive(Clone)]
struct Built {
    small: Option<Matcher>,
    /// The spellings of the values over [`MAX_POOLED_LEN`] (plain, as JSON holds it, base64).
    large: Arc<Vec<String>>,
}

/// The state of the registry, for a health read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// `none` (nothing registered), `pattern`, `exact` or `closed`.
    pub mode: &'static str,
    pub values: usize,
    /// How many of the values are over [`MAX_POOLED_LEN`].
    pub large: usize,
    pub bytes: usize,
    /// Values refused since the process started.
    pub refused: u64,
    /// Values that stopped being held, since the process started, to make room for newer ones.
    pub evicted: u64,
    /// Times the one-pattern matcher could not be built.
    pub fallbacks: u64,
}

struct Entry {
    /// Who holds the value; `None` for one kept for the life of the process.
    owner: Option<String>,
    value: String,
    /// Unix ms from which the value no longer works and is dropped.
    expires_at: Option<i64>,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    matcher: Option<Built>,
    /// Told to the alert hook once the lock is released.
    alerts: Vec<String>,
    /// The sizes the matcher is built under (`None`: the constants), so a test can force a failure.
    limits: Option<(usize, usize)>,
    /// The values changed since `matcher` was built.
    stale: bool,
    /// The earliest expiry among the entries.
    next_expiry: Option<i64>,
    /// How many times the matcher has been built.
    rebuilds: usize,
}

impl Registry {
    fn bytes(&self) -> usize {
        self.entries.iter().map(|e| e.value.len()).sum()
    }

    /// Counts a refusal and queues a line for the log and the alert hook: the first, then each power of two.
    fn refuse(&mut self, why: &str, len: usize) {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_power_of_two() {
            let line = format!("transcript withhold: a value was refused ({why}, {len} bytes); {n} refused since start");
            tracing::warn!(refused = n, len, why, "{line}");
            self.alerts.push(line);
        }
    }

    /// Counts `n` values that stopped being held to make room, and queues a line for the log and the alert hook when
    /// the count first reaches or passes a power of two.
    fn evicted(&mut self, n: u64) {
        let total = EVICTED.fetch_add(n, Ordering::Relaxed) + n;
        if (total - n + 1..=total).any(u64::is_power_of_two) {
            let line = format!("transcript withhold: {n} older value(s) are no longer held to make room; {total} since start");
            tracing::warn!(evicted = total, "{line}");
            self.alerts.push(line);
        }
    }

    /// Whether a value of `len` bytes can be taken at all.
    fn takes(&mut self, len: usize) -> bool {
        if len > MAX_VALUE_LEN {
            self.refuse("over the longest value", len);
            return false;
        }
        true
    }

    /// Makes the registry fit its budget after a value was pushed last, and says whether that value is kept. For its
    /// kind (pooled, or over [`MAX_POOLED_LEN`]) the oldest values an owner holds go first. When what cannot make room
    /// (the values kept for the process, and the new one) is already over the budget, nothing is evicted: the new
    /// value is refused and every other value stays.
    fn fit(&mut self) -> bool {
        let Some(newest) = self.entries.last() else { return true };
        let large = newest.value.len() > MAX_POOLED_LEN;
        let budget = if large { MAX_LARGE_BYTES } else { MAX_TOTAL_BYTES };
        let same_kind = |e: &Entry| (e.value.len() > MAX_POOLED_LEN) == large;
        let last = self.entries.len() - 1;
        let fixed: usize = self.entries.iter().enumerate().filter(|(i, e)| same_kind(e) && (e.owner.is_none() || *i == last)).map(|(_, e)| e.value.len()).sum();
        if fixed > budget {
            let len = self.entries.pop().map_or(0, |e| e.value.len());
            self.refuse("the registry is full", len);
            return false;
        }
        let mut evicted = 0;
        while self.entries.iter().filter(|e| same_kind(e)).map(|e| e.value.len()).sum::<usize>() > budget {
            let last = self.entries.len() - 1;
            if let Some(oldest) = self.entries[..last].iter().position(|e| e.owner.is_some() && same_kind(e)) {
                self.entries.remove(oldest);
                evicted += 1;
            } else {
                break;
            }
        }
        if evicted > 0 {
            self.evicted(evicted);
        }
        true
    }

    fn reindex(&mut self) {
        self.next_expiry = self.entries.iter().filter_map(|e| e.expires_at).min();
    }

    /// Keeps `value` for the process (`owner` `None`) or for `owner`, in place of what the owner held. A value
    /// shorter than [`MIN_LEN`] is not kept, so an owner given one holds nothing.
    fn add(&mut self, owner: Option<&str>, value: &str, expires_at: Option<i64>) {
        let value = value.trim();
        let keep = value.len() >= MIN_LEN;
        if keep && !self.takes(value.len()) {
            // A refused value changes nothing: what the owner held is still held.
            return;
        }
        let mine = |e: &Entry| e.owner.as_deref() == owner;
        if let Some(same) = self.entries.iter_mut().find(|e| mine(e) && e.value == value) {
            if keep && owner.is_none() {
                return;
            }
            if keep && owner.is_some() {
                // The same value again: the matcher stays, only the expiry can move.
                same.expires_at = expires_at;
                self.entries.retain(|e| !(mine(e) && e.value != value));
                self.reindex();
                return;
            }
        }
        let was_stale = self.stale;
        let mut old = Vec::new();
        if owner.is_some() {
            let (theirs, rest) = std::mem::take(&mut self.entries).into_iter().partition(|e| mine(e));
            self.entries = rest;
            old = theirs;
            self.stale |= !old.is_empty();
        }
        if keep {
            self.entries.push(Entry { owner: owner.map(str::to_string), value: value.to_string(), expires_at });
            if self.fit() {
                self.stale = true;
            } else {
                // Not kept for lack of room: what the owner held is still held.
                self.entries.extend(old);
                self.stale = was_stale;
            }
        }
        self.reindex();
    }

    /// Keeps `value` for `owner` along with what the owner kept before, up to [`KEPT_PER_OWNER`] (the oldest is dropped).
    fn add_kept(&mut self, owner: &str, value: &str) {
        let value = value.trim();
        let mine = |e: &Entry| e.owner.as_deref() == Some(owner);
        if value.len() < MIN_LEN || self.entries.iter().any(|e| mine(e) && e.value == value) || !self.takes(value.len()) {
            return;
        }
        self.entries.push(Entry { owner: Some(owner.to_string()), value: value.to_string(), expires_at: None });
        if !self.fit() {
            return;
        }
        if self.entries.iter().filter(|e| mine(e)).count() > KEPT_PER_OWNER {
            if let Some(oldest) = self.entries.iter().position(|e| mine(e)) {
                self.entries.remove(oldest);
            }
        }
        self.stale = true;
        self.reindex();
    }

    /// Drops what `owner` holds, and what it holds with [`hold_until`].
    fn release_all(&mut self, owner: &str) {
        let until = format!("{owner}#");
        let before = self.entries.len();
        self.entries.retain(|e| e.owner.as_deref().map_or(true, |o| o != owner && !o.starts_with(&until)));
        if self.entries.len() != before {
            self.stale = true;
            self.reindex();
        }
    }

    /// Drops what every owner whose name starts with `prefix` holds.
    fn release_prefix(&mut self, prefix: &str) {
        let before = self.entries.len();
        self.entries.retain(|e| !e.owner.as_deref().is_some_and(|o| o.starts_with(prefix)));
        if self.entries.len() != before {
            self.stale = true;
            self.reindex();
        }
    }

    fn release(&mut self, owner: &str) {
        let before = self.entries.len();
        self.entries.retain(|e| e.owner.as_deref() != Some(owner));
        if self.entries.len() != before {
            self.stale = true;
            self.reindex();
        }
    }

    /// Drops what has run out by `now`.
    fn expire(&mut self, now: i64) {
        if self.next_expiry.is_some_and(|at| at <= now) {
            self.entries.retain(|e| e.expires_at.map_or(true, |at| at > now));
            self.stale = true;
            self.reindex();
        }
    }

    /// Whether the matcher is right for `now` as it stands.
    fn current(&self, now: i64) -> bool {
        !self.stale && self.next_expiry.map_or(true, |at| at > now)
    }

    /// The matcher for the values that have not run out at `now`, built only when they changed.
    fn matcher(&mut self, now: i64) -> Option<Built> {
        self.expire(now);
        if self.stale {
            let values: Vec<&str> = self.entries.iter().map(|e| e.value.as_str()).collect();
            let (regex_limit, exact_limit) = self.limits.unwrap_or((REGEX_LIMIT, EXACT_LIMIT));
            let was = mode_of(self.matcher.as_ref());
            self.matcher = build(&values, regex_limit, exact_limit);
            let now_mode = mode_of(self.matcher.as_ref());
            if matches!(now_mode, "exact" | "closed") && now_mode != was {
                let n = FALLBACKS.load(Ordering::Relaxed);
                let line = format!(
                    "transcript withhold: matching is degraded to {now_mode} ({} values, {} bytes; {n} builds failed, {} refused since start)",
                    self.entries.len(),
                    self.bytes(),
                    REFUSED.load(Ordering::Relaxed)
                );
                tracing::warn!(mode = now_mode, values = self.entries.len(), "{line}");
                self.alerts.push(line);
            }
            self.rebuilds += 1;
            self.stale = false;
        }
        self.matcher.clone()
    }
}

fn mode_of(built: Option<&Built>) -> &'static str {
    match built.map(|b| (&b.small, !b.large.is_empty())) {
        None => "none",
        Some((Some(Matcher::Pattern(_)), _)) => "pattern",
        Some((Some(Matcher::Exact(_)), _)) => "exact",
        Some((Some(Matcher::Closed), _)) => "closed",
        Some((None, _)) => "large",
    }
}

fn state() -> &'static RwLock<Registry> {
    static STATE: OnceLock<RwLock<Registry>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

fn write() -> std::sync::RwLockWriteGuard<'static, Registry> {
    state().write().unwrap_or_else(|e| e.into_inner())
}

type Alert = Box<dyn Fn(&str) + Send + Sync>;

static ALERT: OnceLock<Alert> = OnceLock::new();

/// Sets what is told when the registry refuses a value or degrades its matching: one line with the counts, never a
/// value. Called outside the registry's lock, so it may read or write the registry. The first call wins.
pub fn set_alert(alert: impl Fn(&str) + Send + Sync + 'static) {
    let _ = ALERT.set(Box::new(alert));
}

/// Runs `f` on the registry under its write lock, then tells the alert hook what it queued.
fn with_write<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    let (result, alerts) = {
        let mut registry = write();
        let result = f(&mut registry);
        (result, std::mem::take(&mut registry.alerts))
    };
    if let Some(alert) = ALERT.get() {
        alerts.iter().for_each(|line| alert(line));
    }
    result
}

/// The state of the registry now.
pub fn health() -> Health {
    let matcher = matcher();
    let registry = state().read().unwrap_or_else(|e| e.into_inner());
    Health {
        mode: mode_of(matcher.as_ref()),
        values: registry.entries.len(),
        large: registry.entries.iter().filter(|e| e.value.len() > MAX_POOLED_LEN).count(),
        bytes: registry.bytes(),
        refused: REFUSED.load(Ordering::Relaxed),
        evicted: EVICTED.load(Ordering::Relaxed),
        fallbacks: FALLBACKS.load(Ordering::Relaxed),
    }
}

/// Registers `value` to be withheld from now on, for the life of the process. A value shorter than [`MIN_LEN`] and one
/// already registered change nothing.
pub fn register(value: &str) {
    with_write(|r| r.add(None, value, None));
}

/// [`register`] for each of `values`, under one lock: the matcher is built once for the lot, when a line next needs it.
pub fn register_all<'a>(values: impl IntoIterator<Item = &'a str>) {
    with_write(|registry| {
        for value in values {
            registry.add(None, value, None);
        }
    });
}

/// Withholds `value` for `owner`, in place of what the owner held before, until `expires_at` (unix ms; `None` for no
/// expiry) or until the owner is [`release`]d. Holding the value the owner already holds changes nothing.
pub fn hold(owner: &str, value: &str, expires_at: Option<i64>) {
    with_write(|r| r.add(Some(owner), value, expires_at));
}

/// Withholds `value` for `owner` until `expires_at` (unix ms), when it stops working. A renewed value the owner holds
/// next with [`hold_until`] does not take its place: the value that was replaced still works until its own expiry,
/// so it stays withheld until then. Holding the same value and expiry again changes nothing.
pub fn hold_until(owner: &str, value: &str, expires_at: i64) {
    with_write(|r| r.add(Some(&format!("{owner}#{expires_at}")), value, Some(expires_at)));
}

/// Withholds `value` for `owner` with no expiry, until the process ends or the owner is [`release`]d. The values the
/// owner kept before it stay withheld too, up to [`KEPT_PER_OWNER`]: past that the oldest is dropped. Keeping the same
/// value again changes nothing.
pub fn hold_kept(owner: &str, value: &str) {
    with_write(|r| r.add_kept(owner, value));
}

/// Stops withholding what `owner` holds: its value no longer works.
pub fn release(owner: &str) {
    with_write(|r| r.release(owner));
}

/// [`release_all`] for every owner whose name starts with `prefix`.
pub fn release_prefix(prefix: &str) {
    with_write(|r| r.release_prefix(prefix));
}

/// [`release`] for what `owner` holds and what it holds with [`hold_until`].
pub fn release_all(owner: &str) {
    with_write(|r| r.release_all(owner));
}

/// The spellings of a value over [`MAX_POOLED_LEN`]: as it is, as a JSON string holds it, and base64.
fn large_forms(value: &str) -> Vec<String> {
    use base64::Engine;
    let mut out = vec![value.to_string()];
    let json = serde_json::to_string(value).unwrap_or_default();
    if let Some(plain) = json.strip_prefix('"').and_then(|q| q.strip_suffix('"')) {
        out.push(plain.to_string());
    }
    out.push(base64::engine::general_purpose::STANDARD.encode(value));
    out.dedup();
    out
}

/// What matches `values`. The values up to [`MAX_POOLED_LEN`] go into one matcher for every value in every spelling,
/// longest first so a spelling that contains another is replaced whole; when one pattern cannot be built under
/// `regex_limit` the same spellings are searched one at a time, and when they are over `exact_limit` bytes the matcher
/// is [`Matcher::Closed`]. The larger values are held apart and searched exactly. `None` when nothing is registered.
fn build(values: &[&str], regex_limit: usize, exact_limit: usize) -> Option<Built> {
    let (large, pooled): (Vec<&str>, Vec<&str>) = values.iter().partition(|v| v.len() > MAX_POOLED_LEN);
    let mut large_needles: Vec<String> = large.iter().flat_map(|v| large_forms(v)).filter(|f| f.len() >= MIN_LEN).collect();
    large_needles.sort_by(|a, b| b.len().cmp(&a.len()));
    large_needles.dedup();
    let small = build_pooled(&pooled, regex_limit, exact_limit);
    if small.is_none() && large_needles.is_empty() {
        return None;
    }
    Some(Built { small, large: Arc::new(large_needles) })
}

fn build_pooled(values: &[&str], regex_limit: usize, exact_limit: usize) -> Option<Matcher> {
    let mut needles: Vec<String> = Vec::new();
    for value in values {
        for form in crate::backend::vault::scrub::forms(value) {
            if form.len() >= MIN_LEN && !needles.contains(&form) {
                needles.push(form);
            }
        }
    }
    if needles.is_empty() {
        return None;
    }
    needles.sort_by(|a, b| b.len().cmp(&a.len()));
    let pattern = needles.iter().map(|n| regex::escape(n)).collect::<Vec<_>>().join("|");
    // A limit of 0 means no one-pattern matcher at all.
    let built = if regex_limit == 0 { Err("no pattern allowed".to_string()) } else { RegexBuilder::new(&pattern).size_limit(regex_limit).build().map_err(|e| e.to_string()) };
    match built {
        Ok(matcher) => Some(Matcher::Pattern(Arc::new(matcher))),
        Err(e) => {
            FALLBACKS.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(error = %e, values = values.len(), "transcript withhold: the matcher could not be built as one pattern");
            let bytes: usize = needles.iter().map(String::len).sum();
            Some(if bytes > exact_limit { Matcher::Closed } else { Matcher::Exact(Arc::new(needles)) })
        }
    }
}

fn matcher() -> Option<Built> {
    let now = agentmux_common::time::now_ms();
    {
        let registry = state().read().unwrap_or_else(|e| e.into_inner());
        if registry.current(now) {
            return registry.matcher.clone();
        }
    }
    with_write(|r| r.matcher(now))
}

/// `text` with every value in `small` replaced by [`MARK`].
fn replaced_small<'a>(matcher: &Matcher, text: &'a str) -> Cow<'a, str> {
    match matcher {
        Matcher::Pattern(regex) => regex.replace_all(text, NoExpand(MARK)),
        Matcher::Exact(needles) => {
            let mut out = Cow::Borrowed(text);
            for needle in needles.iter() {
                if out.contains(needle.as_str()) {
                    out = Cow::Owned(out.replace(needle.as_str(), MARK));
                }
            }
            out
        }
        Matcher::Closed => Cow::Owned(MARK.to_string()),
    }
}

/// `text` with every value `built` knows replaced by [`MARK`].
fn replaced<'a>(built: &Built, text: &'a str) -> Cow<'a, str> {
    let mut out = match &built.small {
        Some(small) => replaced_small(small, text),
        None => Cow::Borrowed(text),
    };
    for needle in built.large.iter() {
        if needle.len() <= out.len() && out.contains(needle.as_str()) {
            out = Cow::Owned(out.replace(needle.as_str(), MARK));
        }
    }
    out
}

/// `text` with every registered value replaced by [`MARK`]; the text itself when it holds none. Never the text of a
/// line when matching is closed: then every line is the mark.
pub fn text(text: &str) -> Cow<'_, str> {
    match matcher() {
        Some(built) => replaced(&built, text),
        None => Cow::Borrowed(text),
    }
}

/// Whether `text` holds a registered value, in any of its spellings: the same matcher [`text`] replaces with. Always
/// true when matching is closed.
pub fn holds(text: &str) -> bool {
    let Some(built) = matcher() else { return false };
    let small = match &built.small {
        Some(Matcher::Pattern(regex)) => regex.is_match(text),
        Some(Matcher::Exact(needles)) => needles.iter().any(|n| text.contains(n.as_str())),
        Some(Matcher::Closed) => true,
        None => false,
    };
    small || built.large.iter().any(|n| text.contains(n.as_str()))
}

/// [`text`] over transcript bytes (NDJSON lines); bytes that are not UTF-8 are left as they are, except when matching is
/// closed: then every line is the mark.
pub fn lines(data: &[u8]) -> Cow<'_, [u8]> {
    if matcher().is_some_and(|b| matches!(b.small, Some(Matcher::Closed))) {
        return Cow::Owned(MARK.as_bytes().to_vec());
    }
    let Ok(as_text) = std::str::from_utf8(data) else { return Cow::Borrowed(data) };
    match text(as_text) {
        Cow::Borrowed(_) => Cow::Borrowed(data),
        Cow::Owned(replaced) => Cow::Owned(replaced.into_bytes()),
    }
}

/// [`text`] over each line, in place.
pub fn in_place(lines: &mut [String]) {
    for line in lines {
        if let Cow::Owned(replaced) = text(line) {
            *line = replaced;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    // The registry is process-wide and tests run in parallel: every test registers values of its own.

    #[test]
    fn a_registered_value_is_withheld_in_every_ordinary_spelling() {
        let value = "canary-withhold-0001-aB3dE";
        register(value);
        let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
        for shown in [
            value.to_string(),
            base64::engine::general_purpose::STANDARD.encode(value),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value),
            hex,
        ] {
            let line = format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"env: KEY={shown}"}}]}}}}"#);
            let out = text(&line);
            assert!(!out.contains(&shown), "{shown} reached the line: {out}");
            assert!(out.contains(MARK), "{out}");
            assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok(), "the line stays JSON: {out}");
        }
    }

    #[test]
    fn a_value_with_json_special_characters_is_withheld_as_json_holds_it() {
        let value = "canary\"withhold\\0002/ö-xY";
        register(value);
        let line = serde_json::json!({ "type": "user", "content": format!("echo {value}") }).to_string();
        assert!(!line.contains(value), "the stored form is escaped, not the plain value");
        let out = text(&line);
        assert!(out.contains(MARK) && !out.contains("withhold"), "{out}");
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok(), "{out}");
    }

    #[test]
    fn text_with_no_registered_value_is_returned_as_it_is() {
        register("canary-withhold-0003-zQ9");
        assert!(matches!(text("an ordinary line"), Cow::Borrowed(_)));
        assert!(matches!(lines(b"{\"type\":\"result\"}\n"), Cow::Borrowed(_)));
        assert!(matches!(lines(&[0xff, 0xfe, 0x00]), Cow::Borrowed(_)), "bytes that are not text are left alone");
    }

    #[test]
    fn a_value_too_short_to_match_safely_is_not_registered() {
        register("shorty");
        assert_eq!(text("shorty and shorty"), "shorty and shorty");
    }

    #[test]
    fn a_value_registered_later_is_withheld_from_then_on() {
        let before = "canary-withhold-0004-late";
        assert_eq!(text(&format!("key {before}")), format!("key {before}"));
        register(before);
        assert_eq!(text(&format!("key {before}")), format!("key {MARK}"));
        register(before);
        assert_eq!(text(&format!("{before}{before}")), format!("{MARK}{MARK}"), "a value registered twice is one value");
    }

    #[test]
    fn lines_and_in_place_withhold_what_text_does() {
        let value = "canary-withhold-0005-lines";
        register(value);
        let out = lines(format!("a {value}\nb {value}\n").as_bytes()).into_owned();
        assert_eq!(String::from_utf8(out).unwrap(), format!("a {MARK}\nb {MARK}\n"));
        let mut many = vec![format!("x {value}"), "clean".to_string()];
        in_place(&mut many);
        assert_eq!(many, vec![format!("x {MARK}"), "clean".to_string()]);
    }

    // The tests below use a registry of their own and a clock they set, so they say nothing about the process-wide one.

    fn shows(registry: &mut Registry, now: i64, value: &str) -> bool {
        let line = format!("KEY={value}");
        registry.matcher(now).map_or(line.clone(), |m| replaced(&m, &line).into_owned()).contains(value)
    }

    fn big(n: usize) -> String {
        format!("canary-big-{n:04}-{}", "x".repeat(MAX_VALUE_LEN))
    }

    #[test]
    fn a_value_is_dropped_at_its_expiry_and_a_live_one_stays() {
        let mut registry = Registry::default();
        registry.add(Some("short"), "canary-expiry-0001-short", Some(1_000));
        registry.add(Some("long"), "canary-expiry-0002-long", Some(9_000));
        registry.add(None, "canary-expiry-0003-forever", None);
        for value in ["canary-expiry-0001-short", "canary-expiry-0002-long", "canary-expiry-0003-forever"] {
            assert!(!shows(&mut registry, 999, value), "{value} is withheld before any expiry");
        }
        assert!(shows(&mut registry, 1_000, "canary-expiry-0001-short"), "an expired value is dropped");
        assert!(!shows(&mut registry, 1_000, "canary-expiry-0002-long"), "a live one stays");
        assert!(!shows(&mut registry, 1_000, "canary-expiry-0003-forever"), "one with no expiry stays");
        assert_eq!(registry.entries.len(), 2, "the expired value is not kept");
        assert!(shows(&mut registry, 9_000, "canary-expiry-0002-long"));
        assert!(!shows(&mut registry, 9_000, "canary-expiry-0003-forever"));
    }

    #[test]
    fn a_released_owner_is_dropped_and_the_others_stay() {
        let mut registry = Registry::default();
        registry.add(Some("container:a"), "canary-release-0001-a", None);
        registry.add(Some("container:b"), "canary-release-0002-b", None);
        registry.release("container:a");
        assert!(shows(&mut registry, 0, "canary-release-0001-a"), "a revoked value is no longer matched");
        assert!(!shows(&mut registry, 0, "canary-release-0002-b"));
        registry.release("container:a");
        registry.release("no-such-owner");
        assert_eq!(registry.entries.len(), 1, "releasing what is not held changes nothing");
    }

    #[test]
    fn an_owner_given_a_new_value_holds_only_that_one() {
        let mut registry = Registry::default();
        registry.add(Some("signin:acct:refresh"), "canary-replace-0001-old", None);
        registry.add(Some("signin:acct:refresh"), "canary-replace-0002-new", None);
        assert!(shows(&mut registry, 0, "canary-replace-0001-old"), "the value that was replaced is dropped");
        assert!(!shows(&mut registry, 0, "canary-replace-0002-new"));
        registry.add(Some("signin:acct:refresh"), "short", None);
        assert!(registry.entries.is_empty(), "a value too short to match holds nothing, and the old one is gone");
    }

    #[test]
    fn a_value_held_again_with_a_later_expiry_is_kept_to_that_expiry() {
        let mut registry = Registry::default();
        registry.add(Some("github:a"), "canary-again-0001-token", Some(1_000));
        registry.add(Some("github:a"), "canary-again-0001-token", Some(5_000));
        assert_eq!(registry.entries.len(), 1);
        assert!(!shows(&mut registry, 2_000, "canary-again-0001-token"));
        assert!(shows(&mut registry, 5_000, "canary-again-0001-token"));
    }

    #[test]
    fn a_renewed_value_does_not_end_the_withholding_of_the_one_it_replaced_before_that_one_expires() {
        let mut registry = Registry::default();
        // What hold_until does: one owner per expiry.
        registry.add(Some("github:a#3600"), "canary-renew-0001-first", Some(3_600));
        registry.add(Some("github:a#7200"), "canary-renew-0002-second", Some(7_200));
        assert!(!shows(&mut registry, 3_599, "canary-renew-0001-first"), "the replaced one still works");
        assert!(!shows(&mut registry, 3_599, "canary-renew-0002-second"));
        assert!(shows(&mut registry, 3_600, "canary-renew-0001-first"), "dropped at its own expiry");
        assert!(!shows(&mut registry, 3_600, "canary-renew-0002-second"));
        assert_eq!(registry.entries.len(), 1, "the registry holds only what can still be used");
        assert!(shows(&mut registry, 7_200, "canary-renew-0002-second"));
        assert!(registry.entries.is_empty());
    }

    #[test]
    fn kept_values_stay_until_the_cap_and_then_the_oldest_goes() {
        let mut registry = Registry::default();
        for n in 0..KEPT_PER_OWNER + 2 {
            registry.add_kept("acct:a", &format!("canary-kept-{n:04}-value"));
            registry.add_kept("acct:a", &format!("canary-kept-{n:04}-value"));
        }
        registry.add_kept("acct:b", "canary-kept-other-value");
        assert_eq!(registry.entries.iter().filter(|e| e.owner.as_deref() == Some("acct:a")).count(), KEPT_PER_OWNER);
        assert!(shows(&mut registry, 0, "canary-kept-0000-value"), "the oldest was dropped");
        assert!(shows(&mut registry, 0, "canary-kept-0001-value"));
        assert!(!shows(&mut registry, 0, "canary-kept-0002-value"));
        assert!(!shows(&mut registry, 0, &format!("canary-kept-{:04}-value", KEPT_PER_OWNER + 1)));
        assert!(!shows(&mut registry, 0, "canary-kept-other-value"), "another owner's values are its own");
        assert!(!shows(&mut registry, u64::MAX as i64, "canary-kept-0002-value"), "no expiry");
        registry.release("acct:a");
        assert!(shows(&mut registry, 0, "canary-kept-0002-value"));
    }

    #[test]
    fn release_prefix_drops_every_owner_under_the_prefix_and_no_other() {
        let mut registry = Registry::default();
        registry.add(Some("acct:a:access#100"), "canary-prefix-0001-first", Some(100));
        registry.add_kept("acct:a:refresh", "canary-prefix-0002-second");
        registry.add(Some("acct:b:refresh"), "canary-prefix-0003-other", None);
        registry.release_prefix("acct:a:");
        assert_eq!(registry.entries.len(), 1);
        assert!(!shows(&mut registry, 0, "canary-prefix-0003-other"));
    }

    #[test]
    fn one_oversized_registration_is_refused_and_leaves_every_other_value_withheld() {
        let mut registry = Registry::default();
        registry.add(Some("acct:ok"), "canary-oversize-0001-ok", None);
        registry.add(Some("acct:evil"), &big(1), None);
        registry.add_kept("acct:evil", &big(2));
        registry.add(None, &big(3), None);
        assert_eq!(registry.entries.len(), 1, "an oversized value is refused, whoever registers it and however");
        assert!(!shows(&mut registry, 0, "canary-oversize-0001-ok"));
        assert!(REFUSED.load(Ordering::Relaxed) >= 3, "each refusal is counted");
    }

    #[test]
    fn a_value_over_the_pooled_length_is_withheld_by_an_exact_search_and_the_small_values_keep_the_pattern() {
        use base64::Engine as _;
        let mut registry = Registry::default();
        let large = format!("canary-large-0001-{}", "z".repeat(100 * 1024));
        registry.add(Some("acct:small"), "canary-large-small-0001", None);
        registry.add(Some("acct:large"), &large, None);
        let built = registry.matcher(0).unwrap();
        assert!(matches!(built.small, Some(Matcher::Pattern(_))), "the small values still go through the pattern");
        assert!(built.large.len() >= 2, "the large one is held apart, in its spellings");
        assert!(!shows(&mut registry, 0, &large), "a 100 KiB value is withheld");
        assert!(!shows(&mut registry, 0, "canary-large-small-0001"));
        let spelled = base64::engine::general_purpose::STANDARD.encode(&large);
        assert!(!replaced(&built, &format!("out: {spelled}")).contains("canary-large"), "also as base64");
        assert!(replaced(&built, "an ordinary line").as_ref() == "an ordinary line");
    }

    #[test]
    fn the_large_values_have_a_total_of_their_own_and_the_oldest_owned_make_room() {
        let mut registry = Registry::default();
        let each = "q".repeat(1024 * 1024);
        for n in 0..40 {
            registry.add(Some(&format!("acct:{n}")), &format!("canary-lgroom-{n:04}-{each}"), None);
        }
        let large: usize = registry.entries.iter().filter(|e| e.value.len() > MAX_POOLED_LEN).map(|e| e.value.len()).sum();
        assert!(large <= MAX_LARGE_BYTES);
        assert!(registry.entries.iter().any(|e| e.value.starts_with("canary-lgroom-0039")), "the newest is held");
        assert!(!registry.entries.iter().any(|e| e.value.starts_with("canary-lgroom-0000")), "the oldest made room");
    }

    #[test]
    fn the_registry_holds_a_bounded_total_and_the_oldest_owned_values_make_room() {
        let mut registry = Registry::default();
        let each = "y".repeat(MAX_POOLED_LEN - 64);
        for n in 0..100 {
            registry.add(Some(&format!("acct:{n}")), &format!("canary-room-{n:04}-{each}"), None);
        }
        assert!(registry.bytes() <= MAX_TOTAL_BYTES);
        assert!(shows(&mut registry, 0, &format!("canary-room-0000-{each}")), "the oldest made room");
        assert!(!shows(&mut registry, 0, &format!("canary-room-0099-{each}")), "the newest is held");

        // Values kept for the process cannot make room: the one that does not fit is refused, the others stay.
        let mut registry = Registry::default();
        for n in 0..100 {
            registry.add(None, &format!("canary-forever-{n:04}-{each}"), None);
        }
        assert!(registry.bytes() <= MAX_TOTAL_BYTES);
        assert!(!shows(&mut registry, 0, &format!("canary-forever-0000-{each}")));
    }

    #[test]
    fn values_that_stop_being_held_to_make_room_are_counted() {
        let before = EVICTED.load(Ordering::Relaxed);
        let mut registry = Registry::default();
        let each = "y".repeat(MAX_POOLED_LEN - 64);
        for n in 0..100 {
            registry.add(Some(&format!("acct:{n}")), &format!("canary-count-{n:04}-{each}"), None);
        }
        assert!(EVICTED.load(Ordering::Relaxed) - before >= 30, "each value made to make room is counted");
        assert!(health().evicted >= 30);
    }

    #[test]
    fn a_replacement_that_is_refused_leaves_what_its_owner_held_and_every_other_value_hidden() {
        // Too long to be taken at all.
        let mut registry = Registry::default();
        registry.add(Some("acct:a"), "canary-keep-0001-first", None);
        registry.add(Some("acct:b"), "canary-keep-0002-other", None);
        registry.add(Some("acct:a"), &big(1), None);
        registry.add(Some("acct:a"), &big(2), Some(1_000));
        assert!(!shows(&mut registry, 0, "canary-keep-0001-first"), "the owner's value is still held");
        assert!(!shows(&mut registry, 0, "canary-keep-0002-other"));

        // No room, because values kept for the process fill the registry: nothing is evicted for a value that
        // cannot be kept.
        let mut registry = Registry::default();
        registry.add(Some("acct:a"), "canary-keep-0003-first", None);
        registry.add(Some("acct:b"), "canary-keep-0004-other", None);
        let fixed = "z".repeat(MAX_POOLED_LEN);
        for n in 0..63 {
            registry.add(None, &format!("{n:04}{}", &fixed[4..]), None);
        }
        registry.add(None, &"w".repeat(20_000), None);
        let before = registry.entries.len();
        registry.add(Some("acct:a"), &format!("canary-keep-0005-{}", "v".repeat(19_000)), None);
        assert_eq!(registry.entries.len(), before, "refused: nothing changed");
        assert!(!shows(&mut registry, 0, "canary-keep-0003-first"), "the owner's value is still held");
        assert!(!shows(&mut registry, 0, "canary-keep-0004-other"), "another owner's value is still held");
    }

    #[test]
    fn a_matcher_that_cannot_be_built_as_one_pattern_is_replaced_by_an_exact_search() {
        let mut registry = Registry::default();
        registry.add(Some("acct:a"), "canary-exact-0001-first", None);
        registry.add(Some("acct:b"), "canary-exact-0002-second", None);
        registry.limits = Some((0, EXACT_LIMIT));
        let line = "KEY=canary-exact-0001-first and canary-exact-0002-second";
        assert!(matches!(registry.matcher(0).unwrap().small, Some(Matcher::Exact(_))), "the pattern is over its limit");
        assert!(!shows(&mut registry, 0, "canary-exact-0001-first") && !shows(&mut registry, 0, "canary-exact-0002-second"));
        assert!(registry.alerts.iter().any(|a| a.contains("degraded to exact")), "one signal, with the counts: {:?}", registry.alerts);

        // The exact search withholds what the pattern does, in every spelling.
        let exact = registry.matcher(0).unwrap();
        registry.limits = None;
        registry.stale = true;
        let pattern = registry.matcher(0).unwrap();
        assert!(matches!(pattern.small, Some(Matcher::Pattern(_))));
        let spelled = base64::engine::general_purpose::STANDARD.encode("canary-exact-0001-first");
        let line = format!("{line} {spelled}");
        assert_eq!(replaced(&exact, &line), replaced(&pattern, &line));
        assert!(!replaced(&exact, &line).contains("canary-exact"));
    }

    #[test]
    fn when_even_the_exact_search_is_over_its_limit_every_line_reads_as_the_mark() {
        let mut registry = Registry::default();
        registry.add(Some("acct:a"), "canary-closed-0001-value", None);
        registry.limits = Some((0, 1));
        assert!(matches!(registry.matcher(0).unwrap().small, Some(Matcher::Closed)));
        assert!(registry.alerts.iter().any(|a| a.contains("degraded to closed")), "{:?}", registry.alerts);
        let closed = Built { small: Some(Matcher::Closed), large: Arc::new(Vec::new()) };
        assert_eq!(replaced(&closed, "an ordinary line"), MARK, "never the text");
        assert_eq!(replaced(&closed, "KEY=canary-closed-0001-value"), MARK);
    }

    #[test]
    fn release_all_drops_every_value_an_owner_holds_and_no_other_owners() {
        let mut registry = Registry::default();
        registry.add(Some("acct:a#100"), "canary-all-0001-first", Some(100));
        registry.add(Some("acct:a#200"), "canary-all-0002-second", Some(200));
        registry.add(Some("acct:a"), "canary-all-0003-plain", None);
        registry.add(Some("acct:ab#200"), "canary-all-0004-other", Some(200));
        registry.release_all("acct:a");
        assert_eq!(registry.entries.len(), 1, "another owner whose name starts the same stays");
        assert!(!shows(&mut registry, 0, "canary-all-0004-other"));
    }

    #[test]
    fn a_batch_of_registrations_builds_the_matcher_once() {
        let mut registry = Registry::default();
        for i in 0..200 {
            registry.add(None, &format!("canary-batch-{i:04}-value"), None);
            registry.add(Some(&format!("owner-{i}")), &format!("canary-batch-held-{i:04}-value"), Some(100_000));
        }
        assert_eq!(registry.rebuilds, 0, "nothing is built until a line needs it");
        for _ in 0..5 {
            assert!(registry.matcher(0).is_some());
        }
        assert_eq!(registry.rebuilds, 1, "400 registrations and five lines cost one build");
        registry.add(None, "canary-batch-late-value", None);
        registry.matcher(0);
        assert_eq!(registry.rebuilds, 2, "a later change builds again, once");
        registry.matcher(0);
        assert_eq!(registry.rebuilds, 2, "and a registration that changes nothing builds nothing");
        registry.add(None, "canary-batch-late-value", None);
        registry.matcher(0);
        assert_eq!(registry.rebuilds, 2);
    }

    #[test]
    fn the_process_wide_registry_drops_an_owner_that_is_released() {
        let value = "canary-global-release-0001";
        hold("withhold-test:global-release", value, None);
        assert_eq!(text(&format!("k {value}")), format!("k {MARK}"));
        release("withhold-test:global-release");
        assert_eq!(text(&format!("k {value}")), format!("k {value}"));
    }

    #[test]
    fn the_process_wide_registry_drops_a_value_after_its_expiry() {
        let value = "canary-global-expiry-0001";
        let now = agentmux_common::time::now_ms();
        hold("withhold-test:global-expiry", value, Some(now + 600_000));
        assert_eq!(text(&format!("k {value}")), format!("k {MARK}"));
        hold("withhold-test:global-expiry", value, Some(now - 1));
        assert_eq!(text(&format!("k {value}")), format!("k {value}"), "a value held to a time already past is not withheld");
    }
}
