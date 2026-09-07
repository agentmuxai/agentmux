// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! MCP server health/prerequisite probe
//! (docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md §4.4).
//!
//! Opens a short-lived MCP client connection to a configured server — spawns
//! the stdio process (or POSTs to the http url), performs the `initialize`
//! handshake, then `tools/list` — and reports whether it's reachable at all.
//!
//! This is a protocol-level probe only. For a server like ableton-mcp, a
//! successful probe means the MCP process itself started and speaks the
//! protocol — NOT that the underlying app (Ableton Live) is running, since
//! that dependency is only exercised when a tool is actually called. That
//! gap is why the catalog's `prereq_note` (static remediation text, §4.6)
//! exists alongside this dynamic check, not instead of it.
//!
//! Deliberately hand-rolled rather than pulling in an MCP client SDK crate
//! (e.g. `rmcp`): `agentmux-mcp/src/main.rs` already hand-rolls the *server*
//! side of this exact newline-delimited JSON-RPC 2.0 wire format, so a
//! probe-only client follows the same house style instead of adding a new
//! dependency for a handshake this small.

use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    Connected,
    Unreachable,
    HandshakeFailed,
    InvalidConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub status: ProbeStatus,
    pub tool_count: Option<usize>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub error: Option<String>,
}

impl ProbeResult {
    fn empty(status: ProbeStatus, error: Option<String>) -> Self {
        Self { status, tool_count: None, server_name: None, server_version: None, error }
    }
    fn invalid(msg: impl Into<String>) -> Self {
        Self::empty(ProbeStatus::InvalidConfig, Some(msg.into()))
    }
    fn unreachable(msg: impl Into<String>) -> Self {
        Self::empty(ProbeStatus::Unreachable, Some(msg.into()))
    }
    fn handshake_failed(msg: impl Into<String>) -> Self {
        Self::empty(ProbeStatus::HandshakeFailed, Some(msg.into()))
    }
    fn connected(server_name: Option<String>, server_version: Option<String>, tool_count: Option<usize>) -> Self {
        Self { status: ProbeStatus::Connected, tool_count, server_name, server_version, error: None }
    }
}

/// Probe a server given its `transport` (`"stdio"` | `"http"` | `"sse"` |
/// any other free-text value from the pre-v11 field, treated as stdio for
/// back-compat) and its raw `config` JSON blob — the exact object
/// `agent_config::build_mcp_config_from_refs` merges into `.mcp.json`.
pub async fn probe(transport: &str, config_json: &str) -> ProbeResult {
    let config: Value = match serde_json::from_str(config_json) {
        Ok(v) => v,
        Err(e) => return ProbeResult::invalid(format!("config is not valid JSON: {e}")),
    };

    let is_http = matches!(transport, "http" | "sse" | "streamable-http") || config.get("url").is_some();

    let outcome = if is_http {
        timeout(PROBE_TIMEOUT, probe_http(&config)).await
    } else {
        timeout(PROBE_TIMEOUT, probe_stdio(&config)).await
    };

    match outcome {
        Ok(result) => result,
        Err(_) => ProbeResult::handshake_failed(format!("no response within {}s", PROBE_TIMEOUT.as_secs())),
    }
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "agentmux-probe", "version": env!("CARGO_PKG_VERSION") },
        },
    })
}

fn tools_list_request() -> Value {
    json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} })
}

// ── stdio ────────────────────────────────────────────────────────────────

async fn probe_stdio(config: &Value) -> ProbeResult {
    let command = match config.get("command").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => c,
        _ => return ProbeResult::invalid("stdio config missing a non-empty \"command\" string"),
    };
    let args: Vec<String> = config
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let env: Vec<(String, String)> = config
        .get("env")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();

    let mut cmd = Command::new(command);
    cmd.args(&args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // CREATE_NO_WINDOW: console-flash suppression, see agentmux-common/src/cli.rs
    #[cfg(windows)]
    {
        use agentmux_common::win32::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ProbeResult::unreachable(format!("failed to start \"{command}\": {e}")),
    };

    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => return ProbeResult::unreachable("failed to open child stdin"),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return ProbeResult::unreachable("failed to open child stdout"),
    };
    let mut reader = BufReader::new(stdout);

    let result = run_stdio_handshake(&mut stdin, &mut reader).await;
    let _ = child.start_kill(); // probe is ephemeral — never leave it running
    result
}

async fn run_stdio_handshake<W, R>(stdin: &mut W, reader: &mut R) -> ProbeResult
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    if let Err(e) = write_line(stdin, &initialize_request()).await {
        return ProbeResult::unreachable(format!("failed to write initialize request: {e}"));
    }
    let init_resp = match read_json_line(reader).await {
        Ok(Some(v)) => v,
        Ok(None) => return ProbeResult::handshake_failed("process closed stdout before responding to initialize"),
        Err(e) => return ProbeResult::handshake_failed(format!("failed to read initialize response: {e}")),
    };
    if let Some(err) = init_resp.get("error") {
        return ProbeResult::handshake_failed(format!("server rejected initialize: {err}"));
    }
    let server_info = init_resp.pointer("/result/serverInfo").cloned().unwrap_or(Value::Null);
    let server_name = server_info.get("name").and_then(|v| v.as_str()).map(str::to_string);
    let server_version = server_info.get("version").and_then(|v| v.as_str()).map(str::to_string);

    // notifications/initialized has no id and expects no response — best-effort.
    let _ = write_line(stdin, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;

    if write_line(stdin, &tools_list_request()).await.is_err() {
        // The handshake itself already succeeded even if tools/list couldn't be sent.
        return ProbeResult::connected(server_name, server_version, None);
    }
    let tool_count = match read_json_line(reader).await {
        Ok(Some(v)) => v.pointer("/result/tools").and_then(|t| t.as_array()).map(|a| a.len()),
        _ => None,
    };

    ProbeResult::connected(server_name, server_version, tool_count)
}

async fn write_line<W: AsyncWrite + Unpin>(w: &mut W, v: &Value) -> std::io::Result<()> {
    let s = serde_json::to_string(v).unwrap_or_default();
    w.write_all(s.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.flush().await
}

/// Reads lines until one parses as JSON (skipping blank lines or stray
/// non-JSON output, e.g. a server printing a startup banner to stdout),
/// or the stream closes.
async fn read_json_line<R: AsyncBufRead + Unpin>(r: &mut R) -> std::io::Result<Option<Value>> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            return Ok(Some(v));
        }
    }
}

// ── http (Streamable HTTP transport) ────────────────────────────────────

async fn probe_http(config: &Value) -> ProbeResult {
    let url = match config.get("url").and_then(|v| v.as_str()) {
        Some(u) if !u.is_empty() => u,
        _ => return ProbeResult::invalid("http config missing a non-empty \"url\" string"),
    };

    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(h) = config.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in h {
            let (Ok(name), Some(val)) = (reqwest::header::HeaderName::from_bytes(k.as_bytes()), v.as_str()) else {
                continue;
            };
            if let Ok(hv) = reqwest::header::HeaderValue::from_str(val) {
                headers.insert(name, hv);
            }
        }
    }
    let client = match reqwest::Client::builder().default_headers(headers).timeout(PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => return ProbeResult::invalid(format!("failed to build http client: {e}")),
    };

    let init_resp = match post_jsonrpc(&client, url, &initialize_request()).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Some(err) = init_resp.get("error") {
        return ProbeResult::handshake_failed(format!("server rejected initialize: {err}"));
    }
    let server_info = init_resp.pointer("/result/serverInfo").cloned().unwrap_or(Value::Null);
    let server_name = server_info.get("name").and_then(|v| v.as_str()).map(str::to_string);
    let server_version = server_info.get("version").and_then(|v| v.as_str()).map(str::to_string);

    let tool_count = match post_jsonrpc(&client, url, &tools_list_request()).await {
        Ok(v) => v.pointer("/result/tools").and_then(|t| t.as_array()).map(|a| a.len()),
        Err(_) => None, // reachability + handshake already proven above
    };

    ProbeResult::connected(server_name, server_version, tool_count)
}

/// POSTs one JSON-RPC request and returns the first JSON object in the
/// response body — handling both a plain `application/json` body and a
/// Streamable HTTP `text/event-stream` framing (`event: message\ndata:
/// {...}\n\n`), since a probe only needs the first frame either way.
async fn post_jsonrpc(client: &reqwest::Client, url: &str, req: &Value) -> Result<Value, ProbeResult> {
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(req)
        .send()
        .await
        .map_err(|e| ProbeResult::unreachable(format!("request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ProbeResult::handshake_failed(format!("server returned HTTP {}", resp.status())));
    }
    let body = resp.text().await.map_err(|e| ProbeResult::handshake_failed(format!("failed to read response body: {e}")))?;
    let json_line = body.lines().find(|l| l.trim_start().starts_with('{')).unwrap_or(body.trim());
    serde_json::from_str(json_line).map_err(|e| ProbeResult::handshake_failed(format!("non-JSON response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_json_config_is_reported_as_invalid_config() {
        let result = probe("stdio", "{not json").await;
        assert!(matches!(result.status, ProbeStatus::InvalidConfig));
        assert!(result.error.unwrap().contains("not valid JSON"));
    }

    #[tokio::test]
    async fn stdio_config_missing_command_is_invalid() {
        let result = probe("stdio", "{}").await;
        assert!(matches!(result.status, ProbeStatus::InvalidConfig));
        assert!(result.error.unwrap().contains("command"));
    }

    #[tokio::test]
    async fn stdio_config_with_unresolvable_binary_is_unreachable() {
        let config = json!({ "command": "definitely-not-a-real-agentmux-probe-binary-xyz" }).to_string();
        let result = probe("stdio", &config).await;
        assert!(matches!(result.status, ProbeStatus::Unreachable), "expected Unreachable, got {:?}", result.status);
    }

    #[tokio::test]
    async fn http_config_missing_url_is_invalid() {
        let result = probe("http", "{}").await;
        assert!(matches!(result.status, ProbeStatus::InvalidConfig));
        assert!(result.error.unwrap().contains("url"));
    }

    #[tokio::test]
    async fn http_transport_is_inferred_from_url_field_even_without_explicit_kind() {
        // transport is still the free-text pre-v11 field; a config that
        // carries "url" should route to the http probe regardless of what
        // (if anything) the transport string says.
        let config = json!({ "url": "http://127.0.0.1:1/nonexistent" }).to_string();
        let result = probe("stdio", &config).await;
        assert!(matches!(result.status, ProbeStatus::Unreachable), "expected Unreachable, got {:?}", result.status);
    }
}
