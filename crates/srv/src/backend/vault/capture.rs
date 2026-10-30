// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Terminal` `run`'s `capture`: a value a command prints goes straight into the
//! Vault (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.2).
//!
//! srv holds the command's output until it ends ([`crate::backend::shell_node`]),
//! then [`Capture::finish`] applies the rules to its standard output, stores each
//! match as a new entry or as a new value of an existing one (the old value kept 7
//! days, §4.4), and returns a [`Scrubber`] for the values, so the output is
//! published with `[vault:<id>]` where each was. If a rule matches nothing, srv
//! can't tell which line held the secret: the output is withheld whole, and kept
//! as a note the person can open with Reveal.

use std::collections::{BTreeMap, HashMap};

use agentmux_common::api_types::{CaptureFrom, CaptureRule};
use agentmux_common::time::now_ms;
use serde_json::value::RawValue;
use zeroize::Zeroizing;

use super::scrub::Scrubber;
use super::{insert, kinds, new_id, record, seal, validate, versions, NewEntry, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::storage::vault_versions::CaptureWrite;

/// Most rules one command may have.
pub const MOST: usize = 8;
/// A shorter value is refused: the output couldn't be cleaned of it reliably.
pub const MIN_LEN: usize = 8;
/// The most output held for a capture; past it the capture fails and the
/// output kept for the person is cut here.
pub const MAX_HELD: usize = 1024 * 1024;
/// The longest regex, JSON path or label a rule may give.
const MAX_RULE: usize = 500;
/// The action an entry's activity log names a capture by.
pub const CAPTURED: &str = "captured";

/// Where one rule's value goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    New { kind: &'static str, label: String, part: &'static str },
    /// An existing entry, as it was when the command was approved: a value is
    /// never stored into an entry deleted and made again since.
    Existing { id: String, created_at: i64, label: String, part: &'static str },
}

/// Where a rule finds its value in standard output.
#[derive(Debug, Clone)]
pub enum Source {
    All,
    Json(Vec<Step>),
    Regex(regex::Regex),
    Line(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub into: Target,
    pub from: Source,
}

/// A command's capture, checked before it runs.
#[derive(Debug, Clone)]
pub struct Capture {
    pub rules: Vec<Rule>,
    /// The agent that ran the command, as its verified identity names it.
    pub agent: String,
    /// The command line, for the activity log and the kept output's label.
    pub cmd: String,
}

/// A capture as the command's runner holds it: the rules, the store they write
/// to, and a call that tells every Vault pane its entries changed.
pub struct Job {
    pub capture: Capture,
    pub store: std::sync::Arc<Store>,
    pub changed: Box<dyn Fn() + Send + Sync>,
}

/// What a capture came to, once the command ended.
pub enum Outcome {
    /// Every rule matched: the entries, in rule order, and the values to take
    /// out of the output.
    Stored { ids: Vec<String>, scrubber: Scrubber },
    /// The output is withheld: why, and the note the person can open it in.
    Withheld { why: String, kept_as: Option<String> },
}

fn clip(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

/// `$.a.b[0]['c d']` as steps. `$` alone is the whole document.
pub fn json_path(path: &str) -> Result<Vec<Step>, String> {
    let bad = |why: &str| format!("the JSON path {:?} {why}; write it like $.AccessKey.SecretAccessKey or $.items[0].token", clip(path, 60));
    let rest = path.trim().strip_prefix('$').ok_or_else(|| bad("doesn't start with $"))?;
    let mut steps = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                let mut key = String::new();
                while let Some(&n) = chars.peek() {
                    if n == '.' || n == '[' {
                        break;
                    }
                    key.push(n);
                    chars.next();
                }
                if key.is_empty() {
                    return Err(bad("has an empty name"));
                }
                steps.push(Step::Key(key));
            }
            '[' => {
                let quote = chars.peek().copied().filter(|q| *q == '\'' || *q == '"');
                let mut inner = String::new();
                if let Some(q) = quote {
                    chars.next();
                    loop {
                        match chars.next() {
                            Some(c) if c == q && chars.peek() == Some(&']') => break,
                            Some(c) => inner.push(c),
                            None => return Err(bad("has an unclosed quote")),
                        }
                    }
                    chars.next();
                    steps.push(Step::Key(inner));
                } else {
                    loop {
                        match chars.next() {
                            Some(']') => break,
                            Some(c) => inner.push(c),
                            None => return Err(bad("has an unclosed [")),
                        }
                    }
                    steps.push(Step::Index(inner.trim().parse().map_err(|_| bad("has an index that isn't a number"))?));
                }
            }
            _ => return Err(bad("has a step that isn't .name or [n]")),
        }
    }
    Ok(steps)
}

fn rule_from(from: &CaptureFrom) -> Result<Source, String> {
    match from {
        CaptureFrom::Stdout(s) if s == "all" => Ok(Source::All),
        CaptureFrom::Stdout(s) => Err(format!("\"stdout\" takes \"all\", not {:?}", clip(s, 40))),
        CaptureFrom::Json(p) if p.len() > MAX_RULE => Err(format!("a JSON path is at most {MAX_RULE} characters")),
        CaptureFrom::Json(p) => Ok(Source::Json(json_path(p)?)),
        CaptureFrom::Regex(r) if r.len() > MAX_RULE => Err(format!("a regex is at most {MAX_RULE} characters")),
        CaptureFrom::Regex(r) => {
            let re = regex::RegexBuilder::new(r).size_limit(1 << 20).build().map_err(|e| format!("the regex doesn't compile: {e}"))?;
            if re.captures_len() != 2 {
                return Err(format!("the regex needs exactly one capture group, the value; it has {}", re.captures_len() - 1));
            }
            Ok(Source::Regex(re))
        }
        CaptureFrom::Line(0) => Err("lines are counted from 1".into()),
        CaptureFrom::Line(n) => Ok(Source::Line(*n as usize)),
    }
}

/// Checks a command's capture before it runs: every rule readable, every entry
/// it names there, every new one makeable.
pub fn plan(store: &Store, rules: &[CaptureRule], agent: &str, cmd: &str) -> Result<Capture, String> {
    if rules.is_empty() {
        return Err("capture needs at least one rule".into());
    }
    if rules.len() > MOST {
        return Err(format!("a command takes at most {MOST} capture rules"));
    }
    let mut out = Vec::with_capacity(rules.len());
    for (n, r) in rules.iter().enumerate() {
        let which = |e: String| if rules.len() > 1 { format!("capture rule {}: {e}", n + 1) } else { e };
        let from = rule_from(&r.from).map_err(which)?;
        let into = match (&r.into.id, &r.into.kind) {
            (Some(_), Some(_)) => return Err(which("\"as\" names an entry by id, or a new one by kind and label, not both".into())),
            (Some(id), None) => {
                let row = store.vault_get(id.trim()).map_err(|e| e.to_string())?.ok_or_else(|| which(format!("there is no entry {id:?}")))?;
                let kind = kinds::kind(&row.kind).ok_or_else(|| which(format!("entry {id} has an unknown kind")))?;
                let part = part_of(kind, r.into.part.as_deref()).map_err(which)?;
                if r.into.label.as_deref().is_some_and(|l| l.trim() != row.label) {
                    return Err(which(format!("{id} is called {:?}: leave label out to keep it", row.label)));
                }
                Target::Existing { id: row.id, created_at: row.created_at, label: row.label, part }
            }
            (None, Some(kind)) => {
                let k = kinds::kind(kind.trim()).ok_or_else(|| which(format!("unknown kind {kind:?}")))?;
                let label = r.into.label.as_deref().map(str::trim).unwrap_or("");
                if label.is_empty() {
                    return Err(which("a new entry needs a label".into()));
                }
                if label.chars().count() > MAX_RULE {
                    return Err(which(format!("a label is at most {MAX_RULE} characters")));
                }
                let part = part_of(k, r.into.part.as_deref()).map_err(which)?;
                if part != k.parts[0] {
                    return Err(which(format!("a new {} entry is made from its {}; capture the {part} into it once it exists", k.name, k.parts[0])));
                }
                Target::New { kind: k.name, label: label.to_string(), part }
            }
            (None, None) => return Err(which("\"as\" needs an entry's id, or a kind and label for a new one".into())),
        };
        let same = |t: &Target| match (t, &into) {
            (Target::New { kind: a, label: b, .. }, Target::New { kind: c, label: d, .. }) => a == c && b.eq_ignore_ascii_case(d),
            (Target::Existing { id: a, .. }, Target::Existing { id: b, .. }) => a == b,
            _ => false,
        };
        if out.iter().any(|r: &Rule| same(&r.into)) {
            return Err(which("two rules write the same entry; give each its own".into()));
        }
        out.push(Rule { into, from });
    }
    Ok(Capture { rules: out, agent: agent.to_string(), cmd: cmd.to_string() })
}

fn part_of(kind: &kinds::Kind, part: Option<&str>) -> Result<&'static str, String> {
    match part.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(kind.parts[0]),
        Some(p) => kind.parts.iter().copied().find(|k| *k == p).ok_or_else(|| format!("a {} entry has no part {p:?}; its parts are {}", kind.name, kind.parts.join(", "))),
    }
}

/// The JSON text at `steps`, exactly as the output printed it.
fn walk<'a>(mut raw: &'a RawValue, steps: &[Step]) -> Option<&'a RawValue> {
    for s in steps {
        raw = match s {
            Step::Key(k) => serde_json::from_str::<HashMap<String, &'a RawValue>>(raw.get()).ok()?.remove(k.as_str())?,
            Step::Index(i) => serde_json::from_str::<Vec<&'a RawValue>>(raw.get()).ok()?.into_iter().nth(*i)?,
        };
    }
    Some(raw)
}

/// A rule's value, and the text the output printed it as when that differs (a
/// JSON string's escaped form), which is cleaned out of the output too.
pub struct Found {
    pub value: Zeroizing<String>,
    pub printed: Option<Zeroizing<String>>,
}

/// One rule's value in `stdout`, or why there is none. Never says what it found.
pub fn extract(from: &Source, stdout: &str) -> Result<Found, String> {
    let mut printed = None;
    let value: Zeroizing<String> = match from {
        Source::All => Zeroizing::new(stdout.trim().to_string()),
        Source::Line(n) => Zeroizing::new(stdout.lines().nth(n - 1).ok_or_else(|| format!("the output has fewer than {n} lines"))?.trim().to_string()),
        Source::Regex(re) => {
            let caps = re.captures(stdout).ok_or("the regex matched nothing")?;
            Zeroizing::new(caps.get(1).ok_or("the regex's group matched nothing")?.as_str().trim().to_string())
        }
        Source::Json(steps) => {
            let top: &RawValue = serde_json::from_str(stdout.trim()).map_err(|_| "the output isn't JSON".to_string())?;
            let raw = walk(top, steps).ok_or("the JSON path matched nothing")?.get();
            match raw.as_bytes().first() {
                Some(b'"') => {
                    let decoded = Zeroizing::new(serde_json::from_str::<String>(raw).map_err(|_| "the output isn't JSON".to_string())?);
                    let inner = &raw[1..raw.len() - 1];
                    if inner != decoded.as_str() {
                        printed = Some(Zeroizing::new(inner.to_string()));
                    }
                    decoded
                }
                Some(b'-' | b'0'..=b'9') => Zeroizing::new(raw.to_string()),
                Some(b'n') => return Err("the JSON path matched nothing".into()),
                _ => return Err("the JSON path matched an object, a list or true/false, not a value".into()),
            }
        }
    };
    if value.is_empty() {
        return Err("the value it matched is empty".into());
    }
    if value.chars().count() < MIN_LEN {
        return Err(format!("the value it matched is shorter than {MIN_LEN} characters, too short to clean out of the output"));
    }
    Ok(Found { value, printed })
}

impl Capture {
    /// Whether any rule replaces the value of an entry that exists: the person is
    /// asked first.
    pub fn replaces(&self) -> impl Iterator<Item = &Target> {
        self.rules.iter().map(|r| &r.into).filter(|i| matches!(i, Target::Existing { .. }))
    }

    /// Applies the rules to `stdout` and stores what they match. `output` is the
    /// whole output as it would be published, kept for the person if the capture
    /// fails; `cut` says it was longer than [`MAX_HELD`].
    pub fn finish(&self, store: &Store, stdout: &str, output: &str, cut: bool, exit_code: i32) -> Outcome {
        let why = if cut {
            Err(format!("the output is over {} MB, more than a capture holds", MAX_HELD / (1024 * 1024)))
        } else if exit_code != 0 {
            Err(format!("the command failed (exit code {exit_code}), so nothing it printed is taken as the value"))
        } else {
            self.rules
                .iter()
                .enumerate()
                .map(|(n, r)| extract(&r.from, stdout).map_err(|e| if self.rules.len() > 1 { format!("capture rule {}: {e}", n + 1) } else { e }))
                .collect::<Result<Vec<_>, _>>()
        };
        let found = match why {
            Ok(v) => v,
            Err(why) => return self.withhold(store, why, output),
        };
        match self.store(store, &found) {
            Ok(ids) => {
                let mut known: Vec<(String, String)> = Vec::new();
                for (id, f) in ids.iter().zip(found.iter()) {
                    known.push((id.clone(), f.value.to_string()));
                    if let Some(p) = &f.printed {
                        known.push((id.clone(), p.to_string()));
                    }
                }
                Outcome::Stored { scrubber: Scrubber::new(&known), ids }
            }
            Err(why) => self.withhold(store, format!("the value couldn't be stored: {why}"), output),
        }
    }

    /// Stores every rule's value in one transaction: all of them, or none if an
    /// entry one replaces changed, or was deleted or made again, while the command
    /// ran. Each old main value is kept as a version in the same transaction.
    fn store(&self, store: &Store, found: &[Found]) -> Result<Vec<String>, String> {
        let key = seal::key(store.vault_count().map_err(|e| e.to_string())? == 0)?;
        let now = now_ms();
        let mut writes = Vec::with_capacity(found.len());
        let mut made: Vec<(String, String)> = Vec::with_capacity(found.len());
        for (rule, f) in self.rules.iter().zip(found) {
            match &rule.into {
                Target::New { kind, label, part } => {
                    let k = kinds::kind(kind).ok_or_else(|| format!("unknown kind {kind:?}"))?;
                    let mut parts = Parts::default();
                    parts.0.insert(part.to_string(), f.value.to_string());
                    let (fields, tags) = (BTreeMap::new(), Vec::<String>::new());
                    validate(k, label, &fields, &tags, &parts)?;
                    let id = new_id(store, kind, label)?;
                    writes.push(CaptureWrite::Insert(VaultRow {
                        id: id.clone(),
                        kind: kind.to_string(),
                        label: label.clone(),
                        fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?,
                        tags: serde_json::to_string(&tags).map_err(|e| e.to_string())?,
                        sealed: seal::seal(&key, &id, &parts)?,
                        created_at: now,
                        updated_at: now,
                        last_used_at: 0,
                    }));
                    made.push((id, label.clone()));
                }
                Target::Existing { id, created_at, label, part } => {
                    let gone = || format!("{label} was deleted while the command ran");
                    let row = store.vault_get(id).map_err(|e| e.to_string())?.filter(|r| r.created_at == *created_at).ok_or_else(gone)?;
                    let kind = kinds::kind(&row.kind).ok_or_else(|| format!("{label} has an unknown kind"))?;
                    let mut parts = seal::open(&key, &row.id, &row.sealed)?;
                    if let Some(mut old) = parts.0.insert(part.to_string(), f.value.to_string()) {
                        zeroize::Zeroize::zeroize(&mut old);
                    }
                    let kept = (*part == kind.parts[0]).then(|| versions::of(&row, &self.agent, CAPTURED));
                    let was = row.sealed.clone();
                    let changed = VaultRow { sealed: seal::seal(&key, &row.id, &parts)?, updated_at: now, ..row };
                    writes.push(CaptureWrite::Replace { row: changed, was, kept });
                    made.push((id.clone(), label.clone()));
                }
            }
        }
        versions::prune(store);
        if !store.vault_capture_commit(&writes).map_err(|e| e.to_string())? {
            return Err("an entry it replaces changed while the command ran, so nothing was stored".into());
        }
        let target = clip(&self.cmd, 160);
        for (id, label) in &made {
            record(store, id, label, &self.agent, CAPTURED, &target, "done");
        }
        Ok(made.into_iter().map(|(id, _)| id).collect())
    }

    /// Keeps the withheld output as a note only the person opens (with Reveal), and
    /// logs the failed capture against each entry that exists.
    fn withhold(&self, store: &Store, why: String, output: &str) -> Outcome {
        let mut cut = output.len().min(MAX_HELD);
        while !output.is_char_boundary(cut) {
            cut -= 1;
        }
        let kept = output[..cut].trim_end();
        let kept_as = if kept.is_empty() {
            None
        } else {
            let mut parts = Parts::default();
            parts.0.insert("text".into(), kept.to_string());
            let label = format!("Output withheld from {}: {}", self.agent, clip(&self.cmd, 60));
            let entry = NewEntry { kind: "note".into(), label, fields: BTreeMap::new(), tags: vec!["capture".into()], parts };
            match insert(store, entry, &self.agent, "output withheld", &clip(&why, 160)) {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::warn!(error = %e, "vault: couldn't keep a failed capture's output");
                    None
                }
            }
        };
        for rule in &self.rules {
            if let Target::Existing { id, label, .. } = &rule.into {
                record(store, id, label, &self.agent, CAPTURED, &clip(&why, 160), "failed");
            }
        }
        Outcome::Withheld { why, kept_as }
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
