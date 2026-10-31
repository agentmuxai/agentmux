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

/// STS refuses a session shorter than 15 minutes, and the roles AgentMux fleets use allow at most an hour.
pub const MIN_SECONDS: u64 = 900;
pub const MAX_SECONDS: u64 = 3600;
const DEFAULT_MINTS_PER_HOUR: u32 = 12;
const DEFAULT_READ_SECONDS: u64 = 3600;
const DEFAULT_WRITE_SECONDS: u64 = 1800;
/// What a recipe may be given, substituted whole into one argument each.
const PLACEHOLDERS: [&str; 4] = ["reason", "seconds", "session", "agent"];
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
            return Err(format!("{arg:?} uses {{{name}}}; the placeholders are {{reason}}, {{seconds}}, {{session}} and {{agent}}"));
        }
        found.push(name);
        rest = &after[close + 1..];
    }
    if rest.contains('}') {
        return Err(format!("{arg:?} has a stray }}"));
    }
    Ok(found)
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
                    for arg in &argv {
                        placeholders_of(arg).map_err(&at)?;
                    }
                    Mint::Command { argv }
                }
            };
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
        self.grants.values().filter(|g| !g.disabled && g.open_to(slug)).map(|g| g.name.as_str()).collect()
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
    /// The version that last parsed.
    good: Option<Arc<Policy>>,
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
    let cached = guard.get_or_insert_with(|| Cached { path: path.to_path_buf(), stamp: None, good: None, error: None });
    if cached.path != path {
        *cached = Cached { path: path.to_path_buf(), stamp: None, good: None, error: None };
    }
    let Some(stamp) = stamp_of(path) else {
        // Gone: nothing is shared any more.
        *cached = Cached { path: path.to_path_buf(), stamp: None, good: None, error: None };
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
                cached.good = Some(Arc::new(policy));
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

/// The policy cache and `AGENTMUX_AWS_GRANTS_FILE` are one per process: tests that use either take turns.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
