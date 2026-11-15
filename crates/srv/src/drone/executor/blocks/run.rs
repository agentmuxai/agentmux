// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Run block: runs a fixed command line in a container
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.6).
//!
//! Block config (`node.data`):
//!   * `command`  required, a list of strings: the program and its arguments. It is an argv, run
//!                directly, never through a shell: no pipes, no expansion.
//!   * `env`      optional, an object of environment variables.
//!   * `cwd`      optional, a directory inside the container.
//!   * `image`    optional, the container image; this AgentMux's base image when absent.
//!   * `where`    `"container"` (the default). `"host"` is refused: it needs a named allowance that
//!                this build has no way to give.
//!   * `ok_exit_codes`  optional, the exit codes that are not a failure (`[0]` when absent).
//!   * `timeout_seconds`  how long it may run: 5 minutes when absent, at most 60 minutes.
//!
//! **The command line is never built from outside text.** Every `{{reference}}` in `command`, `env`
//! and `cwd` must read a clean value (written in the graph, or a number or id made of it); one that
//! reads text from outside the drone (an event, a web response, an agent's answer, or a number
//! worked out from one) is refused when the block runs.
//!
//! The container has no network, no mounts, a read-only root, no capabilities and limits on
//! memory, processes and CPU (`backend::container::oneshot`). Mounts, network hosts and secrets are
//! not supported yet: a block that asks for them is refused, so none is given by mistake.
//!
//! Output: `{ "exit_code", "stdout", "stderr", "duration_ms", "truncated" }`, with stdout and
//! stderr cut at 64 KB. All of it is outside text (the block's output is tainted). An exit code
//! that is not in `ok_exit_codes` fails the block.

use serde_json::{json, Value};

use crate::backend::container::image::{resolve_container_image, DEFAULT_AGENT_IMAGE};
use crate::backend::container::oneshot::{OneShot, OneShotOutcome};
use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::FlowNode;

/// The longest a Run block may be given, in seconds.
pub const MAX_TIMEOUT_SECONDS: u64 = 3_600;
/// How much of stderr is put in the error of a block that failed.
const ERROR_TAIL_CHARS: usize = 300;
/// What a block asking for these gets, until they are supported.
const NOT_YET: [&str; 6] = ["mounts", "hosts", "network", "vault", "secrets", "stdin"];

/// A command, with its references still to fill in.
#[derive(Debug, PartialEq)]
pub(crate) struct RunSpec {
    pub image: Option<String>,
    pub command: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub ok_exit_codes: Vec<i64>,
}

/// The block's settings, as written. Pure: nothing is filled in or run.
pub(crate) fn parse(node: &FlowNode) -> Result<RunSpec, String> {
    let data = &node.data;
    for key in NOT_YET {
        if data.get(key).is_some_and(|v| !v.is_null()) {
            return Err(format!("run block: `{key}` is not supported yet: the command runs with no network, no mounts and no secrets"));
        }
    }
    match data.get("where").and_then(Value::as_str).map(str::trim) {
        None | Some("") | Some("container") => {}
        Some("host") => return Err("run block: `where: host` needs a named allowance that this build can't give yet; use `where: container`".to_string()),
        Some(other) => return Err(format!("run block: `where` is `container` or `host`, not `{other}`")),
    }
    let command: Vec<String> = data
        .get("command")
        .and_then(Value::as_array)
        .ok_or("run block: `command` must be a list, the program and then its arguments")?
        .iter()
        .map(|a| a.as_str().map(str::to_string).ok_or("run block: every item of `command` must be text"))
        .collect::<Result<_, _>>()?;
    if command.first().map_or(true, |program| program.trim().is_empty()) {
        return Err("run block: `command` needs a program as its first item".to_string());
    }
    let env = match data.get("env") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                let valid = !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit());
                match (valid, v.as_str()) {
                    (true, Some(text)) => Ok((k.clone(), text.to_string())),
                    (false, _) => Err(format!("run block: `{k}` is not a name an environment variable can have")),
                    (_, None) => Err(format!("run block: `env.{k}` must be text")),
                }
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("run block: `env` must be an object of names and text".to_string()),
    };
    let cwd = match data.get("cwd") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => return Err("run block: `cwd` must be text".to_string()),
    };
    let image = match data.get("image") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) if s.contains("{{") => return Err("run block: `image` must be written in the graph, not a {{reference}}".to_string()),
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => return Err("run block: `image` must be text".to_string()),
    };
    let ok_exit_codes = match data.get("ok_exit_codes") {
        None | Some(Value::Null) => vec![0],
        Some(Value::Array(items)) if !items.is_empty() => {
            items.iter().map(|c| c.as_i64().ok_or("run block: `ok_exit_codes` must be a list of whole numbers")).collect::<Result<_, _>>()?
        }
        Some(_) => return Err("run block: `ok_exit_codes` must be a list of whole numbers".to_string()),
    };
    if let Some(secs) = data.get("timeout_seconds").and_then(Value::as_u64) {
        if secs > MAX_TIMEOUT_SECONDS {
            return Err(format!("run block: `timeout_seconds` is at most {MAX_TIMEOUT_SECONDS} (60 minutes)"));
        }
    }
    Ok(RunSpec { image, command, env, cwd, ok_exit_codes })
}

/// The block is set rightly. Called when a graph is checked.
pub fn check(node: &FlowNode) -> Result<(), String> {
    parse(node).map(|_| ())
}

/// What to run, with every reference filled in; refused when one reads outside text.
pub(crate) fn compose(node: &FlowNode, scope: &ExecutionScope) -> Result<(OneShot, Vec<i64>), String> {
    let spec = parse(node)?;
    let clean = |what: String, template: &str| -> Result<String, String> {
        let (tainted, derived) = scope.bits_of(template);
        if tainted || derived {
            let why = if tainted { "text from outside the drone" } else { "a value worked out from text from outside the drone" };
            return Err(format!("run block: {what} reads {why}; a command line is never built from outside text"));
        }
        Ok(scope.resolve(template))
    };
    let mut argv = Vec::with_capacity(spec.command.len());
    for (i, arg) in spec.command.iter().enumerate() {
        argv.push(clean(format!("`command[{i}]`"), arg)?);
    }
    if argv[0].trim().is_empty() {
        return Err("run block: the program is empty once its references are filled in".to_string());
    }
    let mut env = Vec::with_capacity(spec.env.len());
    for (name, value) in &spec.env {
        env.push((name.clone(), clean(format!("`env.{name}`"), value)?));
    }
    let cwd = spec.cwd.as_deref().map(|c| clean("`cwd`".to_string(), c)).transpose()?;
    let image = spec.image.unwrap_or_else(|| resolve_container_image(DEFAULT_AGENT_IMAGE));
    Ok((OneShot { image, argv, env, cwd }, spec.ok_exit_codes))
}

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let (one, ok_codes) = compose(node, scope)?;
    let state = crate::drone::host::state()?;
    let manager = state.container_manager.get_or_explain().await.map_err(|e| format!("run block: {e}"))?;
    let outcome = manager.run_oneshot(&one).await.map_err(|e| format!("run block: {e}"))?;
    outcome_of(&outcome, &ok_codes)
}

/// What the command came to, as the block's output; a code that is not an ok one fails the block.
pub(crate) fn outcome_of(done: &OneShotOutcome, ok_codes: &[i64]) -> Result<Value, String> {
    if !ok_codes.contains(&done.exit_code) {
        let tail = tail_line(&done.stderr);
        return Err(if tail.is_empty() { format!("run block: the command exited with code {}", done.exit_code) } else { format!("run block: the command exited with code {}: {tail}", done.exit_code) });
    }
    Ok(json!({
        "exit_code": done.exit_code,
        "stdout": done.stdout,
        "stderr": done.stderr,
        "duration_ms": done.duration_ms,
        "truncated": done.truncated,
    }))
}

/// The end of `text` on one line.
fn tail_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = flat.chars().count();
    if count <= ERROR_TAIL_CHARS {
        return flat;
    }
    let tail: String = flat.chars().skip(count - ERROR_TAIL_CHARS).collect();
    format!("…{tail}")
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
