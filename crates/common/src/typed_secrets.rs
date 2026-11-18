// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! What was typed into a page from the Vault, kept as a keyed digest so a read can leave it out wherever it shows
//! (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md, G4).
//!
//! A value typed from the Vault is never returned to an agent. The mark on the field it was typed into is one way a
//! read knows; this is the other: for every document a value was typed into, a read of that document's text or of a
//! field's value is checked against the values typed there, wherever they now sit (a field the page replaced, a text it
//! echoed, a longer sentence that holds it).
//!
//! What is kept is, for each value, its length in characters, a rolling fingerprint and an HMAC-SHA-256 of it, both
//! keyed with a key made when the process started, held in memory only and never written anywhere. The value itself is
//! dropped after it is typed. A short value (a PIN) can't be guessed from the digest without the key. A window of that
//! length is slid over the text read; a window whose fingerprint matches is confirmed with the HMAC.
//!
//! A value shorter than [`WINDOW_FROM`] characters is matched as a whole text only (a field that holds exactly it), not
//! inside a longer one, so a one-digit code box doesn't blank every `5` on the page.
//!
//! The digests belong to one document of one pane. A read made against another document (the pane navigated, reloaded
//! or was closed, so its document id differs) finds none, and they are dropped then.
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// What a read shows in place of text that holds a value typed from the Vault.
pub const TYPED_FROM_VAULT: &str = "[typed from the Vault: hidden]";

/// From this many characters a value is matched inside longer text; below it, as a whole text only.
pub const WINDOW_FROM: usize = 6;
/// The most values kept for one document.
const MAX_PER_DOC: usize = 64;
/// The prime of the rolling fingerprint's field (2^61 - 1).
const P: u64 = (1 << 61) - 1;

type Mac256 = Hmac<Sha256>;

struct Typed {
    chars: usize,
    fingerprint: u64,
    mac: [u8; 32],
}

struct Doc {
    /// The document, or `None` when it could not be read as the value was typed: the first read that can tell takes it.
    id: Option<String>,
    typed: Vec<Typed>,
}

impl Doc {
    /// Whether a read of `doc` is of this document: one that can't tell which it is counts as it is, and a document
    /// that wasn't known is the first one a read names.
    fn is_document(&mut self, doc: Option<&str>) -> bool {
        match (&self.id, doc) {
            (_, None) => true,
            (None, Some(id)) => {
                self.id = Some(id.to_string());
                true
            }
            (Some(have), Some(id)) => have == id,
        }
    }
}

/// The digests of pane `block`'s document when a read of `doc` is of it; another document is a page with nothing typed
/// in it, and the digests kept are dropped.
fn of_document<'a>(docs: &'a mut HashMap<String, Doc>, block: &str, doc: Option<&str>) -> Option<&'a Doc> {
    if !docs.get_mut(block)?.is_document(doc) {
        docs.remove(block);
        return None;
    }
    docs.get(block)
}

/// The values typed into the documents of this process's panes, as digests.
pub struct TypedSecrets {
    key: [u8; 32],
    base: u64,
    docs: Mutex<HashMap<String, Doc>>,
}

impl fmt::Debug for TypedSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedSecrets").finish_non_exhaustive()
    }
}

fn mul_mod(a: u64, b: u64) -> u64 {
    ((a as u128 * b as u128) % P as u128) as u64
}

impl TypedSecrets {
    /// Digests keyed with `key`: random, made when the process started, and kept nowhere else.
    pub fn new(key: [u8; 32]) -> Self {
        let mut mac = Mac256::new_from_slice(&key).expect("an HMAC takes a key of any length");
        mac.update(b"fingerprint base");
        let bytes = mac.finalize().into_bytes();
        let base = u64::from_be_bytes(bytes[..8].try_into().expect("eight bytes")) % (P - 1000) + 500;
        Self { key, base, docs: Mutex::new(HashMap::new()) }
    }

    fn fingerprint(&self, text: &[char]) -> u64 {
        text.iter().fold(0u64, |h, c| (mul_mod(h, self.base) + *c as u64 + 1) % P)
    }

    fn mac(&self, text: &str) -> [u8; 32] {
        let mut mac = Mac256::new_from_slice(&self.key).expect("an HMAC takes a key of any length");
        mac.update(text.as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// `value` was typed into the document `doc` of pane `block` (`None`: the document could not be read, and the first
    /// read that can tell is taken for it). A different `doc` than the one kept for the pane replaces it: the earlier
    /// document is gone.
    pub fn note<'a>(&self, block: &str, doc: impl Into<Option<&'a str>>, value: &str) {
        let doc: Option<&str> = doc.into();
        let text: Vec<char> = value.chars().collect();
        if text.is_empty() {
            return;
        }
        let typed = Typed { chars: text.len(), fingerprint: self.fingerprint(&text), mac: self.mac(value) };
        let mut docs = self.docs.lock().unwrap_or_else(|p| p.into_inner());
        let entry = docs.entry(block.to_string()).or_insert_with(|| Doc { id: doc.map(str::to_string), typed: Vec::new() });
        if !entry.is_document(doc) {
            *entry = Doc { id: doc.map(str::to_string), typed: Vec::new() };
        }
        if entry.typed.iter().any(|t| t.mac == typed.mac) {
            return;
        }
        if entry.typed.len() >= MAX_PER_DOC {
            entry.typed.remove(0);
        }
        entry.typed.push(typed);
    }

    /// The pane is closed, or its document is gone.
    pub fn forget(&self, block: &str) {
        self.docs.lock().unwrap_or_else(|p| p.into_inner()).remove(block);
    }

    /// Whether a value was typed into the document `doc` of pane `block` (and it is still that document). A `doc` of
    /// `None` is a read that could not tell which document it is: it counts as the document the values were typed in.
    pub fn typed_into(&self, block: &str, doc: Option<&str>) -> bool {
        let mut docs = self.docs.lock().unwrap_or_else(|p| p.into_inner());
        of_document(&mut docs, block, doc).is_some_and(|d| !d.typed.is_empty())
    }

    /// `text` with every part that is a value typed into the document replaced by [`TYPED_FROM_VAULT`], or `None`
    /// when nothing in it is one.
    pub fn withhold(&self, block: &str, doc: Option<&str>, text: &str) -> Option<String> {
        let typed: Vec<(usize, u64, [u8; 32])> = {
            let mut docs = self.docs.lock().unwrap_or_else(|p| p.into_inner());
            let Some(d) = of_document(&mut docs, block, doc) else { return None };
            d.typed.iter().map(|t| (t.chars, t.fingerprint, t.mac)).collect()
        };
        if typed.is_empty() || text.is_empty() {
            return None;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut hidden = vec![false; chars.len()];
        let mut any = false;
        let mut lengths: Vec<usize> = typed.iter().map(|t| t.0).collect();
        lengths.sort_unstable();
        lengths.dedup();
        for len in lengths {
            let wanted: Vec<&(usize, u64, [u8; 32])> = typed.iter().filter(|t| t.0 == len).collect();
            if len < WINDOW_FROM {
                // The whole text, trimmed, is the value.
                let (start, end) = trimmed_range(&chars);
                if end - start == len && self.confirm(&chars[start..end], &wanted) {
                    hidden[start..end].fill(true);
                    any = true;
                }
                continue;
            }
            if chars.len() < len {
                continue;
            }
            // pow = base^(len - 1), to take the oldest character out of the rolling fingerprint.
            let pow = (1..len).fold(1u64, |p, _| mul_mod(p, self.base));
            let mut h = self.fingerprint(&chars[..len]);
            let mut i = 0;
            loop {
                if wanted.iter().any(|t| t.1 == h) && self.confirm(&chars[i..i + len], &wanted) {
                    hidden[i..i + len].fill(true);
                    any = true;
                }
                if i + len >= chars.len() {
                    break;
                }
                let out = mul_mod((chars[i] as u64 + 1) % P, pow);
                h = (mul_mod((h + P - out) % P, self.base) + chars[i + len] as u64 + 1) % P;
                i += 1;
            }
        }
        if !any {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut in_run = false;
        for (c, h) in chars.iter().zip(&hidden) {
            if *h {
                if !in_run {
                    out.push_str(TYPED_FROM_VAULT);
                    in_run = true;
                }
            } else {
                in_run = false;
                out.push(*c);
            }
        }
        Some(out)
    }

    /// Every string in `value` (a read's whole answer: a snapshot, a find, a read-back) with the parts that are a
    /// value typed into the document replaced by [`TYPED_FROM_VAULT`]. How many strings were changed.
    pub fn withhold_json(&self, block: &str, doc: Option<&str>, value: &mut serde_json::Value) -> usize {
        use serde_json::Value;
        match value {
            Value::String(s) => match self.withhold(block, doc, s) {
                Some(shown) => {
                    *s = shown;
                    1
                }
                None => 0,
            },
            Value::Array(items) => items.iter_mut().map(|v| self.withhold_json(block, doc, v)).sum(),
            Value::Object(fields) => fields.values_mut().map(|v| self.withhold_json(block, doc, v)).sum(),
            _ => 0,
        }
    }

    fn confirm(&self, window: &[char], wanted: &[&(usize, u64, [u8; 32])]) -> bool {
        let text: String = window.iter().collect();
        let mac = self.mac(&text);
        wanted.iter().any(|t| crate::secret_eq::secret_eq(&t.2, &mac))
    }
}

/// The range of `chars` without the whitespace at either end.
fn trimmed_range(chars: &[char]) -> (usize, usize) {
    let start = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(chars.len());
    let end = chars.iter().rposition(|c| !c.is_whitespace()).map_or(start, |e| e + 1);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "Tr0ub4dor&3-sentinel";

    fn secrets() -> TypedSecrets {
        TypedSecrets::new([7u8; 32])
    }

    #[test]
    fn a_typed_value_is_hidden_wherever_it_sits_in_a_text() {
        let s = secrets();
        s.note("pane-1", "doc-1", PASSWORD);
        assert_eq!(s.withhold("pane-1", Some("doc-1"), PASSWORD).as_deref(), Some(TYPED_FROM_VAULT));
        let said = s.withhold("pane-1", Some("doc-1"), &format!("your password is {PASSWORD}, keep it")).unwrap();
        assert_eq!(said, format!("your password is {TYPED_FROM_VAULT}, keep it"));
        // A field that holds the value and a space, or twice, or in another place.
        assert_eq!(s.withhold("pane-1", Some("doc-1"), &format!("{PASSWORD} ")).unwrap(), format!("{TYPED_FROM_VAULT} "));
        assert_eq!(s.withhold("pane-1", Some("doc-1"), &format!("{PASSWORD}/{PASSWORD}")).unwrap(), format!("{TYPED_FROM_VAULT}/{TYPED_FROM_VAULT}"));
        assert_eq!(s.withhold("pane-1", Some("doc-1"), &format!("a{PASSWORD}")).unwrap(), format!("a{TYPED_FROM_VAULT}"));
    }

    #[test]
    fn text_that_is_not_the_value_is_left_alone() {
        let s = secrets();
        s.note("pane-1", "doc-1", PASSWORD);
        for text in ["", "hello", "Tr0ub4dor&3-sentinel".trim_end_matches('l'), "tr0ub4dor&3-sentinel", "Welcome back, alex"] {
            assert_eq!(s.withhold("pane-1", Some("doc-1"), text), None, "{text:?}");
        }
    }

    #[test]
    fn a_short_value_is_matched_as_a_whole_text_only() {
        let s = secrets();
        s.note("pane-1", "doc-1", "4821");
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "4821").as_deref(), Some(TYPED_FROM_VAULT));
        assert_eq!(s.withhold("pane-1", Some("doc-1"), " 4821 ").as_deref(), Some(format!(" {TYPED_FROM_VAULT} ").as_str()));
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "order 4821 shipped"), None, "inside a longer text a short value is not matched");
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "48210"), None);
        s.note("pane-1", "doc-1", "5");
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "5").as_deref(), Some(TYPED_FROM_VAULT), "a one-digit code box");
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "5 items in the cart"), None);
    }

    #[test]
    fn a_value_with_other_letters_and_languages_is_matched_by_its_characters() {
        let s = secrets();
        s.note("pane-1", "doc-1", "pässwörd-日本語-🔑");
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "x pässwörd-日本語-🔑 y").unwrap(), format!("x {TYPED_FROM_VAULT} y"));
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "x passwörd-日本語-🔑 y"), None);
    }

    #[test]
    fn two_values_and_two_panes_are_kept_apart() {
        let s = secrets();
        s.note("pane-1", "doc-1", PASSWORD);
        s.note("pane-1", "doc-1", "another-typed-value");
        s.note("pane-2", "doc-9", "pane-two-value-77");
        let both = s.withhold("pane-1", Some("doc-1"), &format!("{PASSWORD} and another-typed-value")).unwrap();
        assert_eq!(both, format!("{TYPED_FROM_VAULT} and {TYPED_FROM_VAULT}"));
        assert_eq!(s.withhold("pane-1", Some("doc-1"), "pane-two-value-77"), None, "another pane's value is not this pane's");
        assert!(s.withhold("pane-2", Some("doc-9"), "pane-two-value-77").is_some());
    }

    #[test]
    fn a_new_document_is_a_page_with_nothing_typed_in_it() {
        let s = secrets();
        s.note("pane-1", "doc-1", PASSWORD);
        assert!(s.typed_into("pane-1", Some("doc-1")));
        assert!(s.typed_into("pane-1", None), "a read that can't tell which document it is counts as the one typed in");
        // The pane navigated: its document is another one, and the digests are dropped.
        assert!(!s.typed_into("pane-1", Some("doc-2")));
        assert_eq!(s.withhold("pane-1", Some("doc-2"), PASSWORD), None);
        assert!(!s.typed_into("pane-1", Some("doc-1")), "and they do not come back");
        // A value typed into the new document is its own.
        s.note("pane-1", "doc-2", "second-document-value");
        assert!(s.withhold("pane-1", Some("doc-2"), "second-document-value").is_some());
        s.note("pane-1", "doc-3", "third-document-value");
        assert_eq!(s.withhold("pane-1", Some("doc-3"), "second-document-value"), None);
        s.forget("pane-1");
        assert!(!s.typed_into("pane-1", None), "a closed pane keeps nothing");
    }

    #[test]
    fn a_value_typed_while_the_document_could_not_be_read_is_kept_for_the_first_one_a_read_names() {
        let s = secrets();
        s.note("pane-1", None, PASSWORD);
        assert!(s.typed_into("pane-1", None), "eval stays refused");
        assert!(s.withhold("pane-1", None, PASSWORD).is_some());
        // The first read that can tell which document it is takes it; a later document is another one.
        assert!(s.typed_into("pane-1", Some("doc-1")));
        assert!(s.withhold("pane-1", Some("doc-1"), PASSWORD).is_some());
        assert!(!s.typed_into("pane-1", Some("doc-2")));
        assert_eq!(s.withhold("pane-1", Some("doc-2"), PASSWORD), None);
    }

    #[test]
    fn what_is_kept_is_a_keyed_digest_and_not_the_value() {
        let one = TypedSecrets::new([1u8; 32]);
        let other = TypedSecrets::new([2u8; 32]);
        one.note("pane-1", "doc-1", PASSWORD);
        other.note("pane-1", "doc-1", PASSWORD);
        assert!(!format!("{one:?}").contains(PASSWORD) && !format!("{one:?}").contains("sentinel"));
        let digest = |s: &TypedSecrets| s.docs.lock().unwrap().get("pane-1").unwrap().typed.iter().map(|t| (t.fingerprint, t.mac)).collect::<Vec<_>>();
        assert_ne!(digest(&one), digest(&other), "another key gives another digest, so a short value can't be guessed without it");
        let kept = one.docs.lock().unwrap();
        let t = &kept.get("pane-1").unwrap().typed[0];
        assert_eq!(t.chars, PASSWORD.chars().count(), "the length is kept beside it");
        assert!(!t.mac.windows(8).any(|w| PASSWORD.as_bytes().windows(8).any(|v| v == w)));
    }

    #[test]
    fn typing_the_same_value_again_keeps_one_digest_and_the_most_per_document_is_bounded() {
        let s = secrets();
        for _ in 0..3 {
            s.note("pane-1", "doc-1", PASSWORD);
        }
        assert_eq!(s.docs.lock().unwrap().get("pane-1").unwrap().typed.len(), 1);
        for n in 0..(MAX_PER_DOC + 10) {
            s.note("pane-1", "doc-1", &format!("value-number-{n:04}"));
        }
        assert_eq!(s.docs.lock().unwrap().get("pane-1").unwrap().typed.len(), MAX_PER_DOC);
        assert!(s.withhold("pane-1", Some("doc-1"), &format!("value-number-{:04}", MAX_PER_DOC + 9)).is_some(), "the newest are kept");
    }

    #[test]
    fn the_rolling_fingerprint_agrees_with_the_one_made_whole_on_a_long_text() {
        let s = secrets();
        s.note("pane-1", "doc-1", "needle-in-a-haystack");
        let text = format!("{}needle-in-a-haystack{}", "hay ".repeat(5000), " straw".repeat(3000));
        let said = s.withhold("pane-1", Some("doc-1"), &text).unwrap();
        assert_eq!(said.matches(TYPED_FROM_VAULT).count(), 1);
        assert!(!said.contains("needle"));
        assert_eq!(said.len(), text.len() - "needle-in-a-haystack".len() + TYPED_FROM_VAULT.len());
    }

    /// A page as the agent's reads see it: the fields, with the facts a snapshot lists, and a script of the page's
    /// own that can change them. No field here holds the Vault's mark after the page's script has run.
    struct FakePage {
        fields: Vec<(String, String, String)>, // name, type, value
        marked: Vec<String>,
    }

    impl FakePage {
        fn type_into(&mut self, secrets: &TypedSecrets, name: &str, value: &str) {
            let field = self.fields.iter_mut().find(|f| f.0 == name).unwrap();
            field.2 = value.to_string();
            self.marked.push(name.to_string());
            secrets.note("pane-1", "doc-1", value);
        }

        /// What the page's own script does: a type switch that shows the password, the mark taken off, and a copy
        /// of the value into a heading and a status line.
        fn page_script_runs(&mut self) {
            self.marked.clear();
            for f in self.fields.iter_mut() {
                f.1 = "text".to_string();
            }
            let shown = self.fields.iter().find(|f| f.0 == "password").unwrap().2.clone();
            self.fields.push(("heading".into(), "text".into(), format!("Welcome, your password is {shown}")));
            self.fields.push(("status".into(), "text".into(), shown));
        }

        /// The answers a snapshot, a find and a read-back give: every field with its type, mark and value.
        fn reads(&self) -> serde_json::Value {
            let list = |_: &str| -> Vec<serde_json::Value> {
                self.fields
                    .iter()
                    .map(|(name, kind, value)| {
                        let hidden = self.marked.contains(name);
                        serde_json::json!({ "name": name, "type": kind, "marked": hidden, "value": if hidden { "[hidden]" } else { value.as_str() } })
                    })
                    .collect()
            };
            serde_json::json!({ "snapshot": list("snapshot"), "find": { "matches": list("find") }, "readBack": { "after": { "value": self.fields[0].2 } } })
        }
    }

    #[test]
    fn g4_web_no_agent_script_runs_and_no_call_returns_the_field_value_even_after_a_type_switch() {
        let s = secrets();
        let mut page = FakePage {
            fields: vec![("user".into(), "text".into(), String::new()), ("password".into(), "password".into(), String::new())],
            marked: Vec::new(),
        };
        page.type_into(&s, "password", PASSWORD);
        // A script an agent runs is refused for the document, whatever the page does to the mark.
        assert!(s.typed_into("pane-1", Some("doc-1")), "eval is refused while a Vault value was typed into the document");
        page.page_script_runs();
        assert!(page.marked.is_empty(), "the page took the mark off");
        assert!(s.typed_into("pane-1", Some("doc-1")), "and the refusal does not depend on it");
        let mut answer = page.reads();
        assert!(answer.to_string().contains(PASSWORD), "the page's own copy is in what it would read back");
        let changed = s.withhold_json("pane-1", Some("doc-1"), &mut answer);
        assert!(changed >= 4, "the field, the heading, the status and the read-back: {changed}");
        assert!(!answer.to_string().contains(PASSWORD) && !answer.to_string().contains("sentinel"), "{answer}");
        assert!(answer.to_string().contains(TYPED_FROM_VAULT));
        assert_eq!(answer["snapshot"][0]["name"], "user", "what is not the value is read as it was");
        // The page is another document once it navigates: nothing typed there is kept, and eval is allowed again.
        assert!(!s.typed_into("pane-1", Some("doc-2")));
    }
}
