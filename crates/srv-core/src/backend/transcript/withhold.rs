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

use std::borrow::Cow;
use std::sync::{Arc, OnceLock, RwLock};

use regex::{NoExpand, Regex, RegexBuilder};

/// What a withheld value is replaced by.
pub const MARK: &str = "[withheld]";

/// Shorter values are not registered: matching them would mangle ordinary text.
pub const MIN_LEN: usize = 8;

/// How many values with no known expiry an owner keeps with [`hold_kept`].
pub const KEPT_PER_OWNER: usize = 8;

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
    matcher: Option<Arc<Regex>>,
    /// The values changed since `matcher` was built.
    stale: bool,
    /// The earliest expiry among the entries.
    next_expiry: Option<i64>,
    /// How many times the matcher has been built.
    rebuilds: usize,
}

impl Registry {
    fn reindex(&mut self) {
        self.next_expiry = self.entries.iter().filter_map(|e| e.expires_at).min();
    }

    /// Keeps `value` for the process (`owner` `None`) or for `owner`, in place of what the owner held. A value
    /// shorter than [`MIN_LEN`] is not kept, so an owner given one holds nothing.
    fn add(&mut self, owner: Option<&str>, value: &str, expires_at: Option<i64>) {
        let value = value.trim();
        let keep = value.len() >= MIN_LEN;
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
            self.stale = true;
        }
        self.reindex();
    }

    /// Keeps `value` for `owner` along with what the owner kept before, up to [`KEPT_PER_OWNER`] (the oldest is dropped).
    fn add_kept(&mut self, owner: &str, value: &str) {
        let value = value.trim();
        let mine = |e: &Entry| e.owner.as_deref() == Some(owner);
        if value.len() < MIN_LEN || self.entries.iter().any(|e| mine(e) && e.value == value) {
            return;
        }
        self.entries.push(Entry { owner: Some(owner.to_string()), value: value.to_string(), expires_at: None });
        if self.entries.iter().filter(|e| mine(e)).count() > KEPT_PER_OWNER {
            if let Some(oldest) = self.entries.iter().position(|e| mine(e)) {
                self.entries.remove(oldest);
            }
        }
        self.stale = true;
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
    fn matcher(&mut self, now: i64) -> Option<Arc<Regex>> {
        self.expire(now);
        if self.stale {
            let values: Vec<&str> = self.entries.iter().map(|e| e.value.as_str()).collect();
            self.matcher = build(&values);
            self.rebuilds += 1;
            self.stale = false;
        }
        self.matcher.clone()
    }
}

fn state() -> &'static RwLock<Registry> {
    static STATE: OnceLock<RwLock<Registry>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

fn write() -> std::sync::RwLockWriteGuard<'static, Registry> {
    state().write().unwrap_or_else(|e| e.into_inner())
}

/// Registers `value` to be withheld from now on, for the life of the process. A value shorter than [`MIN_LEN`] and one
/// already registered change nothing.
pub fn register(value: &str) {
    write().add(None, value, None);
}

/// [`register`] for each of `values`, under one lock: the matcher is built once for the lot, when a line next needs it.
pub fn register_all<'a>(values: impl IntoIterator<Item = &'a str>) {
    let mut registry = write();
    for value in values {
        registry.add(None, value, None);
    }
}

/// Withholds `value` for `owner`, in place of what the owner held before, until `expires_at` (unix ms; `None` for no
/// expiry) or until the owner is [`release`]d. Holding the value the owner already holds changes nothing.
pub fn hold(owner: &str, value: &str, expires_at: Option<i64>) {
    write().add(Some(owner), value, expires_at);
}

/// Withholds `value` for `owner` until `expires_at` (unix ms), when it stops working. A renewed value the owner holds
/// next with [`hold_until`] does not take its place: the value that was replaced still works until its own expiry,
/// so it stays withheld until then. Holding the same value and expiry again changes nothing.
pub fn hold_until(owner: &str, value: &str, expires_at: i64) {
    write().add(Some(&format!("{owner}#{expires_at}")), value, Some(expires_at));
}

/// Withholds `value` for `owner` with no expiry, until the process ends or the owner is [`release`]d. The values the
/// owner kept before it stay withheld too, up to [`KEPT_PER_OWNER`]: past that the oldest is dropped. Keeping the same
/// value again changes nothing.
pub fn hold_kept(owner: &str, value: &str) {
    write().add_kept(owner, value);
}

/// Stops withholding what `owner` holds: its value no longer works.
pub fn release(owner: &str) {
    write().release(owner);
}

/// [`release_all`] for every owner whose name starts with `prefix`.
pub fn release_prefix(prefix: &str) {
    write().release_prefix(prefix);
}

/// [`release`] for what `owner` holds and what it holds with [`hold_until`].
pub fn release_all(owner: &str) {
    write().release_all(owner);
}

/// One matcher for every value in every spelling, longest first so a spelling that contains another is
/// replaced whole. `None` when nothing is registered, or when the matcher could not be built.
fn build(values: &[&str]) -> Option<Arc<Regex>> {
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
    match RegexBuilder::new(&pattern).size_limit(64 << 20).build() {
        Ok(matcher) => Some(Arc::new(matcher)),
        Err(e) => {
            tracing::warn!(error = %e, values = values.len(), "transcript withhold: the matcher could not be built");
            None
        }
    }
}

fn matcher() -> Option<Arc<Regex>> {
    let now = agentmux_common::time::now_ms();
    {
        let registry = state().read().unwrap_or_else(|e| e.into_inner());
        if registry.current(now) {
            return registry.matcher.clone();
        }
    }
    write().matcher(now)
}

/// `text` with every registered value replaced by [`MARK`]; the text itself when it holds none.
pub fn text(text: &str) -> Cow<'_, str> {
    match matcher() {
        Some(matcher) => matcher.replace_all(text, NoExpand(MARK)),
        None => Cow::Borrowed(text),
    }
}

/// Whether `text` holds a registered value, in any of its spellings: the same matcher [`text`] replaces with.
pub fn holds(text: &str) -> bool {
    matcher().is_some_and(|matcher| matcher.is_match(text))
}

/// [`text`] over transcript bytes (NDJSON lines); bytes that are not UTF-8 are left as they are.
pub fn lines(data: &[u8]) -> Cow<'_, [u8]> {
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
        registry.matcher(now).map_or(line.clone(), |m| m.replace_all(&line, NoExpand(MARK)).into_owned()).contains(value)
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
