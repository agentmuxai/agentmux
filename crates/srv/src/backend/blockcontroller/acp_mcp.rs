// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers an ACP agent gets in `session/new` and `session/load`: the
//! ones in its `.mcp.json` (AgentMux's own `agentmux` server and the user's),
//! in ACP's shape. SPEC_NINE_NEW_HARNESSES_2026_10_14.md §5 item 3.

use serde_json::{json, Value};

/// `.mcp.json`'s `mcpServers` as ACP `McpServer`s. Every ACP agent takes stdio
/// servers; http and sse ones go only to an agent whose `initialize` said it
/// takes them (`mcpCapabilities`). `${VAR}` and `${VAR:-default}` in a command,
/// args, env, URL or headers are expanded with `lookup` (the agent's
/// environment), as Claude Code does for `.mcp.json`. ACP wants a stdio
/// command as an absolute path, so a bare one goes through `resolve`; one it
/// can't find keeps its name.
pub(super) fn acp_mcp_servers(
    mcp_json: &str,
    http: bool,
    sse: bool,
    resolve: impl Fn(&str) -> Option<String>,
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<Value> {
    let x = |v: &str| expand_vars(v, &lookup);
    let Ok(v) = serde_json::from_str::<Value>(mcp_json) else { return Vec::new() };
    let Some(servers) = v.get("mcpServers").and_then(Value::as_object) else { return Vec::new() };
    let mut out = Vec::new();
    for (name, cfg) in servers {
        if cfg.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let kind = match cfg.get("type").and_then(Value::as_str) {
            Some("http" | "streamable-http" | "streamableHttp") => "http",
            Some("sse") => "sse",
            Some("stdio") => "stdio",
            Some(_) => continue,
            None if cfg.get("url").is_some() => "http",
            None => "stdio",
        };
        if kind == "stdio" {
            let Some(command) = cfg.get("command").and_then(Value::as_str).map(x) else { continue };
            let args: Vec<String> = cfg
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(x).collect())
                .unwrap_or_default();
            out.push(json!({
                "name": name,
                "command": resolve(&command).unwrap_or(command),
                "args": args,
                "env": name_values(cfg.get("env"), &x),
            }));
        } else if (kind == "http" && http) || (kind == "sse" && sse) {
            let Some(url) = cfg.get("url").and_then(Value::as_str).map(x) else { continue };
            out.push(json!({ "type": kind, "name": name, "url": url, "headers": name_values(cfg.get("headers"), &x) }));
        }
    }
    out
}

/// A `{ "K": "v" }` object as ACP's `[{ "name": "K", "value": "v" }]`, each value
/// through `x` (placeholder expansion).
fn name_values(obj: Option<&Value>, x: &impl Fn(&str) -> String) -> Vec<Value> {
    obj.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| {
                    let value = match v {
                        Value::String(s) => x(s),
                        Value::Number(_) | Value::Bool(_) => v.to_string(),
                        _ => return None,
                    };
                    Some(json!({ "name": k, "value": value }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `${VAR}` and `${VAR:-default}` in `s` replaced from `lookup`. A variable that
/// is unset and has no default stays as written, as does anything that isn't a
/// complete placeholder.
fn expand_vars(s: &str, lookup: &impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let (name, default) = match inner.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (inner, None),
        };
        match lookup(name).or_else(|| default.map(str::to_string)) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + 2 + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// An absolute path for a stdio server's command: as given when it is one,
/// else the copy beside srv's own executable (where AgentMux ships
/// `agentmux-mcp`), else the first on srv's PATH.
pub(super) fn resolve_command(cmd: &str) -> Option<String> {
    if std::path::Path::new(cmd).is_absolute() {
        return Some(cmd.to_string());
    }
    let beside_srv = std::env::current_exe().ok().and_then(|exe| {
        let dir = exe.parent()?.to_path_buf();
        [format!("{cmd}{}", std::env::consts::EXE_SUFFIX), cmd.to_string()]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file())
    });
    beside_srv
        .or_else(|| which::which(cmd).ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(cmd: &str) -> Option<String> {
        (cmd == "agentmux-mcp").then(|| "C:/AgentMux/agentmux-mcp.exe".to_string())
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    const MCP: &str = r#"{"mcpServers":{
        "agentmux":{"type":"stdio","command":"agentmux-mcp","args":[],"env":{"AGENTMUX_AGENT_ID":"aria","N":3}},
        "git":{"command":"uvx","args":["mcp-server-git", 7]},
        "docs":{"type":"http","url":"https://example.com/mcp","headers":{"Authorization":"Bearer t"}},
        "feed":{"type":"sse","url":"https://example.com/sse"},
        "off":{"command":"x","disabled":true},
        "odd":{"type":"websocket","url":"wss://example.com"}
    }}"#;

    #[test]
    fn stdio_servers_go_to_every_agent_with_absolute_commands_and_env_as_pairs() {
        let s = acp_mcp_servers(MCP, false, false, abs, no_env);
        assert_eq!(s.len(), 2);
        let agentmux = s.iter().find(|v| v["name"] == "agentmux").unwrap();
        assert_eq!(
            *agentmux,
            json!({
                "name": "agentmux",
                "command": "C:/AgentMux/agentmux-mcp.exe",
                "args": [],
                "env": [{"name": "AGENTMUX_AGENT_ID", "value": "aria"}, {"name": "N", "value": "3"}],
            })
        );
        // No type field on a stdio server; a command `resolve` can't find keeps its name.
        let git = s.iter().find(|v| v["name"] == "git").unwrap();
        assert_eq!(*git, json!({"name": "git", "command": "uvx", "args": ["mcp-server-git"], "env": []}));
    }

    #[test]
    fn http_and_sse_servers_go_only_to_an_agent_that_takes_them() {
        let names = |http, sse| {
            let mut n: Vec<String> =
                acp_mcp_servers(MCP, http, sse, abs, no_env).iter().map(|v| v["name"].as_str().unwrap().to_string()).collect();
            n.sort();
            n
        };
        assert_eq!(names(true, false), ["agentmux", "docs", "git"]);
        assert_eq!(names(false, true), ["agentmux", "feed", "git"]);
        let docs = acp_mcp_servers(MCP, true, false, abs, no_env).into_iter().find(|v| v["name"] == "docs").unwrap();
        assert_eq!(
            docs,
            json!({"type": "http", "name": "docs", "url": "https://example.com/mcp",
                   "headers": [{"name": "Authorization", "value": "Bearer t"}]})
        );
    }

    #[test]
    fn a_missing_or_broken_mcp_json_gives_no_servers() {
        assert!(acp_mcp_servers("", true, true, abs, no_env).is_empty());
        assert!(acp_mcp_servers("not json", true, true, abs, no_env).is_empty());
        assert!(acp_mcp_servers(r#"{"other":1}"#, true, true, abs, no_env).is_empty());
    }

    #[test]
    fn placeholders_expand_from_the_agents_environment() {
        let env = |k: &str| match k {
            "TOKEN" => Some("t0k".to_string()),
            "TOOLS" => Some("C:/tools".to_string()),
            _ => None,
        };
        let raw = r#"{"mcpServers":{
            "s":{"command":"${TOOLS}/srv.exe","args":["--port=${PORT:-8080}","${MISSING}"],"env":{"T":"${TOKEN}"}},
            "h":{"type":"http","url":"https://${HOST:-example.com}/mcp","headers":{"Authorization":"Bearer ${TOKEN}"}}
        }}"#;
        let s = acp_mcp_servers(raw, true, false, |_| None, env);
        let stdio = s.iter().find(|v| v["name"] == "s").unwrap();
        assert_eq!(stdio["command"], "C:/tools/srv.exe");
        // A default when unset; an unset variable with none stays as written.
        assert_eq!(stdio["args"], json!(["--port=8080", "${MISSING}"]));
        assert_eq!(stdio["env"], json!([{"name": "T", "value": "t0k"}]));
        let http = s.iter().find(|v| v["name"] == "h").unwrap();
        assert_eq!(http["url"], "https://example.com/mcp");
        assert_eq!(http["headers"], json!([{"name": "Authorization", "value": "Bearer t0k"}]));
        // Not a complete placeholder: left alone.
        assert_eq!(expand_vars("a${b", &env), "a${b");
        assert_eq!(expand_vars("$TOKEN ${TOKEN}", &env), "$TOKEN t0k");
    }

    #[test]
    fn an_absolute_command_is_kept_as_given() {
        let given = if cfg!(windows) { r"C:\tools\x.exe" } else { "/usr/bin/x" };
        assert_eq!(resolve_command(given).as_deref(), Some(given));
    }
}
