// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The extra environment variables a definition launches with: the `env`
//! content blob (`agent_content`, `content_type = "env"`), one `KEY=VALUE` per
//! line, `#` comments and blank lines allowed. The launch reads it the way
//! [`classify`] does (`frontend/app/view/agent/agent-model.ts`): a line that
//! isn't a comment and has no valid `KEY=` is skipped.
//!
//! The operator edits the blob freely in My Agents. An agent edits it through
//! `Agents env_set` (`server/app_api/agent_env.rs`), and these rules bound
//! what it may write: no name AgentMux itself sets or that points a provider
//! at another home, endpoint or credential, no value that looks like a secret,
//! and a small size. Everything an agent reads back has the values of
//! names it may not set hidden.

use std::collections::BTreeMap;

/// The `content_type` of the blob.
pub const CONTENT_TYPE: &str = "env";
/// The most lines (comments and blanks included) an agent may write.
pub const MAX_LINES: usize = 64;
/// The most bytes an agent may write.
pub const MAX_BYTES: usize = 4096;
/// What stands for a value an agent isn't shown.
pub const HIDDEN: &str = "[hidden]";

/// Names AgentMux sets for every agent, or that decide where its files and
/// tools are looked up, which proxy it uses or which libraries load.
const RESERVED_NAMES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "TMPDIR",
    "TEMP",
    "TMP",
    "SHELL",
    "COMSPEC",
    "GH_CONFIG_DIR",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_SHELL_PREFIX",
    "CODEX_HOME",
    "NODE_OPTIONS",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GITHUB_TOKEN",
    "GH_TOKEN",
];

/// Names that run a program or load code in a tool the agent starts, or that
/// point one at another config home, certificate store or server.
const RESERVED_PREFIXES: &[&str] = &[
    "LD_", "DYLD_", "PYTHON", "NODE_EXTRA", "NODE_TLS", "NODE_PATH", "NODE_REPL", "NPM_CONFIG_", "RUBY", "PERL", "BASH_", "SSH_", "SSL_",
    "XDG_", "CARGO_", "RUSTUP_", "RUSTC", "GIT_CONFIG", "GIT_SSH", "GIT_ASKPASS", "GIT_EXEC", "GIT_PROXY", "GIT_EXTERNAL", "GIT_TEMPLATE",
    "GIT_DIR", "GIT_WORK", "_JAVA", "JAVA_TOOL", "JDK_JAVA",
];

/// Names a shell reads and runs, whatever their prefix.
const RESERVED_SHELL_NAMES: &[&str] = &["ENV", "BASH_ENV", "PROMPT_COMMAND", "IFS", "ZDOTDIR", "CDPATH", "PS4"];

/// Name endings that point a tool at a server, proxy, certificate store or
/// home of its own.
const RESERVED_SUFFIXES: &[&str] =
    &["_BASE_URL", "_API_BASE", "_URL", "_ENDPOINT", "_HOST", "_PROXY", "_CA_BUNDLE", "_CA_CERTS", "_CERT_FILE", "_CONFIG_DIR", "_HOME", "_COMMAND", "_HELPER", "_PREFIX", "_EDITOR", "_PAGER"];

/// What an agent may set, each name spelled out: the settings that tune a
/// provider CLI's context, output, timeouts and telemetry, and a few ordinary
/// ones. A prefix would let through names that choose which program runs or
/// which provider is used, so there is none. Any other name is the operator's
/// to set in My Agents, because an agent's variables apply to another agent's
/// launches and a list of what is dangerous is never complete. The reserved
/// rules above still apply to these.
const ALLOWED_NAMES: &[&str] = &[
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "DISABLE_TELEMETRY",
    "DISABLE_ERROR_REPORTING",
    "DISABLE_AUTOUPDATER",
    "DISABLE_COST_WARNINGS",
    "MAX_THINKING_TOKENS",
    "MAX_MCP_OUTPUT_TOKENS",
    "MCP_TIMEOUT",
    "MCP_TOOL_TIMEOUT",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "RUST_LOG",
    "RUST_BACKTRACE",
    "NO_COLOR",
    "FORCE_COLOR",
    "TZ",
    "LANG",
    "LC_ALL",
];

/// Name endings that mark a credential. An ending, not a substring: a
/// setting such as `CLAUDE_CODE_MAX_OUTPUT_TOKENS` is not one.
const CREDENTIAL_SUFFIXES: &[&str] = &["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_PASSWD", "_CREDENTIAL", "_CREDENTIALS"];

/// How one line reads at launch.
#[derive(Debug, PartialEq, Eq)]
pub enum Line<'a> {
    Blank,
    Comment,
    Pair { key: &'a str, value: &'a str },
    /// Not blank, not a comment, and no valid `KEY=`: skipped at launch.
    Ignored,
}

/// How the launch reads `raw`: trimmed, `#` starts a comment, the key is
/// everything before the first `=` and must be a valid name (not trimmed, so
/// `KEY = v` is skipped), the value everything after it.
pub fn classify(raw: &str) -> Line<'_> {
    let line = raw.trim();
    if line.is_empty() {
        return Line::Blank;
    }
    if line.starts_with('#') {
        return Line::Comment;
    }
    match line.find('=') {
        Some(at) if at >= 1 && is_valid_name(&line[..at]) => Line::Pair { key: &line[..at], value: &line[at + 1..] },
        _ => Line::Ignored,
    }
}

/// `^[a-zA-Z_][a-zA-Z0-9_]*$`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The variables the launch ends up with: a name given twice has its last value.
pub fn effective(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| match classify(l) {
            Line::Pair { key, value } => Some((key.to_string(), value.to_string())),
            _ => None,
        })
        .collect()
}

/// Whether `name` looks like it holds a credential.
pub fn is_credential_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "TOKEN" | "SECRET" | "PASSWORD" | "APIKEY") || CREDENTIAL_SUFFIXES.iter().any(|s| upper.ends_with(s))
}

/// Names the providers use for their config homes, endpoints and the
/// variables they are launched without, which an agent mustn't change.
fn provider_names() -> impl Iterator<Item = String> {
    crate::backend::providers::all_providers().flat_map(|p| {
        std::iter::once(p.auth_config_dir_env_var.to_string())
            .chain(p.base_url_env_var.map(str::to_string))
            .chain(p.auth_extra_env.iter().map(|(k, _)| k.to_string()))
            .chain(p.unset_env.iter().map(|k| k.to_string()))
    })
}

/// Why an agent may not set or remove `name`, if it may not. A pattern, not
/// a list of names: a variable that runs code or redirects traffic in the
/// tools another agent launches is refused whatever it is called.
pub fn name_problem(name: &str) -> Option<String> {
    if !is_valid_name(name) {
        return Some(format!("{name}: a name is letters, digits and underscores, not starting with a digit"));
    }
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("AGENTMUX_") {
        return Some(format!("{name}: names starting AGENTMUX_ are reserved for AgentMux"));
    }
    let reserved = RESERVED_NAMES.contains(&upper.as_str())
        || RESERVED_SHELL_NAMES.contains(&upper.as_str())
        || RESERVED_PREFIXES.iter().any(|p| upper.starts_with(p))
        || RESERVED_SUFFIXES.iter().any(|x| upper.ends_with(x));
    if reserved || provider_names().any(|n| n.eq_ignore_ascii_case(name)) {
        return Some(format!("{name}: AgentMux sets this one itself, or it decides where an agent's tools, credentials or traffic go"));
    }
    if is_credential_name(name) {
        return Some(format!("{name}: looks like a credential; the operator sets those in My Agents"));
    }
    if !ALLOWED_NAMES.contains(&upper.as_str()) {
        return Some(format!("{name}: an agent may set only {}; the operator sets any other in My Agents", ALLOWED_NAMES.join(", ")));
    }
    None
}

/// Whether `value` has the shape of a secret (a token with a known prefix, a
/// private key, a JWT, ...).
pub fn looks_like_secret(value: &str) -> bool {
    agentmux_common::redact::redact_secrets(value) != value
}

/// Why an agent may not give a variable `value`, if it may not.
pub fn value_problem(name: &str, value: &str) -> Option<String> {
    if value == HIDDEN {
        return Some(format!("{name}: {HIDDEN} stands for a value you aren't shown; use set and unset to change other variables"));
    }
    if value.chars().any(|c| c.is_control() && c != '\t') {
        return Some(format!("{name}: a value is one line of plain text"));
    }
    if looks_like_secret(value) {
        return Some(format!("{name}: the value looks like a secret; values here are plain text, so keep secrets in the Vault"));
    }
    None
}

/// `text` within the size an agent may write.
pub fn check_size(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err(format!("the environment is {} bytes; at most {MAX_BYTES}", text.len()));
    }
    let lines = text.lines().count();
    if lines > MAX_LINES {
        return Err(format!("the environment is {lines} lines; at most {MAX_LINES}"));
    }
    Ok(())
}

/// What an agent is shown for a line it may not read: a comment or a skipped
/// line standing where it can't be told apart from a secret.
const HIDDEN_LINE: &str = "# [hidden]";

/// Whether an agent is shown `raw` as `[hidden]`: a value it may not read, a
/// skipped line (a mistyped `NAME = value`), or a comment that holds a `=` or
/// `:` or a secret, since a commented-out credential is still one.
fn is_hidden_line(raw: &str) -> bool {
    match classify(raw) {
        Line::Blank => false,
        Line::Comment => raw.contains('=') || raw.contains(':') || looks_like_secret(raw),
        Line::Ignored => true,
        Line::Pair { key, value } => shown(key, value) == HIDDEN,
    }
}

/// The name of a line an agent can neither read nor write: its name is one an
/// agent may not set, or it is shown to an agent as hidden. A hidden comment
/// or skipped line has the name "".
fn is_kept_line(raw: &str) -> Option<&str> {
    match classify(raw) {
        Line::Pair { key, .. } if name_problem(key).is_some() || is_hidden_line(raw) => Some(key),
        Line::Comment | Line::Ignored if is_hidden_line(raw) => Some(""),
        _ => None,
    }
}

/// The text an agent proposes as the whole environment (`lines`), with the
/// lines it can't write kept. It is within the size, and every line is a
/// comment, a blank or a `KEY=VALUE` it may write; a line that would be
/// skipped at launch is refused rather than kept silently. A name in
/// `current` that the agent may not write (a credential, a reserved name, a
/// hidden value) stays as the operator wrote it: where the agent kept
/// the line as `env_get` showed it (`NAME=[hidden]`, or unchanged), the
/// operator's line goes back in that place, and one the agent left out goes at the end.
pub fn resolve_lines(current: &str, text: &str) -> Result<String, String> {
    check_size(text)?;
    let mut kept: Vec<(&str, &str, bool)> = current.lines().filter_map(|raw| is_kept_line(raw).map(|key| (key, raw, false))).collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        match classify(raw) {
            Line::Blank => out.push(raw),
            Line::Comment => {
                let operators = if raw.trim() == HIDDEN_LINE { kept.iter_mut().find(|(k, _, used)| k.is_empty() && !*used) } else { None };
                if let Some((_, original, used)) = operators {
                    *used = true;
                    out.push(original);
                } else {
                    out.push(raw);
                }
            }
            Line::Ignored => return Err(format!("line {}: not a comment and not KEY=VALUE, so it would be ignored", i + 1)),
            Line::Pair { key, value } => {
                let operators = kept.iter_mut().find(|(k, original, used)| *k == key && !*used && (value == HIDDEN || original.trim() == raw.trim()));
                if let Some((_, original, used)) = operators {
                    *used = true;
                    out.push(original);
                } else if let Some(why) = name_problem(key).or_else(|| value_problem(key, value)) {
                    return Err(format!("line {}: {why}", i + 1));
                } else {
                    out.push(raw);
                }
            }
        }
    }
    out.extend(kept.iter().filter(|(_, _, used)| !used).map(|(_, raw, _)| *raw));
    let mut joined = out.join("\n");
    if !joined.is_empty() && text.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// `current` with `set` applied and `unset` removed, every other line (and
/// comment) as it was. A name already there has its value replaced where it
/// stands, a new one goes at the end.
pub fn edit(current: &str, set: &BTreeMap<String, String>, unset: &[String]) -> Result<String, String> {
    for (name, value) in set {
        if let Some(why) = name_problem(name).or_else(|| value_problem(name, value)) {
            return Err(why);
        }
    }
    for name in unset {
        if let Some(why) = name_problem(name) {
            return Err(why);
        }
    }
    let trailing_newline = current.is_empty() || current.ends_with('\n');
    let mut seen: Vec<&str> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    for raw in current.lines() {
        match classify(raw) {
            Line::Pair { key, .. } if unset.iter().any(|n| n == key) => {}
            Line::Pair { key, .. } if set.contains_key(key) => {
                seen.push(key);
                out.push(format!("{key}={}", set[key]));
            }
            _ => out.push(raw.to_string()),
        }
    }
    for (name, value) in set {
        if !seen.contains(&name.as_str()) {
            out.push(format!("{name}={value}"));
        }
    }
    let mut text = out.join("\n");
    if !text.is_empty() && trailing_newline {
        text.push('\n');
    }
    Ok(text)
}

/// What an edit did, by name. Values of credential-looking names are `HIDDEN`.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Change {
    pub added: BTreeMap<String, String>,
    pub changed: BTreeMap<String, ValueChange>,
    pub removed: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct ValueChange {
    pub from: String,
    pub to: String,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    /// The names and what happened to each, never a value: for the audit entry.
    pub fn summary(&self) -> String {
        let names = |keys: Vec<&String>| keys.into_iter().cloned().collect::<Vec<_>>().join(",");
        format!(
            "added [{}] changed [{}] removed [{}]",
            names(self.added.keys().collect()),
            names(self.changed.keys().collect()),
            self.removed.join(",")
        )
    }
}

/// What a value is shown as to an agent: only the value of a name an agent may
/// set, and not one that looks like a secret. Anything else is the operator's,
/// and a secret can sit under a name no pattern recognises (`PGPASSWORD`,
/// `DB_PASS`), so the name decides, not a guess at what the value is.
fn shown(name: &str, value: &str) -> String {
    let readable = ALLOWED_NAMES.contains(&name.to_ascii_uppercase().as_str()) && !is_credential_name(name) && !looks_like_secret(value);
    if readable { value.to_string() } else { HIDDEN.to_string() }
}

/// What changed in the variables the launch gets from `old` to `new`.
pub fn diff(old: &str, new: &str) -> Change {
    let (old, new) = (effective(old), effective(new));
    let mut change = Change::default();
    for (name, value) in &new {
        match old.get(name) {
            None => {
                change.added.insert(name.clone(), shown(name, value));
            }
            Some(before) if before != value => {
                change.changed.insert(name.clone(), ValueChange { from: shown(name, before), to: shown(name, value) });
            }
            Some(_) => {}
        }
    }
    change.removed = old.keys().filter(|n| !new.contains_key(*n)).cloned().collect();
    change
}

/// `text` as an agent is shown it: the values of names it may not set hidden,
/// a comment that may hold a secret and a skipped line as `# [hidden]`, every
/// other line as written.
pub fn redacted(text: &str) -> String {
    let mut out: Vec<String> = text
        .lines()
        .map(|raw| match classify(raw) {
            Line::Pair { key, .. } if is_hidden_line(raw) => format!("{key}={HIDDEN}"),
            Line::Comment | Line::Ignored if is_hidden_line(raw) => HIDDEN_LINE.to_string(),
            _ => raw.to_string(),
        })
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    out.join("\n")
}

#[cfg(test)]
#[path = "agent_env_tests.rs"]
mod tests;
