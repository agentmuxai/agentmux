// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers a Gemini turn gets: AgentMux's own `agentmux` server and the
//! catalog servers, from the `.mcp.json` the launch wrote in the agent's
//! folder, as a settings file Gemini loads on top of its own.
//! `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §5.
//!
//! The file is per agent, under srv's data folder, and named by the
//! `GEMINI_CLI_SYSTEM_SETTINGS_PATH` variable the turn is launched with. Gemini
//! merges its `mcpServers` with the folder's `.gemini/settings.json` and the
//! user's, so nothing the person set up is replaced.
//!
//! A server's `env` is expanded by Gemini from its own environment, and it
//! hands a server none of the variables named like a key or token unless the
//! server's `env` asks for them. So a `${VAR}` stays a reference, and the
//! `agentmux` server asks for every `AGENTMUX_` variable the agent was launched
//! with by reference, not by value: a key the agent has is not copied into a
//! second file. A header is expanded against the same stripped environment, so
//! a server whose header holds a variable is left out (it would be sent
//! empty) and named in the log.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::super::acp_mcp::{resolve_command, server_kind};
use super::codex_mcp::{env_value, launch_env, scalar};

/// Finds a bare command's absolute path (`resolve_command` in the product).
type Resolve<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The variable that points Gemini at a settings file of the system's.
const SETTINGS_ENV: &str = "GEMINI_CLI_SYSTEM_SETTINGS_PATH";

/// What [`gemini_settings`] made of a `.mcp.json`.
#[derive(Debug, Default, PartialEq)]
pub(super) struct GeminiMcp {
    /// The settings file's text, `None` when no server is to be given.
    pub settings: Option<String>,
    /// Servers left out, with why. Names only; never a value.
    pub skipped: Vec<(String, &'static str)>,
}

/// One server as Gemini's `mcpServers` entry, or why it is left out.
fn server_entry(name: &str, cfg: &Value, kind: &str, agent_env: &HashMap<String, String>, resolve: Resolve) -> Result<Value, &'static str> {
    let x = |v: &str| if v.contains("${") { Err("a command, argument or url holds a variable") } else { Ok(v.to_string()) };
    if kind == "stdio" {
        let command = x(cfg.get("command").and_then(Value::as_str).ok_or("no command")?)?;
        let args = cfg.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(scalar).map(|v| x(&v)).collect::<Result<Vec<_>, _>>()?;
        let mut env: Map<String, Value> = cfg.get("env").and_then(Value::as_object).into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), Value::String(scalar(v)?)))).collect();
        if name == "agentmux" {
            // By reference: a value the agent has is not written to this file.
            let mut own: Vec<&String> = agent_env.keys().filter(|k| k.starts_with("AGENTMUX_")).collect();
            own.sort();
            for k in own {
                env.insert(k.clone(), Value::String(format!("${{{k}}}")));
            }
        }
        let mut entry = json!({ "command": resolve(&command).unwrap_or(command), "args": args });
        if !env.is_empty() {
            entry["env"] = Value::Object(env);
        }
        Ok(entry)
    } else {
        let url = x(cfg.get("url").and_then(Value::as_str).ok_or("no url")?)?;
        let mut headers = Map::new();
        for (header, raw) in cfg.get("headers").and_then(Value::as_object).into_iter().flatten() {
            let Some(raw) = scalar(raw) else { continue };
            if raw.contains("${") {
                return Err("a header holds a variable Gemini would send empty");
            }
            headers.insert(header.clone(), Value::String(raw));
        }
        let mut entry = Map::new();
        // Gemini's `httpUrl` is the streamable-http transport, `url` the sse one.
        entry.insert(if kind == "sse" { "url" } else { "httpUrl" }.to_string(), Value::String(url));
        if !headers.is_empty() {
            entry.insert("headers".to_string(), Value::Object(headers));
        }
        Ok(Value::Object(entry))
    }
}

/// The settings file for the servers in `mcp_json`. `agent_env` is the
/// environment the turn is launched with (see `launch_env`).
pub(super) fn gemini_settings(mcp_json: &str, agent_env: &HashMap<String, String>, resolve: Resolve) -> GeminiMcp {
    let mut result = GeminiMcp::default();
    let Ok(root) = serde_json::from_str::<Value>(mcp_json) else { return result };
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else { return result };
    let mut given = Map::new();
    for (name, cfg) in servers {
        let Some(kind) = server_kind(cfg) else { continue };
        match server_entry(name, cfg, kind, agent_env, resolve) {
            Ok(entry) => {
                given.insert(name.clone(), entry);
            }
            Err(why) => result.skipped.push((name.clone(), why)),
        }
    }
    if !given.is_empty() {
        result.settings = serde_json::to_string_pretty(&json!({ "mcpServers": given })).ok();
    }
    result
}

/// Whether a spawn is a Gemini turn: its command is `gemini`, with or without a
/// folder or an extension (`gemini.cmd` on Windows).
pub(super) fn is_gemini(cli_command: &str) -> bool {
    // Both separators: a Windows path is not split by `Path` on another system.
    let base = cli_command.rsplit(['/', '']).next().unwrap_or(cli_command);
    Path::new(base).file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("gemini"))
}

/// Writes `text` as `<dir>/<block_id>.json` and returns the path. The write is
/// to a temporary file and a rename, and is skipped when the file already holds
/// `text`, so a turn that starts as another one reads the file never sees half.
fn write_settings(dir: &Path, block_id: &str, text: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let name: String = block_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let path = dir.join(format!("{name}.json"));
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
        let tmp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("write {}: {e}", path.display())
        })?;
    }
    Ok(path)
}

impl super::SubprocessController {
    /// What this turn adds for MCP when it is a Gemini turn on the host: the
    /// variable that points Gemini at the settings file made from the folder's
    /// `.mcp.json`; else nothing.
    pub(super) fn gemini_mcp(&self, config: &super::SubprocessSpawnConfig) -> Vec<(String, String)> {
        if !is_gemini(&config.cli_command) || config.working_dir.is_empty() {
            return Vec::new();
        }
        let folder = crate::backend::blockcontroller::core::expand_home_dir(&config.working_dir);
        let Ok(raw) = std::fs::read_to_string(Path::new(&folder).join(".mcp.json")) else { return Vec::new() };
        let path = config.env_vars.get("PATH").cloned();
        let env = launch_env(std::env::vars(), &config.env_vars);
        if env_value(&env, SETTINGS_ENV).is_some() {
            tracing::warn!(block_id = %self.block_id, "this Gemini turn already has {SETTINGS_ENV} set, so its MCP servers are not given");
            return Vec::new();
        }
        let made = gemini_settings(&raw, &env, &|c| resolve_command(c, path.as_deref()));
        for (name, why) in &made.skipped {
            tracing::warn!(block_id = %self.block_id, server = %name, why = %why, "MCP server not given to this Gemini turn");
        }
        let Some(text) = made.settings else { return Vec::new() };
        let dir = crate::backend::base::get_mux_data_dir().join("gemini-mcp");
        match write_settings(&dir, &self.block_id, &text) {
            Ok(file) => vec![(SETTINGS_ENV.to_string(), file.to_string_lossy().into_owned())],
            Err(e) => {
                tracing::warn!(block_id = %self.block_id, error = %e, "Gemini MCP settings not written; this turn has no MCP servers from .mcp.json");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
#[path = "gemini_mcp_tests.rs"]
mod tests;
