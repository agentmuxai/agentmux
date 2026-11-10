// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers a Codex `exec` turn gets: AgentMux's own `agentmux` server
//! and the catalog servers, from the `.mcp.json` the launch wrote in the
//! agent's folder, as `-c mcp_servers.<name>.<key>=<toml>` overrides.
//! `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §5.
//!
//! The overrides are on the command line, which other processes on the machine
//! can read, so no secret goes in them. A value in a server's `env` or
//! `headers`, where a key lives, is not written there: it travels in the
//! environment the Codex process is launched with, and the server asks Codex to
//! forward it by NAME (`env_vars`, `env_http_headers`, `bearer_token_env_var`).
//! A `${VAR}` that is the whole of such a value is forwarded the same way, from
//! the environment the agent was launched with. A command, argument or url is
//! written as it is in `.mcp.json`; a server with a variable in one, or with a
//! variable mixed into other text in an env or header value, is left out and
//! named in the log, never written with the variable's value.

use std::collections::HashMap;

use serde_json::Value;

use super::super::acp_mcp::{resolve_command, server_kind};

/// Finds a bare command's absolute path (`resolve_command` in the product).
type Resolve<'a> = &'a dyn Fn(&str) -> Option<String>;

/// What [`codex_mcp_overrides`] made of a `.mcp.json`.
#[derive(Debug, Default, PartialEq)]
pub(super) struct CodexMcp {
    /// `-c`, `key=value` pairs to put after `exec`.
    pub overrides: Vec<String>,
    /// Variables to launch Codex with, which the servers' `env_vars` and
    /// `env_http_headers` forward by name.
    pub env: Vec<(String, String)>,
    /// Servers left out, with why. Names only; never a value.
    pub skipped: Vec<(String, &'static str)>,
}

/// A TOML basic string. JSON's string escapes are valid TOML ones.
fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        toml_str(k)
    }
}

fn toml_array(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|i| toml_str(i)).collect::<Vec<_>>().join(","))
}

fn toml_table(pairs: &[(String, String)]) -> String {
    format!("{{{}}}", pairs.iter().map(|(k, v)| format!("{}={}", toml_key(k), toml_str(v))).collect::<Vec<_>>().join(","))
}

/// Codex accepts a server name of letters, digits, `_` and `-` only.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The text of a JSON string, number or bool value; `None` for anything else.
pub(super) fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(v.to_string()),
        _ => None,
    }
}

/// `${NAME}` when `raw` is exactly that.
fn whole_placeholder(raw: &str) -> Option<&str> {
    raw.strip_prefix("${")?.strip_suffix('}').filter(|n| !n.is_empty() && !n.contains(['$', '{', '}', ':']))
}

/// What one server needs: its overrides, and the variables the Codex process
/// must be launched with for `env_vars` and `env_http_headers` to forward.
#[derive(Default)]
struct Plan {
    overrides: Vec<(String, String)>,
    env: Vec<(String, String)>,
}

/// A name Codex can forward from its own environment.
fn valid_env_name(k: &str) -> bool {
    let mut chars = k.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One server's plan, or why it can't be given without a secret on the
/// command line. `slot` numbers the server among those given, for the names of
/// the variables its literal header values travel in.
fn server_plan(
    name: &str,
    cfg: &Value,
    kind: &str,
    slot: usize,
    agent_env: &HashMap<String, String>,
    resolve: Resolve,
) -> Result<Plan, &'static str> {
    // A command, argument or url is written as it is, so one that holds a
    // variable can't be given without putting its value on the command line.
    let x = |v: &str| if v.contains("${") { Err("a command, argument or url holds a variable") } else { Ok(v.to_string()) };
    let mut plan = Plan::default();
    let mut put = |key: &str, value: String| plan.overrides.push((format!("mcp_servers.{name}.{key}"), value));
    if kind == "stdio" {
        let command = x(cfg.get("command").and_then(Value::as_str).ok_or("no command")?)?;
        put("command", toml_str(&resolve(&command).unwrap_or(command)));
        let args = cfg.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(scalar).map(|v| x(&v)).collect::<Result<Vec<_>, _>>()?;
        put("args", toml_array(&args));
        let (set_env, forwarded) = split_env(cfg.get("env"), name == "agentmux", agent_env)?;
        if !forwarded.is_empty() {
            put("env_vars", toml_array(&forwarded));
        }
        plan.env = set_env;
    } else {
        let url = x(cfg.get("url").and_then(Value::as_str).ok_or("no url")?)?;
        put("url", toml_str(&url));
        let (from_env, bearer, set_env) = split_headers(cfg.get("headers"), slot)?;
        if !from_env.is_empty() {
            put("env_http_headers", toml_table(&from_env));
        }
        if let Some(var) = bearer {
            put("bearer_token_env_var", toml_str(&var));
        }
        plan.env = set_env;
    }
    Ok(plan)
}

/// A stdio server's `env`: the variables to launch Codex with (a literal value
/// travels in the environment too, since an `env` table would put it on the
/// command line) and the names Codex forwards to the server. For `agentmux`,
/// also every `AGENTMUX_` variable the agent was launched with (its endpoint,
/// keys and identity), since Codex gives a server only a small set of its own;
/// the agent's own value of such a variable wins over a copy in `.mcp.json`.
#[allow(clippy::type_complexity)]
fn split_env(env: Option<&Value>, is_agentmux: bool, agent_env: &HashMap<String, String>) -> Result<(Vec<(String, String)>, Vec<String>), &'static str> {
    let mut set_env = Vec::new();
    let mut forwarded: Vec<String> = Vec::new();
    for (key, raw) in env.and_then(Value::as_object).into_iter().flatten() {
        let Some(raw) = scalar(raw) else { continue };
        if !valid_env_name(key) {
            return Err("an env name Codex can't forward");
        }
        if !raw.contains("${") {
            let own = is_agentmux && key.starts_with("AGENTMUX_") && agent_env.contains_key(key);
            if !own {
                set_env.push((key.clone(), raw));
            }
            forwarded.push(key.clone());
        } else if let Some(source) = whole_placeholder(&raw) {
            // The server's `KEY` is read from the variable `source`. Codex forwards a
            // name as it is, so another source's value is staged under `KEY`; a
            // source the turn does not have gives the server no `KEY`.
            if source != key {
                if let Some(value) = env_value(agent_env, source) {
                    set_env.push((key.clone(), value.clone()));
                    forwarded.push(key.clone());
                }
            } else {
                forwarded.push(key.clone());
            }
        } else {
            return Err("an env value mixes a variable into other text");
        }
    }
    if is_agentmux {
        let mut names: Vec<&String> = agent_env.keys().filter(|k| k.starts_with("AGENTMUX_") && !forwarded.contains(k)).collect();
        names.sort();
        forwarded.extend(names.into_iter().cloned());
    }
    Ok((set_env, forwarded))
}

/// An http server's `headers`: the headers Codex reads from a variable, the
/// variable behind `Authorization: Bearer ${VAR}`, and the variables to launch
/// Codex with for the literal headers (a header value is as likely a key as an
/// env value is, so it never goes on the command line either).
#[allow(clippy::type_complexity)]
fn split_headers(headers: Option<&Value>, slot: usize) -> Result<(Vec<(String, String)>, Option<String>, Vec<(String, String)>), &'static str> {
    let (mut from_env, mut bearer, mut set_env) = (Vec::new(), None, Vec::new());
    for (i, (header, raw)) in headers.and_then(Value::as_object).into_iter().flatten().enumerate() {
        let Some(raw) = scalar(raw) else { continue };
        if !raw.contains("${") {
            let var = format!("AGENTMUX_MCP_S{slot}_H{i}");
            from_env.push((header.clone(), var.clone()));
            set_env.push((var, raw));
        } else if let Some(var) = whole_placeholder(&raw) {
            from_env.push((header.clone(), var.to_string()));
        } else if let Some(var) = header.eq_ignore_ascii_case("authorization").then(|| raw.strip_prefix("Bearer ")).flatten().and_then(whole_placeholder) {
            bearer = Some(var.to_string());
        } else {
            return Err("a header value mixes a variable into other text");
        }
    }
    Ok((from_env, bearer, set_env))
}

/// The `-c` overrides for the servers in `mcp_json`, and the variables to
/// launch Codex with for them. `agent_env` is the environment the turn is
/// launched with (see [`launch_env`]). A server of a kind Codex can't take
/// (`sse`), one that is switched off, or one whose name Codex refuses is left
/// out; so is one that would need a secret on the command line, or sets a
/// variable Codex already has to another value.
pub(super) fn codex_mcp_overrides(mcp_json: &str, agent_env: &HashMap<String, String>, resolve: Resolve) -> CodexMcp {
    let mut result = CodexMcp::default();
    let Ok(root) = serde_json::from_str::<Value>(mcp_json) else { return result };
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else { return result };
    let mut given = 0;
    for (name, cfg) in servers {
        let Some(kind) = server_kind(cfg) else { continue };
        let plan = if kind == "sse" {
            Err("Codex takes no sse server")
        } else if !valid_name(name) {
            Err("Codex takes only letters, digits, _ and - in a server name")
        } else {
            server_plan(name, cfg, kind, given, agent_env, resolve).and_then(|plan| {
                let taken = |k: &String, v: &String| env_value(agent_env, k).is_some_and(|o| o != v) || result.env.iter().any(|(rk, rv)| rk.eq_ignore_ascii_case(k) && rv != v);
                if plan.env.iter().any(|(k, v)| taken(k, v)) { Err("an env name is already set to another value for this turn") } else { Ok(plan) }
            })
        };
        match plan {
            Ok(plan) => {
                given += 1;
                for (key, value) in plan.overrides {
                    result.overrides.push("-c".to_string());
                    result.overrides.push(format!("{key}={value}"));
                }
                result.env.extend(plan.env);
            }
            Err(why) => result.skipped.push((name.clone(), why)),
        }
    }
    result
}

/// `argv` with `overrides` right after the `exec` subcommand, where the other
/// provider flags are; unchanged when there is none or no `exec`.
pub(super) fn with_overrides(argv: &[String], overrides: &[String]) -> Vec<String> {
    let mut out = argv.to_vec();
    if let Some(at) = out.iter().position(|a| a == "exec") {
        out.splice(at + 1..at + 1, overrides.iter().cloned());
    }
    out
}

/// The environment the Codex process starts with: srv's own, except the
/// AgentMux variables a pane must not inherit (`pane_env`), then the turn's
/// own, which win. The servers' forwarding needs it for two things: Codex gives
/// a server only the names it is told to forward, and `agentmux-mcp` needs
/// srv's endpoint, which is inherited, not in the turn's own variables; and a
/// server's literal env value must not replace a variable Codex already has.
pub(super) fn launch_env(inherited: impl Iterator<Item = (String, String)>, own: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = inherited.filter(|(k, _)| !k.starts_with("AGENTMUX") || !crate::backend::pane_env::should_strip(k)).collect();
    env.extend(own.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

/// The value `env` has for `name`; a Windows name is matched without regard to case.
pub(super) fn env_value<'a>(env: &'a HashMap<String, String>, name: &str) -> Option<&'a String> {
    if cfg!(windows) { env.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v) } else { env.get(name) }
}

/// Launch the Codex process with the variables its servers forward by name.
/// Set as they are, after the agent's own, which [`codex_mcp_overrides`] has
/// checked they do not contradict.
pub(super) fn apply_env(cmd: &mut tokio::process::Command, env: &[(String, String)]) {
    for (k, v) in env {
        cmd.env(k, v);
    }
}

impl super::SubprocessController {
    /// What this turn adds for MCP: those of the `.mcp.json` in the turn's
    /// folder when it is a Codex `exec` turn on the host, else nothing.
    pub(super) fn codex_mcp(&self, config: &super::SubprocessSpawnConfig) -> CodexMcp {
        if config.resume_strategy != "codex-exec" || config.working_dir.is_empty() {
            return CodexMcp::default();
        }
        // The folder as the spawn expands it (`~` is the home directory). The config may be
        // outside it (#3680); `read_for_turn` finds it and keeps the project's own servers.
        let folder = crate::backend::blockcontroller::core::expand_home_dir(&config.working_dir);
        let Some(raw) = crate::backend::mcp_config_file::read_for_turn(&folder) else { return CodexMcp::default() };
        let path = config.env_vars.get("PATH").cloned();
        let env = launch_env(std::env::vars(), &config.env_vars);
        let made = codex_mcp_overrides(&raw, &env, &|c| resolve_command(c, path.as_deref()));
        for (name, why) in &made.skipped {
            tracing::warn!(block_id = %self.block_id, server = %name, why = %why, "MCP server not given to this Codex turn");
        }
        made
    }
}

#[cfg(test)]
#[path = "codex_mcp_tests.rs"]
mod tests;
