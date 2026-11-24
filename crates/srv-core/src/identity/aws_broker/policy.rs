// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The AWS grants a host shares with its container agents (`~/.agentmux/aws-grants.json`).
//! Spec: docs/specs/SPEC_CONTAINER_AGENT_AWS_CREDENTIALS_2026_10_31.md §6.
//!
//! Closed by default: no file, no grants. A file that does not parse is an error naming the line, and the version
//! that last parsed stays in force until it is fixed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Deserialize;

use super::record;

/// STS refuses a session shorter than 15 minutes, and the roles AgentMux fleets use allow at most an hour.
pub const MIN_SECONDS: u64 = 900;
pub const MAX_SECONDS: u64 = 3600;
const DEFAULT_MINTS_PER_HOUR: u32 = 12;
const DEFAULT_READ_SECONDS: u64 = 3600;
const DEFAULT_WRITE_SECONDS: u64 = 1800;
/// What a recipe may be given, substituted whole into one argument each.
const PLACEHOLDERS: [&str; 6] = ["reason", "seconds", "session", "agent", "tag", "tag_key"];
/// The most characters a caller-supplied session tag value, and a tag key, may have.
pub const TAG_VALUE_MAX: usize = 256;
const TAG_KEY_MAX: usize = 128;
/// The characters a caller-supplied tag may hold, besides letters and digits: no space, and none that is a wildcard.
const TAG_CHARS: &str = "_.:/=+-@";
/// The environment variable that points at another policy file (tests, an isolated instance).
pub const FILE_ENV: &str = "AGENTMUX_AWS_GRANTS_FILE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Read,
    Write,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Read => "read",
            Tier::Write => "write",
        }
    }
}

/// Why a grant in the file is not honoured yet (`record`): it is new or wider than what was last allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Waiting for a native agent or a person to allow it.
    Awaiting,
    /// Someone said it must not be used.
    Refused,
}

/// One session tag the caller supplies on each mint, under a key the grant fixes. A grant for a role whose own policy builds
/// what it may reach from a tag takes the tag's value from the caller, so the value is checked here: it starts with the
/// prefix the grant fixes, and holds only letters, digits and `_.:/=+-@`, at most 256 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerTag {
    /// The tag's key as the recipe sets it (`{tag_key}`).
    pub key: String,
    /// What every value starts with. May be empty.
    pub prefix: String,
}

fn tag_chars_ok(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || TAG_CHARS.contains(c))
}

impl CallerTag {
    /// The value the caller gave, or the sentence that says why it is not allowed. Nothing is cleaned: a value that is not
    /// allowed is refused, not changed into one that is.
    pub fn check(&self, grant: &str, given: Option<&str>) -> Result<String, String> {
        let Some(value) = given.map(str::trim).filter(|v| !v.is_empty()) else {
            return Err(format!(
                "The grant '{grant}' needs the {} tag. Run the command with AGENTMUX_AWS_TAG='<value>' in front of it; the value starts with '{}'.",
                self.key, self.prefix
            ));
        };
        // The value is one whole argument of a program, which reads one that starts with '-' as an option: never given to it.
        if value.starts_with('-') {
            return Err(format!("The {} tag for the grant '{grant}' may not start with '-'.", self.key));
        }
        if value.chars().count() > TAG_VALUE_MAX || !tag_chars_ok(value) {
            return Err(format!(
                "The {} tag for the grant '{grant}' may hold only letters, digits and _.:/=+-@ (no spaces), at most {TAG_VALUE_MAX} characters.",
                self.key
            ));
        }
        if !value.starts_with(&self.prefix) {
            return Err(format!("The {} tag for the grant '{grant}' has to start with '{}'.", self.key, self.prefix));
        }
        Ok(value.to_string())
    }
}

/// How srv gets the credentials (spec §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mint {
    /// The host's own AWS profile, through the AWS CLI.
    HostProfile { profile: String },
    /// A program that prints `credential_process` JSON.
    Command { argv: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub name: String,
    pub tier: Tier,
    pub mint: Mint,
    pub default_seconds: u64,
    pub max_seconds: u64,
    pub needs_reason: bool,
    /// Agent slugs (lower case), or `*`.
    pub agents: Vec<String>,
    pub max_mints_per_hour: u32,
    pub disabled: bool,
    pub note: Option<String>,
    /// One tag the caller names on each mint (`None`: the grant takes none).
    pub caller_tag: Option<CallerTag>,
    /// Set when the file's version of this grant has not been allowed: it is refused. Always none from `parse`.
    pub hold: Option<Hold>,
}

impl Grant {
    /// Whether the agent with this slug may have the grant.
    pub fn open_to(&self, slug: &str) -> bool {
        self.agents.iter().any(|a| a == "*" || a.eq_ignore_ascii_case(slug))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub region: String,
    pub default_grant: Option<String>,
    pub grants: BTreeMap<String, Grant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    version: u32,
    region: Option<String>,
    default_grant: Option<String>,
    #[serde(default)]
    grants: BTreeMap<String, RawGrant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGrant {
    tier: String,
    mint: RawMint,
    default_seconds: Option<u64>,
    max_seconds: Option<u64>,
    needs_reason: Option<bool>,
    #[serde(default)]
    agents: Vec<String>,
    max_mints_per_hour: Option<u32>,
    #[serde(default)]
    disabled: bool,
    note: Option<String>,
    caller_tag: Option<RawCallerTag>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCallerTag {
    key: String,
    #[serde(default)]
    prefix: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawMint {
    HostProfile { profile: String },
    Command { argv: Vec<String> },
}

/// A grant's name: what the container's profile is called.
pub fn is_grant_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && s.len() <= 41 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_profile_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.starts_with('-') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.@:/+=-".contains(&b))
}

fn is_region(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The placeholders an argument uses (`{name}`), or what is wrong with it.
fn placeholders_of(arg: &str) -> Result<Vec<&str>, String> {
    let mut found = Vec::new();
    let mut rest = arg;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| format!("{arg:?} has an unclosed {{"))?;
        let name = &after[..close];
        if !PLACEHOLDERS.contains(&name) {
            return Err(format!("{arg:?} uses {{{name}}}; the placeholders are {{reason}}, {{seconds}}, {{session}}, {{agent}}, {{tag}} and {{tag_key}}"));
        }
        found.push(name);
        rest = &after[close + 1..];
    }
    if rest.contains('}') {
        return Err(format!("{arg:?} has a stray }}"));
    }
    Ok(found)
}

/// A grant's `caller_tag`, checked: it needs a command recipe, and a key and prefix with only the characters a tag may hold.
fn caller_tag_of(raw: Option<RawCallerTag>, mint: &Mint) -> Result<Option<CallerTag>, String> {
    let Some(t) = raw else { return Ok(None) };
    if matches!(mint, Mint::HostProfile { .. }) {
        return Err("a caller_tag needs a command recipe".to_string());
    }
    if t.key.is_empty() || t.key.len() > TAG_KEY_MAX || !tag_chars_ok(&t.key) || t.key.to_ascii_lowercase().starts_with("aws:") {
        return Err(format!("caller_tag key {:?}: letters, digits and _.:/=+-@, at most {TAG_KEY_MAX} characters, and not starting with aws:", t.key));
    }
    if t.prefix.len() > TAG_VALUE_MAX || !tag_chars_ok(&t.prefix) {
        return Err(format!("caller_tag prefix {:?}: letters, digits and _.:/=+-@, at most {TAG_VALUE_MAX} characters", t.prefix));
    }
    // Both reach the program as whole arguments, which it would read as options.
    if t.key.starts_with('-') || t.prefix.starts_with('-') {
        return Err("caller_tag: the key and the prefix may not start with '-'".to_string());
    }
    Ok(Some(CallerTag { key: t.key, prefix: t.prefix }))
}

impl Policy {
    /// Parse and check a policy file's text. The error names the line (JSON errors) or the grant.
    pub fn parse(text: &str) -> Result<Policy, String> {
        let raw: RawPolicy = serde_json::from_str(text).map_err(|e| format!("line {}, column {}: {e}", e.line(), e.column()))?;
        if raw.version != 1 {
            return Err(format!("version {} is not known: this AgentMux reads version 1", raw.version));
        }
        let region = raw.region.unwrap_or_else(|| "us-east-1".to_string());
        if !is_region(&region) {
            return Err(format!("region {region:?} is not an AWS region name"));
        }
        let mut grants = BTreeMap::new();
        for (name, g) in raw.grants {
            if !is_grant_name(&name) {
                return Err(format!("grant name {name:?}: lower case letters, digits and -, starting with a letter, at most 41 characters"));
            }
            let at = |m: String| format!("grant {name:?}: {m}");
            let tier = match g.tier.as_str() {
                "read" => Tier::Read,
                "write" => Tier::Write,
                other => return Err(at(format!("tier {other:?} is not read or write"))),
            };
            let mint = match g.mint {
                RawMint::HostProfile { profile } => {
                    if !is_profile_name(&profile) {
                        return Err(at(format!("profile {profile:?} is not a usable AWS profile name")));
                    }
                    Mint::HostProfile { profile }
                }
                RawMint::Command { argv } => {
                    let Some(program) = argv.first() else { return Err(at("the command's argv is empty".to_string())) };
                    if program.is_empty() || program.contains('{') {
                        return Err(at("the program (argv[0]) must be a fixed path or name".to_string()));
                    }
                    let mut used = Vec::new();
                    for arg in &argv {
                        used.extend(placeholders_of(arg).map_err(&at)?);
                    }
                    let takes_tag = g.caller_tag.is_some();
                    if used.iter().any(|p| matches!(*p, "tag" | "tag_key")) && !takes_tag {
                        return Err(at("the command uses {tag} or {tag_key}, and the grant has no caller_tag".to_string()));
                    }
                    if takes_tag && !used.contains(&"tag") {
                        return Err(at("caller_tag is set and no argument of the command uses {tag}".to_string()));
                    }
                    Mint::Command { argv }
                }
            };
            let caller_tag = caller_tag_of(g.caller_tag, &mint).map_err(&at)?;
            let default_seconds = g.default_seconds.unwrap_or(if tier == Tier::Read { DEFAULT_READ_SECONDS } else { DEFAULT_WRITE_SECONDS });
            let max_seconds = g.max_seconds.unwrap_or(MAX_SECONDS);
            for (what, secs) in [("default_seconds", default_seconds), ("max_seconds", max_seconds)] {
                if !(MIN_SECONDS..=MAX_SECONDS).contains(&secs) {
                    return Err(at(format!("{what} {secs} is outside {MIN_SECONDS} to {MAX_SECONDS}")));
                }
            }
            if default_seconds > max_seconds {
                return Err(at(format!("default_seconds {default_seconds} is more than max_seconds {max_seconds}")));
            }
            let agents: Vec<String> = g.agents.iter().map(|a| a.trim().to_lowercase()).collect();
            if agents.iter().any(String::is_empty) {
                return Err(at("agents has an empty name".to_string()));
            }
            let max_mints_per_hour = g.max_mints_per_hour.unwrap_or(DEFAULT_MINTS_PER_HOUR);
            if max_mints_per_hour == 0 {
                return Err(at("max_mints_per_hour is 0: use disabled to turn a grant off".to_string()));
            }
            grants.insert(
                name.clone(),
                Grant {
                    name,
                    tier,
                    mint,
                    default_seconds,
                    max_seconds,
                    needs_reason: g.needs_reason.unwrap_or(tier == Tier::Write),
                    agents,
                    max_mints_per_hour,
                    disabled: g.disabled,
                    note: g.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
                    caller_tag,
                    hold: None,
                },
            );
        }
        if let Some(default) = raw.default_grant.as_deref() {
            if !grants.contains_key(default) {
                return Err(format!("default_grant {default:?} is not one of the grants"));
            }
        }
        Ok(Policy { region, default_grant: raw.default_grant, grants })
    }

    /// The grant names an agent may be told about: every grant, disabled or not (a refusal says why).
    pub fn names(&self) -> Vec<&str> {
        self.grants.keys().map(String::as_str).collect()
    }

    /// The names of the grants open to `slug`.
    pub fn open_to(&self, slug: &str) -> Vec<&str> {
        self.grants.values().filter(|g| !g.disabled && g.hold.is_none() && g.open_to(slug)).map(|g| g.name.as_str()).collect()
    }

    /// The AWS config a container reads: one profile per grant, `default` when the policy names one. Names and the
    /// command only; no secret is ever in it.
    pub fn render_config(&self) -> String {
        let mut out = String::from("# Written by AgentMux; read-only. Change the grants in ~/.agentmux/aws-grants.json on the host.\n");
        let profile = |name: &str| format!("credential_process = agentmux-bashwrap aws-credentials --profile {name}\nregion = {}\n", self.region);
        if let Some(default) = &self.default_grant {
            out.push_str(&format!("\n[default]\n{}", profile(default)));
        }
        for name in self.grants.keys() {
            out.push_str(&format!("\n[profile {name}]\n{}", profile(name)));
        }
        out
    }
}

/// The policy file's path: the override in [`FILE_ENV`], else `aws-grants.json` in the AgentMux root.
pub fn file_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(FILE_ENV).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    crate::srv_info::agentmux_root().map(|root| root.join("aws-grants.json"))
}

struct Cached {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    /// The version that last parsed, as the broker honours it: a new or wider grant is held (`record`).
    good: Option<Arc<Policy>>,
    /// The same version as the file says it, to work the holds out again when a grant is allowed.
    raw: Option<Arc<Policy>>,
    /// What the current file said, once, so a bad edit is logged when it is made and not on every request.
    error: Option<String>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// The policy now in force. `Ok(None)`: no file, nothing shared. `Err`: the file does not parse and no earlier
/// version exists.
pub fn current() -> Result<Option<Arc<Policy>>, String> {
    match file_path() {
        Some(path) => load(&path),
        None => Ok(None),
    }
}

fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn load(path: &Path) -> Result<Option<Arc<Policy>>, String> {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cached = guard.get_or_insert_with(|| Cached { path: path.to_path_buf(), stamp: None, good: None, raw: None, error: None });
    if cached.path != path {
        *cached = Cached { path: path.to_path_buf(), stamp: None, good: None, raw: None, error: None };
    }
    let Some(stamp) = stamp_of(path) else {
        // Gone: nothing is shared any more.
        *cached = Cached { path: path.to_path_buf(), stamp: None, good: None, raw: None, error: None };
        return Ok(None);
    };
    if cached.stamp != Some(stamp) {
        // The stamp is kept only once the file has been read: a read that failed (a lock, a permission) is tried again
        // on the next request and says so, instead of looking like a file that was already dealt with.
        let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        cached.stamp = Some(stamp);
        match Policy::parse(&text) {
            Ok(policy) => {
                tracing::info!(target: "identity", grants = policy.grants.len(), "identity.aws: the grants file was read");
                let sha = record::sha256_hex(&text);
                cached.good = Some(Arc::new(guarded(path, &policy, &sha)));
                cached.raw = Some(Arc::new(policy));
                cached.error = None;
            }
            Err(e) => {
                tracing::warn!(target: "identity", error = %e, kept_previous = cached.good.is_some(), "identity.aws: the grants file does not parse");
                cached.error = Some(e);
            }
        }
    }
    match (&cached.good, &cached.error) {
        (Some(good), _) => Ok(Some(good.clone())),
        (None, Some(e)) => Err(format!("{}: {e}", path.display())),
        (None, None) => Ok(None),
    }
}

/// The audit action of a change to the grants file, and of an answer to a held grant.
pub const AUDIT_ACTION: &str = "aws.grants";

/// The file's grants compared with the record: what changed is recorded in the Audit view, the record is brought up to
/// date, and the policy comes back with every new or wider grant held. A record that can't be read holds everything.
fn guarded(path: &Path, policy: &Policy, sha: &str) -> Policy {
    let record_path = record::path_beside(path);
    let old = match record::read(&record_path) {
        Ok(old) => old,
        Err(e) => {
            tracing::warn!(target: "identity", error = %e, "identity.aws: the grants record can't be read, so every grant is held");
            return record::hold_all(policy);
        }
    };
    let (rec, events) = record::sync(old.clone(), policy, sha, agentmux_common::time::now_ms());
    if old.as_ref() != Some(&rec) {
        if let Err(e) = record::write(&record_path, &rec) {
            tracing::warn!(target: "identity", error = %e, "identity.aws: the grants record was not saved");
        }
    }
    emit(&events);
    record::effective(policy, &rec)
}

/// Puts each event in the Audit view: a held grant as a failure, so the view marks it; never a credential.
fn emit(events: &[record::Event]) {
    let handler = crate::backend::reactive::get_global_handler();
    for e in events {
        let target = e.grant.as_deref().unwrap_or("aws grants");
        let error = e.held.then_some("held");
        handler.log_fleet_action_audit(Some(e.by.as_deref().unwrap_or("agentmux")), target, "", AUDIT_ACTION, !e.held, error, &e.id, Some(&e.text));
        tracing::info!(target: "identity", kind = e.kind, held = e.held, "identity.aws: {}", e.text);
    }
}

/// Answers a held grant: allows it or refuses it, with the reason, by an agent or a person, and puts that in the record
/// and the Audit view. The grant works (or stays refused) from the next request.
pub fn decide(d: &record::Decision<'_>) -> Result<record::Event, String> {
    let path = file_path().ok_or_else(|| "this AgentMux has no data folder".to_string())?;
    current()?;
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cached = guard.as_mut().ok_or_else(|| "the grants file has not been read".to_string())?;
    let raw = cached.raw.clone().ok_or_else(|| "there is no grants file to answer for".to_string())?;
    let record_path = record::path_beside(&path);
    let mut rec = record::read(&record_path)?.ok_or_else(|| "there is no record of the grants file yet".to_string())?;
    let event = record::decide(&mut rec, d, agentmux_common::time::now_ms())?;
    record::write(&record_path, &rec)?;
    cached.good = Some(Arc::new(record::effective(&raw, &rec)));
    emit(std::slice::from_ref(&event));
    Ok(event)
}

/// The grants as the record has them, with what changed lately: when each was added and by which change.
pub fn view() -> Result<record::View, String> {
    let path = file_path().ok_or_else(|| "this AgentMux has no data folder".to_string())?;
    current()?;
    let rec = record::read(&record::path_beside(&path))?;
    Ok(rec.map(|r| record::view(&r)).unwrap_or(record::View { grants: Vec::new(), changes: Vec::new() }))
}

/// The policy cache and `AGENTMUX_AWS_GRANTS_FILE` are one per process: tests that use either take turns.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
