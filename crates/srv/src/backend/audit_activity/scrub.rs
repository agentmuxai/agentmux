// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one-line summaries of tool calls. A summary is made when the row is written and never copies a raw call: a
//! command is cut to its start, a path is the path, a MuxAPI tool is its tool and action. Text that still looks like a
//! secret is dropped, and the row says only which tool it was.

use serde_json::Value;

/// The longest a command or pattern is kept.
const TEXT_MAX: usize = 80;
/// The longest a path is kept.
const PATH_MAX: usize = 200;

/// Words in the name of a variable or flag that make what follows a secret: `API_KEY=…`, `--token …`.
const SECRET_NAMES: [&str; 8] = ["pass", "secret", "token", "key", "auth", "cred", "cookie", "bearer"];
/// What a known kind of secret starts with.
const SECRET_PREFIXES: [&str; 11] = ["sk-", "sk_", "ghp_", "gho_", "ghs_", "ghu_", "github_pat_", "xox", "akia", "asia", "aiza"];

/// The one-line summary of the call `tool(input)`. Never longer than a line, and never a secret.
pub fn summarize(tool: &str, input: &Value) -> String {
    let field = |name: &str| input.get(name).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let text = |value: &str| {
        let one_line = first_line(value, TEXT_MAX);
        if looks_secret(&one_line) {
            None
        } else {
            Some(one_line)
        }
    };
    let detail = match tool {
        "Bash" => field("command").and_then(text),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            field("file_path").or_else(|| field("notebook_path")).map(|p| first_line(p, PATH_MAX))
        }
        "Glob" | "Grep" => field("pattern").and_then(text),
        "WebFetch" | "WebSearch" => field("url").map(site_of).or_else(|| field("query").and_then(text)),
        "Task" | "Agent" => field("description").and_then(text),
        _ => mux_detail(tool, input),
    };
    match detail {
        Some(d) => format!("{} {d}", short_name(tool)),
        None => short_name(tool),
    }
}

/// `mcp__agentmux__Message` is `Message`; any other tool keeps its name.
fn short_name(tool: &str) -> String {
    tool.strip_prefix("mcp__agentmux__").unwrap_or(tool).to_string()
}

/// A MuxAPI tool's action, and for a message who it goes to; the Vault's action only.
fn mux_detail(tool: &str, input: &Value) -> Option<String> {
    let name = tool.strip_prefix("mcp__agentmux__")?;
    let action = input.get("action").and_then(Value::as_str).filter(|a| is_word(a))?;
    if name == "Vault" {
        return Some(action.to_string());
    }
    match input.get("to").and_then(Value::as_str).filter(|t| is_word(t)) {
        Some(to) if action == "send" => Some(format!("{action} to {to}")),
        _ => Some(action.to_string()),
    }
}

/// An action word or an agent name: short, no spaces, nothing that carries text.
fn is_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// `https://host/path` without the query or fragment, which is where tokens go.
fn site_of(url: &str) -> String {
    let cut = url.split(['?', '#']).next().unwrap_or(url);
    first_line(cut, PATH_MAX)
}

/// The first line of `s`, at most `max` characters, with the cut marked.
fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > max {
        let mut cut: String = line.chars().take(max).collect();
        cut.push('…');
        cut
    } else {
        line.to_string()
    }
}

/// Whether `s` looks like it holds a secret: a token of a known kind, a long run of token characters, a secret-named
/// variable or flag with a value, or an authorization header.
pub fn looks_secret(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if lower.contains("authorization") || lower.contains("bearer ") || lower.contains("-----begin") {
        return true;
    }
    if names_a_secret(&lower) {
        return true;
    }
    s.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+'))).any(token_looks_secret)
}

/// `NAME=value`, `NAME: value` or `--name value` where the name is a secret's.
fn names_a_secret(lower: &str) -> bool {
    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let (name, has_value) = match word.split_once(['=', ':']) {
            Some((n, v)) => (n, !v.is_empty() || words.get(i + 1).is_some()),
            None if word.starts_with("--") => (*word, words.get(i + 1).is_some_and(|next| !next.starts_with('-'))),
            // A snake_case name such as `aws_secret_access_key` with a value after it.
            None if word.contains('_') && words.get(i + 1).is_some() => (*word, true),
            None => continue,
        };
        if has_value && SECRET_NAMES.iter().any(|s| name.contains(s)) {
            return true;
        }
    }
    false
}

fn token_looks_secret(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if token.len() >= 12 && SECRET_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    // A JWT starts with the encoded header `{"`.
    if token.len() >= 20 && token.starts_with("eyJ") {
        return true;
    }
    if token.len() < 32 || is_uuid(token) {
        return false;
    }
    let (digits, letters) = (token.chars().any(|c| c.is_ascii_digit()), token.chars().any(|c| c.is_ascii_alphabetic()));
    digits && letters
}

fn is_uuid(token: &str) -> bool {
    let parts: Vec<&str> = token.split('-').collect();
    parts.len() == 5 && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_hexdigit()))
}
