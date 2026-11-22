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
//! The registry is bounded and never fails open. A value over [`MAX_VALUE_LEN`] is refused, and the registry holds at
//! most [`MAX_TOTAL_BYTES`] (the oldest values an owner holds make room first); each refusal is counted and logged. A
//! matcher that cannot be built as one pattern is replaced by an exact search over the same values, and if even that
//! is over its limit every line is read as the mark ([`MARK`]) in place of its text. Either state is logged once with
//! the counts and reported to the hook ([`set_alert`]).

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

/// The longest value the registry takes (bytes): a longer one is refused. No real secret is this long, and one this
/// long could stop the matcher of every other value from building.
pub const MAX_VALUE_LEN: usize = 32 * 1024;

/// The most the registry holds (bytes of values, not of spellings): past it the oldest values an owner holds make room.
pub const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

/// The size limit of the one-pattern matcher, and of the exact search that replaces it (bytes of spellings).
const REGEX_LIMIT: usize = 64 << 20;
const EXACT_LIMIT: usize = 32 << 20;

/// How many values were refused, and how many times the one-pattern matcher could not be built.
static REFUSED: AtomicU64 = AtomicU64::new(0);
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

/// The state of the registry, for a health read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// `none` (nothing registered), `pattern`, `exact` or `closed`.
    pub mode: &'static str,
    pub values: usize,
    pub bytes: usize,
    /// Values refused since the process started.
    pub refused: u64,
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
    matcher: Option<Matcher>,
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

    /// Whether a value of `len` bytes can be taken at all.
    fn takes(&mut self, len: usize) -> bool {
        if len > MAX_VALUE_LEN {
            self.refuse("over the longest value", len);
            return false;
        }
        true
    }

    /// Makes the registry fit [`MAX_TOTAL_BYTES`] after `newest` was pushed: the oldest values an owner holds go first.
    /// `newest` itself goes (and is refused) when nothing else can make room.
    fn fit(&mut self) {
        while self.bytes() > MAX_TOTAL_BYTES {
            let last = self.entries.len().saturating_sub(1);
            match self.entries[..last].iter().position(|e| e.owner.is_some()) {
                Some(oldest) => {
                    self.entries.remove(oldest);
                }
                None => {
                    let len = self.entries.pop().map_or(0, |e| e.value.len());
                    self.refuse("the registry is full", len);
                    break;
                }
            }
        }
    }

    fn reindex(&mut self) {
        self.next_expiry = self.entries.iter().filter_map(|e| e.expires_at).min();
    }

    /// Keeps `value` for the process (`owner` `None`) or for `owner`, in place of what the owner held. A value
    /// shorter than [`MIN_LEN`] is not kept, so an owner given one holds nothing.
    fn add(&mut self, owner: Option<&str>, value: &str, expires_at: Option<i64>) {
        let value = value.trim();
        let keep = value.len() >= MIN_LEN && self.takes(value.len());
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
        if owner.is_some() {
            let before = self.entries.len();
            self.entries.retain(|e| !mine(e));
            self.stale |= self.entries.len() != before;
        }
        if keep {
            self.entries.push(Entry { owner: owner.map(str::to_string), value: value.to_string(), expires_at });
            self.fit();
            self.stale = true;
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
        if self.entries.iter().filter(|e| mine(e)).count() > KEPT_PER_OWNER {
            if let Some(oldest) = self.entries.iter().position(|e| mine(e)) {
                self.entries.remove(oldest);
            }
        }
        self.fit();
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
    fn matcher(&mut self, now: i64) -> Option<Matcher> {
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

fn mode_of(matcher: Option<&Matcher>) -> &'static str {
    match matcher {
        None => "none",
        Some(Matcher::Pattern(_)) => "pattern",
        Some(Matcher::Exact(_)) => "exact",
        Some(Matcher::Closed) => "closed",
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
        bytes: registry.bytes(),
        refused: REFUSED.load(Ordering::Relaxed),
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

/// One matcher for every value in every spelling, longest first so a spelling that contains another is replaced
/// whole. `None` when nothing is registered. When one pattern cannot be built under `regex_limit` the same spellings
/// are searched one at a time; when they are over `exact_limit` bytes the matcher is [`Matcher::Closed`].
fn build(values: &[&str], regex_limit: usize, exact_limit: usize) -> Option<Matcher> {
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

fn matcher() -> Option<Matcher> {
    let now = agentmux_common::time::now_ms();
    {
        let registry = state().read().unwrap_or_else(|e| e.into_inner());
        if registry.current(now) {
            return registry.matcher.clone();
        }
    }
    with_write(|r| r.matcher(now))
}

/// `text` with every value `matcher` knows replaced by [`MARK`].
fn replaced<'a>(matcher: &Matcher, text: &'a str) -> Cow<'a, str> {
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

/// `text` with every registered value replaced by [`MARK`]; the text itself when it holds none. Never the text of a
/// line when matching is closed: then every line is the mark.
pub fn text(text: &str) -> Cow<'_, str> {
    match matcher() {
        Some(matcher) => replaced(&matcher, text),
        None => Cow::Borrowed(text),
    }
}

/// Whether `text` holds a registered value, in any of its spellings: the same matcher [`text`] replaces with. Always
/// true when matching is closed.
pub fn holds(text: &str) -> bool {
    match matcher() {
        Some(Matcher::Pattern(regex)) => regex.is_match(text),
        Some(Matcher::Exact(needles)) => needles.iter().any(|n| text.contains(n.as_str())),
        Some(Matcher::Closed) => true,
        None => false,
    }
}

/// [`text`] over transcript bytes (NDJSON lines); bytes that are not UTF-8 are left as they are, except when matching is
/// closed: then every line is the mark.
pub fn lines(data: &[u8]) -> Cow<'_, [u8]> {
    if matches!(matcher(), Some(Matcher::Closed)) {
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
    fn the_registry_holds_a_bounded_total_and_the_oldest_owned_values_make_room() {
        let mut registry = Registry::default();
        let each = "y".repeat(MAX_VALUE_LEN - 64);
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
    fn a_matcher_that_cannot_be_built_as_one_pattern_is_replaced_by_an_exact_search() {
        let mut registry = Registry::default();
        registry.add(Some("acct:a"), "canary-exact-0001-first", None);
        registry.add(Some("acct:b"), "canary-exact-0002-second", None);
        registry.limits = Some((0, EXACT_LIMIT));
        let line = "KEY=canary-exact-0001-first and canary-exact-0002-second";
        assert!(matches!(registry.matcher(0), Some(Matcher::Exact(_))), "the pattern is over its limit");
        assert!(!shows(&mut registry, 0, "canary-exact-0001-first") && !shows(&mut registry, 0, "canary-exact-0002-second"));
        assert!(registry.alerts.iter().any(|a| a.contains("degraded to exact")), "one signal, with the counts: {:?}", registry.alerts);

        // The exact search withholds what the pattern does, in every spelling.
        let exact = registry.matcher(0).unwrap();
        registry.limits = None;
        registry.stale = true;
        let pattern = registry.matcher(0).unwrap();
        assert!(matches!(pattern, Matcher::Pattern(_)));
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
        assert!(matches!(registry.matcher(0), Some(Matcher::Closed)));
        assert!(registry.alerts.iter().any(|a| a.contains("degraded to closed")), "{:?}", registry.alerts);
        let closed = Matcher::Closed;
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
