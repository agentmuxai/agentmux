// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP servers an ACP agent gets in `session/new` and `session/load`: the
//! ones in its `.mcp.json` (AgentMux's own `agentmux` server and the user's),
//! in ACP's shape. SPEC_NINE_NEW_HARNESSES_2026_10_14.md §5 item 3.

use serde_json::{json, Value};

/// How a `.mcp.json` server is reached: `"stdio"`, `"http"` or `"sse"`. `None`
/// for one that is switched off (`disabled`) or of a type no agent takes.
pub(super) fn server_kind(cfg: &Value) -> Option<&'static str> {
    if cfg.get("disabled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    match cfg.get("type").and_then(Value::as_str) {
        Some("http" | "streamable-http" | "streamableHttp") => Some("http"),
        Some("sse") => Some("sse"),
        Some("stdio") => Some("stdio"),
        Some(_) => None,
        None if cfg.get("url").is_some() => Some("http"),
        None => Some("stdio"),
    }
}

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
        let Some(kind) = server_kind(cfg) else { continue };
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

/// Give AgentMux's own MCP server (`agentmux-mcp`) the `AGENTMUX_*` variables of the agent's environment
/// that its entry does not set. Claude Code starts an MCP server with the agent's own environment, so
/// `agentmux-mcp` finds `AGENTMUX_AUTH_KEY` there; an ACP agent that starts it with only the `env` of its
/// `session/new` (dsh) left it without the key, and every call was refused, `Pane` `whoami` included.
/// Entries the server already has (the identity and signing keys of `.mcp.json`) win; other servers are
/// untouched. The agent holds these values itself, so nothing new reaches it.
pub(super) fn carry_agentmux_env(servers: &mut [Value], agent_env: &std::collections::HashMap<String, String>) {
    for server in servers.iter_mut() {
        let is_agentmux_mcp = server
            .get("command")
            .and_then(Value::as_str)
            .and_then(|c| std::path::Path::new(c).file_stem())
            .is_some_and(|stem| stem == "agentmux-mcp");
        let Some(env) = server.get_mut("env").and_then(Value::as_array_mut) else { continue };
        if !is_agentmux_mcp {
            continue;
        }
        let mut extra: Vec<(&String, &String)> = agent_env
            .iter()
            .filter(|(k, v)| k.starts_with("AGENTMUX_") && !v.is_empty())
            .filter(|(k, _)| !env.iter().any(|e| e.get("name").and_then(Value::as_str) == Some(k.as_str())))
            .collect();
        extra.sort();
        env.extend(extra.into_iter().map(|(k, v)| json!({ "name": k, "value": v })));
    }
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
/// else AgentMux's own tool dirs (bundled `<srv-dir>/tools/bin`, where a
/// packaged build ships `agentmux-mcp`, then the user tools dir `task dev`
/// uses), else beside srv, else the first on `path` (the agent's PATH, srv's
/// when the agent sets none).
pub(super) fn resolve_command(cmd: &str, path: Option<&str>) -> Option<String> {
    if std::path::Path::new(cmd).is_absolute() {
        return Some(cmd.to_string());
    }
    use crate::backend::agent_tools::{find_in_tool_dirs, ToolDir};
    let in_tool_dirs = find_in_tool_dirs(cmd, &[ToolDir::Bundled, ToolDir::User, ToolDir::BesideSrv]);
    let on_path = || {
        let path = path.map(str::to_string).or_else(|| std::env::var("PATH").ok())?;
        which::which_in(cmd, Some(path), std::env::current_dir().ok()?).ok()
    };
    in_tool_dirs.or_else(on_path).map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_env() -> std::collections::HashMap<String, String> {
        [
            ("AGENTMUX_AUTH_KEY", "the-key"),
            ("AGENTMUX_BLOCKID", "blk-1"),
            ("AGENTMUX_AGENT_ID", "from-process"),
            ("AGENTMUX_EMPTY", ""),
            ("PATH", "/bin"),
            ("AWS_PROFILE", "p"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn env_of(server: &Value) -> Vec<(String, String)> {
        server["env"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["name"].as_str().unwrap().to_string(), e["value"].as_str().unwrap().to_string()))
            .collect()
    }

    #[test]
    fn agentmux_mcp_gets_the_agents_agentmux_variables_it_lacks() {
        let mut servers = vec![json!({
            "name": "agentmux", "command": "C:/AgentMux/agentmux-mcp.exe", "args": [],
            "env": [{ "name": "AGENTMUX_AGENT_ID", "value": "deephar" }, { "name": "AGENTMUX_JEKT_KEY", "value": "j" }],
        })];
        carry_agentmux_env(&mut servers, &agent_env());
        let env = env_of(&servers[0]);
        assert!(env.contains(&("AGENTMUX_AUTH_KEY".into(), "the-key".into())));
        assert!(env.contains(&("AGENTMUX_BLOCKID".into(), "blk-1".into())));
        // The entry's own values win, and nothing is listed twice.
        assert!(env.contains(&("AGENTMUX_AGENT_ID".into(), "deephar".into())));
        assert_eq!(env.iter().filter(|(k, _)| k == "AGENTMUX_AGENT_ID").count(), 1);
        assert!(env.contains(&("AGENTMUX_JEKT_KEY".into(), "j".into())));
    }

    #[test]
    fn only_agentmux_variables_and_only_non_empty_ones() {
        let mut servers = vec![json!({ "name": "agentmux", "command": "/t/agentmux-mcp", "args": [], "env": [] })];
        carry_agentmux_env(&mut servers, &agent_env());
        let names: Vec<String> = env_of(&servers[0]).into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, vec!["AGENTMUX_AGENT_ID", "AGENTMUX_AUTH_KEY", "AGENTMUX_BLOCKID"]);
    }

    #[test]
    fn another_server_is_left_alone() {
        let other = json!({ "name": "docs", "command": "/t/docs-mcp", "args": [], "env": [] });
        let mut servers = vec![other.clone()];
        carry_agentmux_env(&mut servers, &agent_env());
        assert_eq!(servers[0], other);
        // An http server has no env list at all.
        let mut http = vec![json!({ "type": "http", "name": "x", "url": "https://x", "headers": [] })];
        let before = http.clone();
        carry_agentmux_env(&mut http, &agent_env());
        assert_eq!(http, before);
    }

    #[test]
    fn a_bare_command_name_counts_too() {
        let mut servers = vec![json!({ "name": "agentmux", "command": "agentmux-mcp", "args": [], "env": [] })];
        carry_agentmux_env(&mut servers, &agent_env());
        assert!(!env_of(&servers[0]).is_empty());
    }

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
        assert_eq!(resolve_command(given, None).as_deref(), Some(given));
    }

    #[test]
    fn a_bare_command_is_found_on_the_agents_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("acp-mcp-probe-tool{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(dir.path().join(&name), b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path().join(&name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = dir.path().to_string_lossy().into_owned();
        let found = resolve_command("acp-mcp-probe-tool", Some(&path)).unwrap();
        assert_eq!(std::path::Path::new(&found), dir.path().join(&name));
        assert_eq!(resolve_command("acp-mcp-probe-tool-missing", Some(&path)), None);
    }
}
