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

use agentmux_common::api_types::{CaptureFrom, CaptureInto, CaptureRule};
use agentmux_common::time::now_ms;
use serde_json::value::RawValue;
use zeroize::Zeroizing;

use super::scrub::{Scrubber, MIN_LINE};
use super::{card, insert, kinds, new_id_besides, record_line, seal, validate, versions, Line, NewEntry, Parts};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use crate::backend::storage::vault_versions::CaptureWrite;

/// Most rules one command may have.
pub const MOST: usize = 8;
/// A shorter value is refused: the output couldn't be cleaned of it reliably.
pub const MIN_LEN: usize = 8;
/// The shortest value of a card's other parts (a security code, a month, a ZIP) a `parts` capture takes: they are
/// short by nature, and the output is cleaned of them on their own (`scrub.rs`).
pub const MIN_PART: usize = 2;
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
    /// `fields`: plain fields it starts with (a browser capture's origin, as its
    /// `service` or `site`).
    New { kind: &'static str, label: String, part: &'static str, fields: BTreeMap<String, String>, vault_id: String },
    /// An existing entry, as it was when the command was approved: a value is
    /// never stored into an entry deleted and made again since.
    Existing { id: String, created_at: i64, label: String, part: &'static str, kind: String },
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
    /// A new card's other parts, from the same output (`CaptureFrom::Parts`): the part and where it is. `from` is
    /// then the card's number.
    pub also: Vec<(&'static str, Source)>,
}

/// A command's capture, checked before it runs.
#[derive(Debug, Clone)]
pub struct Capture {
    pub rules: Vec<Rule>,
    /// The agent that ran the command, as its verified identity names it.
    pub agent: String,
    /// The command line, for the activity log and the kept output's label.
    pub cmd: String,
    /// The agent's reason for the capture, for the activity log.
    pub reason: String,
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
    /// Every rule matched: the entries, in rule order, the values to take out of
    /// the output, and the lines of a multi-line value too short for the scrubber
    /// to find on their own (see [`clean_held`]).
    Stored { ids: Vec<String>, scrubber: Scrubber, short: Vec<Zeroizing<String>> },
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

/// One line of held output: its stream, its text and when it was printed.
pub type HeldLine = (&'static str, String, u64);

/// The held output cleaned of every value, as the lines to publish. It is cleaned
/// as one text, so a multi-line value is found across the lines it was printed
/// on; lines it took up become one. If a line of such a value that is too short
/// for the scrubber to find alone is still there (printed apart from the rest),
/// the output can't be published: the reason is returned.
pub fn clean_held(held: &[HeldLine], scrubber: &Scrubber, short: &[Zeroizing<String>]) -> Result<Vec<HeldLine>, String> {
    let joined = Zeroizing::new(held.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join("\n"));
    let clean = scrubber.scrub(&joined);
    let lines: Vec<&str> = clean.split('\n').collect();
    if lines.iter().any(|l| short.iter().any(|f| l.trim() == f.as_str())) {
        return Err(format!("a line of a captured value is shorter than {MIN_LINE} characters and was printed apart from the rest, so it can't be cleaned out"));
    }
    if lines.len() == held.len() {
        return Ok(held.iter().zip(lines).map(|(h, l)| (h.0, l.to_string(), h.2)).collect());
    }
    // A value over several lines became one mark: which stream each line came from
    // no longer lines up, and all of it is published as standard output.
    let ts = held.last().map(|h| h.2).unwrap_or(0);
    Ok(lines.into_iter().map(|l| ("stdout", l.to_string(), ts)).collect())
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
        CaptureFrom::Parts(_) => Err("\"parts\" is a whole rule's, not one part's".into()),
    }
}

/// The keys a `parts` capture takes and the card part each fills (`exp` is the older single text, read into the month
/// and year when the card is settled).
const CARD_KEYS: [(&str, &str); 7] = [("number", "number"), ("exp", "expiry"), ("exp_month", "exp_month"), ("exp_year", "exp_year"), ("cvc", "cvc"), ("name", "name"), ("zip", "zip")];

/// A `parts` capture's sources: the card's number, and its other parts. Only a new card is made this way.
fn card_parts(map: &BTreeMap<String, CaptureFrom>, into: &CaptureInto) -> Result<(Source, Vec<(&'static str, Source)>), String> {
    if into.id.is_some() || into.kind.as_deref().map(str::trim) != Some("card") {
        return Err("\"parts\" makes a new card: name it in \"as\" by kind \"card\" and a label, not by an entry's id".into());
    }
    if into.part.as_deref().is_some_and(|p| !p.trim().is_empty()) {
        return Err("\"parts\" fills each part it names: leave part out".into());
    }
    if map.contains_key("exp") && (map.contains_key("exp_month") || map.contains_key("exp_year")) {
        return Err("give the expiry as \"exp\", or as \"exp_month\" and \"exp_year\", not both".into());
    }
    let (mut number, mut also) = (None, Vec::new());
    for (key, from) in map {
        let Some((_, part)) = CARD_KEYS.iter().find(|(k, _)| k == key) else {
            let names: Vec<&str> = CARD_KEYS.iter().map(|(k, _)| *k).collect();
            return Err(format!("a card has no {key:?} to capture; its parts are {}", names.join(", ")));
        };
        let source = rule_from(from).map_err(|e| format!("{key}: {e}"))?;
        if *part == "number" {
            number = Some(source);
        } else {
            also.push((*part, source));
        }
    }
    Ok((number.ok_or("\"parts\" needs the card's \"number\"")?, also))
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
        let (from, also) = match &r.from {
            CaptureFrom::Parts(map) => card_parts(map, &r.into).map_err(which)?,
            one => (rule_from(one).map_err(which)?, Vec::new()),
        };
        let into = match (&r.into.id, &r.into.kind) {
            (Some(_), Some(_)) => return Err(which("\"as\" names an entry by id, or a new one by kind and label, not both".into())),
            (Some(id), None) => {
                let row = store.vault_get(id.trim()).map_err(|e| e.to_string())?.ok_or_else(|| which(format!("there is no entry {id:?}")))?;
                if !super::vaults::entry_row_usable(store, &row.vault_id, Some(agent))? {
                    return Err(which(format!("there is no entry {id:?}")));
                }
                let kind = kinds::kind(&row.kind).ok_or_else(|| which(format!("entry {id} has an unknown kind")))?;
                let part = part_of(kind, r.into.part.as_deref()).map_err(which)?;
                if r.into.label.as_deref().is_some_and(|l| l.trim() != row.label) {
                    return Err(which(format!("{id} is called {:?}: leave label out to keep it", row.label)));
                }
                Target::Existing { id: row.id, created_at: row.created_at, label: row.label, part, kind: row.kind }
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
                // Settled now, before the command runs: a command that issues a key must not run when the value cannot be kept.
                let vault_id = super::vaults::default_for(store, Some(agent)).map_err(which)?;
                Target::New { kind: k.name, label: label.to_string(), part, fields: BTreeMap::new(), vault_id }
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
        out.push(Rule { into, from, also });
    }
    Ok(Capture { rules: out, agent: agent.to_string(), cmd: cmd.to_string(), reason: String::new() })
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
    /// A card's other parts, when the rule was a `parts` one.
    pub also: Vec<(&'static str, Found)>,
}

/// One rule's value in `stdout`, or why there is none. Never says what it found.
pub fn extract(from: &Source, stdout: &str) -> Result<Found, String> {
    extract_min(from, stdout, MIN_LEN)
}

/// [`extract`], taking a value of at least `min` characters.
pub fn extract_min(from: &Source, stdout: &str, min: usize) -> Result<Found, String> {
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
    if value.chars().count() < min {
        return Err(format!("the value it matched is shorter than {min} characters, too short to clean out of the output"));
    }
    Ok(Found { value, printed, also: Vec::new() })
}

impl Rule {
    /// The shortest main value the rule takes: a card's number, like any value, is cleaned out of the output as text; a
    /// security code, an expiry or a ZIP captured into a card that exists is short by nature (`MIN_PART`).
    fn min_len(&self) -> usize {
        match &self.into {
            Target::Existing { kind, part, .. } if kind == card::KIND && *part != "number" => MIN_PART,
            _ => MIN_LEN,
        }
    }
}

/// A rule's values in `stdout`: its main one, and a card's other parts, which may be short.
fn extract_rule(r: &Rule, stdout: &str) -> Result<Found, String> {
    let mut found = extract_min(&r.from, stdout, r.min_len())?;
    for (part, from) in &r.also {
        let f = extract_min(from, stdout, MIN_PART).map_err(|e| format!("the card's {part}: {e}"))?;
        found.also.push((part, f));
    }
    Ok(found)
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
                .map(|(n, r)| extract_rule(r, stdout).map_err(|e| if self.rules.len() > 1 { format!("capture rule {}: {e}", n + 1) } else { e }))
                .collect::<Result<Vec<_>, _>>()
        };
        let found = match why {
            Ok(v) => v,
            Err(why) => return self.withhold(store, why, output),
        };
        match self.store(store, &found) {
            Ok(ids) => {
                let mut known: Vec<(String, String)> = Vec::new();
                let mut short = Vec::new();
                for (id, f) in ids.iter().zip(found.iter()) {
                    known.push((id.clone(), f.value.to_string()));
                    if let Some(p) = &f.printed {
                        known.push((id.clone(), p.to_string()));
                    }
                    for (_, x) in &f.also {
                        known.push((id.clone(), x.value.to_string()));
                        if let Some(p) = &x.printed {
                            known.push((id.clone(), p.to_string()));
                        }
                    }
                    if f.value.contains('\n') {
                        short.extend(f.value.lines().map(str::trim).filter(|l| !l.is_empty() && l.len() < MIN_LINE).map(|l| Zeroizing::new(l.to_string())));
                    }
                }
                Outcome::Stored { scrubber: Scrubber::new(&known), ids, short }
            }
            Err(why) => self.withhold(store, format!("the value couldn't be stored: {why}"), output),
        }
    }

    /// Stores every rule's value in one transaction: all of them, or none if an
    /// entry one replaces changed, or was deleted or made again, while the command
    /// ran. Each old main value is kept as a version in the same transaction.
    pub fn store(&self, store: &Store, found: &[Found]) -> Result<Vec<String>, String> {
        let key = seal::key(store.vault_count().map_err(|e| e.to_string())? == 0)?;
        let now = now_ms();
        let mut writes = Vec::with_capacity(found.len());
        let mut made: Vec<(String, String)> = Vec::with_capacity(found.len());
        for (rule, f) in self.rules.iter().zip(found) {
            match &rule.into {
                Target::New { kind, label, part, fields, vault_id } => {
                    let k = kinds::kind(kind).ok_or_else(|| format!("unknown kind {kind:?}"))?;
                    let mut parts = Parts::default();
                    parts.0.insert(part.to_string(), f.value.to_string());
                    for (other, x) in &f.also {
                        parts.0.insert(other.to_string(), x.value.to_string());
                    }
                    let tags = Vec::<String>::new();
                    let mut fields: BTreeMap<String, String> = fields.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.trim().to_string())).collect();
                    // A card's number, expiry, brand and reminder date agree, as they do for a save by the person.
                    card::settle(k.name, &mut fields, &mut parts)?;
                    validate(k, label, &fields, &tags, &parts)?;
                    let taken: Vec<String> = made.iter().map(|(id, _)| id.clone()).collect();
                    let id = new_id_besides(store, kind, label, &taken)?;
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
                        vault_id: vault_id.clone(),
                    }));
                    made.push((id, label.clone()));
                }
                Target::Existing { id, created_at, label, part, .. } => {
                    let gone = || format!("{label} was deleted while the command ran");
                    let row = store.vault_get(id).map_err(|e| e.to_string())?.filter(|r| r.created_at == *created_at).ok_or_else(gone)?;
                    let mut parts = seal::open(&key, &row.id, &row.sealed)?;
                    if let Some(mut old) = parts.0.insert(part.to_string(), f.value.to_string()) {
                        zeroize::Zeroize::zeroize(&mut old);
                    }
                    // A captured expiry replaces the month and year, or the next save would put them back.
                    if row.kind == card::KIND && *part == "expiry" {
                        for stale in ["exp_month", "exp_year"] {
                            if let Some(mut old) = parts.0.remove(stale) {
                                zeroize::Zeroize::zeroize(&mut old);
                            }
                        }
                    }
                    let mut fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
                    card::settle(&row.kind, &mut fields, &mut parts)?;
                    // The whole entry as it was, whichever part this replaces: the person was
                    // told the old value is kept for 7 days.
                    let kept = Some(versions::of(&row, &self.agent, CAPTURED));
                    let was = row.sealed.clone();
                    let changed = VaultRow { sealed: seal::seal(&key, &row.id, &parts)?, fields: serde_json::to_string(&fields).map_err(|e| e.to_string())?, updated_at: now, ..row };
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
            record_line(store, Line { entry_id: id, label, actor: &self.agent, action: CAPTURED, target: &target, outcome: "done", reason: &self.reason });
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
                record_line(store, Line { entry_id: id, label, actor: &self.agent, action: CAPTURED, target: &clip(&why, 160), outcome: "failed", reason: &self.reason });
            }
        }
        Outcome::Withheld { why, kept_as }
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
