use super::*;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// A fixed answer for the one command the tests resolve, so they do not depend on the machine's PATH.
fn abs(cmd: &str) -> Option<String> {
    (cmd == "uvx").then(|| "/bin/uvx".to_string())
}

const MCP: &str = r#"{"mcpServers":{
    "agentmux":{"type":"stdio","command":"agentmux-mcp","args":[],"env":{"AGENTMUX_AGENT_ID":"aria","AGENTMUX_JEKT_KEY":"literal-key"}},
    "git":{"command":"uvx","args":["mcp-server-git","--repo","C:\\work"],"env":{"TOKEN":"${TOKEN}","MODE":"fast"}},
    "web":{"type":"http","url":"https://example.com/mcp","headers":{"X-A":"b"}},
    "authed":{"type":"http","url":"https://example.com/a","headers":{"Authorization":"Bearer ${GH_TOKEN}"}},
    "old":{"type":"sse","url":"https://example.com/sse"},
    "withvar":{"command":"run","args":["${HOME}/x"]},
    "off":{"command":"x","disabled":true}
}}"#;

fn servers(made: &GeminiMcp) -> Value {
    serde_json::from_str::<Value>(made.settings.as_deref().expect("a settings file")).unwrap()["mcpServers"].clone()
}

#[test]
fn a_stdio_server_keeps_its_command_args_and_env_references() {
    let s = servers(&gemini_settings(MCP, &env(&[]), &abs));
    assert_eq!(s["git"]["command"], "/bin/uvx");
    assert_eq!(s["git"]["args"], json!(["mcp-server-git", "--repo", "C:\\work"]));
    assert_eq!(s["git"]["env"], json!({"MODE": "fast", "TOKEN": "${TOKEN}"}));
}

#[test]
fn http_and_sse_servers_use_gemini_keys() {
    let s = servers(&gemini_settings(MCP, &env(&[]), &abs));
    assert_eq!(s["web"], json!({"httpUrl": "https://example.com/mcp", "headers": {"X-A": "b"}}));
    assert_eq!(s["old"], json!({"url": "https://example.com/sse"}));
}

#[test]
fn the_agentmux_server_asks_for_the_agents_variables_by_reference() {
    let agent = env(&[("AGENTMUX_AGENT_ID", "aria"), ("AGENTMUX_JEKT_KEY", "real-key"), ("AGENTMUX_AUTH_KEY", "auth"), ("PATH", "/bin")]);
    let made = gemini_settings(MCP, &agent, &abs);
    let text = made.settings.clone().unwrap();
    assert!(!text.contains("real-key") && !text.contains("literal-key") && !text.contains("auth\""), "{text}");
    let e = &servers(&made)["agentmux"]["env"];
    assert_eq!(e["AGENTMUX_AUTH_KEY"], "${AGENTMUX_AUTH_KEY}");
    assert_eq!(e["AGENTMUX_JEKT_KEY"], "${AGENTMUX_JEKT_KEY}");
    assert_eq!(e["AGENTMUX_AGENT_ID"], "${AGENTMUX_AGENT_ID}");
    assert!(e.get("PATH").is_none());
}

#[test]
fn a_server_that_would_get_a_variable_it_cannot_expand_is_left_out_and_named() {
    let made = gemini_settings(MCP, &env(&[]), &abs);
    let s = servers(&made);
    assert!(s.get("authed").is_none() && s.get("withvar").is_none() && s.get("off").is_none());
    let names: Vec<&str> = made.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["authed", "withvar"]);
}

#[test]
fn no_server_means_no_file() {
    assert_eq!(gemini_settings("{}", &env(&[]), &abs), GeminiMcp::default());
    assert_eq!(gemini_settings("not json", &env(&[]), &abs), GeminiMcp::default());
    assert_eq!(gemini_settings(r#"{"mcpServers":{"off":{"command":"x","disabled":true}}}"#, &env(&[]), &abs).settings, None);
}

#[test]
fn a_gemini_turn_is_told_by_its_command() {
    for yes in ["gemini", "gemini.cmd", "C:\\npm\\gemini.CMD", "/usr/bin/gemini"] {
        assert!(is_gemini(yes), "{yes}");
    }
    for no in ["claude", "codex", "gemini-wrapper", ""] {
        assert!(!is_gemini(no), "{no}");
    }
}

#[test]
fn the_file_is_written_once_and_replaced_atomically() {
    let dir = std::env::temp_dir().join(format!("gemini-mcp-test-{}", uuid::Uuid::new_v4()));
    let path = write_settings(&dir, "block/1", "{\"a\":1}").unwrap();
    assert_eq!(path.file_name().unwrap(), "block_1.json");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    write_settings(&dir, "block/1", "{\"a\":2}").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":2}");
    let leftovers = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).count();
    assert_eq!(leftovers, 0);
    let _ = std::fs::remove_dir_all(&dir);
}
