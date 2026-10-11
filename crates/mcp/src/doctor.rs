// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux-mcp doctor [--heal]`: why this agent's AgentMux tools aren't
//! connected, run from its shell with no MCP session
//! (docs/specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md §7). It checks what a
//! person would check by hand: that this binary starts and answers MCP, also
//! through Claude Code's shell prefix the way the CLI starts it; the agent's
//! `.mcp.json`; what srv saw and tried; and Claude Code's own log for the
//! server. Read-only, except `--heal`, which asks srv to run the repair for
//! the caller's own pane.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"agentmux-mcp-doctor","version":"0"}}}"#;

#[derive(Default)]
struct Report {
    lines: Vec<(&'static str, String)>,
}

impl Report {
    fn ok(&mut self, text: impl Into<String>) {
        self.lines.push(("ok  ", text.into()));
    }
    fn fail(&mut self, text: impl Into<String>) {
        self.lines.push(("FAIL", text.into()));
    }
    fn info(&mut self, text: impl Into<String>) {
        self.lines.push(("info", text.into()));
    }
    fn failed(&self) -> bool {
        self.lines.iter().any(|(k, _)| *k == "FAIL")
    }
}

/// Run the checks, print the report, and return the exit code: 1 when a
/// check failed.
pub async fn run(args: Vec<String>) -> i32 {
    let heal = args.iter().any(|a| a == "--heal");
    let mut report = Report::default();
    let workdir = std::env::var("AGENTMUX_AGENT_WORKDIR")
        .ok()
        .filter(|w| !w.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();

    report.ok(format!(
        "agentmux-mcp {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
    ));
    let entry = check_config(&mut report, &workdir);
    check_handshakes(&mut report, entry.as_ref());
    let srv = Srv::from_env();
    match &srv {
        Some(srv) => check_srv(&mut report, srv).await,
        None => report.fail("AGENTMUX_LOCAL_URL, AGENTMUX_AUTH_KEY, AGENTMUX_AGENT_TOKEN or AGENTMUX_AGENT_SLUG is not set: run this from the agent's own shell"),
    }
    check_cli_log(&mut report, &workdir);
    if heal {
        match &srv {
            Some(srv) => request_heal(&mut report, srv).await,
            None => report.fail("--heal needs srv (see above)"),
        }
    }

    for (kind, text) in &report.lines {
        println!("{kind} {text}");
    }
    if !heal && report.failed() {
        println!("\nRun `agentmux-mcp doctor --heal` to ask AgentMux to repair it, and tell the operator.");
    }
    i32::from(report.failed())
}

/// The `agentmux` entry in the agent's `.mcp.json`: its command and args.
struct Entry {
    command: String,
    args: Vec<String>,
}

fn check_config(report: &mut Report, workdir: &Path) -> Option<Entry> {
    let path = workdir.join(".mcp.json");
    let parsed = match std::fs::read_to_string(&path).map(|t| serde_json::from_str::<Value>(&t)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            report.fail(format!("{} is not valid JSON: {e}", path.display()));
            return None;
        }
        Err(e) => {
            report.fail(format!("can't read {}: {e}", path.display()));
            return None;
        }
    };
    let Some(server) = parsed.pointer("/mcpServers/agentmux") else {
        report.fail(format!("{} has no mcpServers.agentmux entry", path.display()));
        return None;
    };
    let command = server.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
    if command.is_empty() {
        report.fail("the agentmux entry in .mcp.json has no command");
        return None;
    }
    let args: Vec<String> = server
        .get("args")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    // Present or not, never their values.
    let key_set = |k: &str| server.pointer(&format!("/env/{k}")).and_then(Value::as_str).is_some_and(|v| !v.is_empty());
    if key_set("AGENTMUX_JEKT_KEY") {
        report.ok(format!(".mcp.json has the agentmux entry ({command}) and its signing key"));
    } else {
        report.fail(".mcp.json's agentmux entry has no AGENTMUX_JEKT_KEY: messages can't be signed; restart the agent to get one");
    }
    Some(Entry { command, args })
}

/// Start the server the way the CLI does, and without the prefix too, so a
/// failure says which part broke.
fn check_handshakes(report: &mut Report, entry: Option<&Entry>) {
    let (command, args) = match entry {
        Some(e) => (e.command.clone(), e.args.clone()),
        None => ("agentmux-mcp".to_string(), Vec::new()),
    };
    match handshake(Command::new(&command).args(&args)) {
        Ok(version) => report.ok(format!("`{command}` starts and answers MCP (server {version})")),
        Err(e) => report.fail(format!("`{command}` doesn't answer MCP: {e}")),
    }
    let Some(prefix) = std::env::var("CLAUDE_CODE_SHELL_PREFIX").ok().filter(|p| !p.is_empty()) else {
        report.info("no CLAUDE_CODE_SHELL_PREFIX: the CLI starts the server directly");
        return;
    };
    // The CLI passes the prefix one argument: the command line, joined.
    let joined = std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
    let name = Path::new(&prefix).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(prefix.clone());
    match handshake(Command::new(&prefix).arg(&joined)) {
        Ok(_) => report.ok(format!("it starts through the shell prefix ({name}), as the CLI starts it")),
        Err(e) => report.fail(format!(
            "it does NOT start through the shell prefix ({name}), which is how the CLI starts it: {e}"
        )),
    }
}

/// Send `initialize` and wait for the answer: the server's version, or why not.
fn handshake(cmd: &mut Command) -> Result<String, String> {
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        cmd.no_window();
    }
    let mut child = cmd.spawn().map_err(|e| format!("can't start it: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let _ = writeln!(stdin, "{INITIALIZE}");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let answer = rx.recv_timeout(HANDSHAKE_TIMEOUT);
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let first_stderr_line = || {
        let mut text = String::new();
        let _ = BufReader::new(stderr).read_line(&mut text);
        text.trim().to_string()
    };
    match answer {
        Ok(line) if !line.trim().is_empty() => match serde_json::from_str::<Value>(&line) {
            Ok(v) => v
                .pointer("/result/serverInfo/version")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("unexpected answer: {}", line.trim())),
            Err(_) => Err(format!("not JSON-RPC on stdout: {}", line.trim())),
        },
        Ok(_) => {
            let err = first_stderr_line();
            Err(if err.is_empty() { "it exited without answering".to_string() } else { format!("it exited: {err}") })
        }
        Err(_) => Err(format!("no answer in {} s", HANDSHAKE_TIMEOUT.as_secs())),
    }
}

struct Srv {
    client: reqwest::Client,
    url: String,
    auth_key: String,
    token: String,
    slug: String,
}

impl Srv {
    fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Some(Self {
            client: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().ok()?,
            url: var("AGENTMUX_LOCAL_URL")?,
            auth_key: var("AGENTMUX_AUTH_KEY")?,
            token: var("AGENTMUX_AGENT_TOKEN")?,
            slug: var("AGENTMUX_AGENT_SLUG")?,
        })
    }

    async fn post(&self, path: &str) -> Result<(u16, Value), String> {
        let resp = self
            .client
            .post(format!("{}{path}", self.url.trim_end_matches('/')))
            .header(agentmux_common::AUTH_KEY_HEADER, &self.auth_key)
            .header("X-Agent-Token", self.token.trim())
            .json(&json!({ "agent": self.slug }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        Ok((status, resp.json().await.unwrap_or(Value::Null)))
    }
}

async fn check_srv(report: &mut Report, srv: &Srv) {
    match srv.post("/api/v1/agent/self/mcp-health").await {
        Ok((200, body)) => {
            report.ok(format!("AgentMux (srv) answers at {}", srv.url));
            report.info(srv_health_line(&body));
        }
        Ok((404, _)) => report.info("this AgentMux is older than this doctor: it can't say what it saw, or repair"),
        Ok((code, body)) => report.fail(format!("srv refused ({code}): {}", body.get("error").and_then(Value::as_str).unwrap_or(""))),
        Err(e) => report.fail(format!("can't reach srv at {}: {e}", srv.url)),
    }
}

/// One line on what srv saw and did, from its `mcp-health` answer.
fn srv_health_line(body: &Value) -> String {
    let auto = if body.get("auto").and_then(Value::as_bool) == Some(false) { " (automatic repair is off)" } else { "" };
    let Some(h) = body.get("health").filter(|h| !h.is_null()) else {
        return format!("srv hasn't seen this agent's MCP status yet{auto}");
    };
    let get = |k: &str| h.get(k).and_then(Value::as_str).unwrap_or("?");
    let mut line = format!("srv saw the server {}; repair: {}", get("status"), get("phase"));
    if h.get("safe_profile").and_then(Value::as_bool) == Some(true) {
        line.push_str("; running in the safe profile (no shell prefix)");
    }
    if let Some(e) = h.get("last_error").and_then(Value::as_str) {
        line.push_str(&format!("; last error: {e}"));
    }
    line + auto
}

async fn request_heal(report: &mut Report, srv: &Srv) {
    match srv.post("/api/v1/agent/self/mcp-heal").await {
        Ok((202, _)) => report.ok("repair started: AgentMux is reconnecting the server; the pane shows how it goes"),
        Ok((404, _)) => report.fail("this AgentMux is older than this doctor and can't repair: ask the operator to restart the agent"),
        Ok((200, body)) => {
            let status = body.get("status").and_then(Value::as_str).unwrap_or("?");
            report.info(match status {
                "connected" => "nothing to repair: srv saw the server connected".to_string(),
                "already_healing" => "a repair is already under way".to_string(),
                "no_status_yet" => "srv hasn't seen the server's status yet; try again once the agent has started".to_string(),
                "no_process" => "the agent's CLI isn't running".to_string(),
                other => format!("srv: {other}"),
            });
        }
        Ok((code, body)) => report.fail(format!("srv refused the repair ({code}): {}", body.get("error").and_then(Value::as_str).unwrap_or(""))),
        Err(e) => report.fail(format!("can't reach srv: {e}")),
    }
}

/// Claude Code's folder name for a project: its path with every character
/// that isn't a letter or digit made `-`.
fn project_key(dir: &Path) -> String {
    dir.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Where Claude Code keeps its per-project caches (the `env-paths` cache
/// folder for `claude-cli`). Its own layout, so a miss is reported, not an
/// error.
fn cli_cache_dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        Some(var("LOCALAPPDATA")?.join("claude-cli-nodejs").join("Cache"))
    } else if cfg!(target_os = "macos") {
        Some(var("HOME")?.join("Library").join("Caches").join("claude-cli-nodejs"))
    } else {
        let base = var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|h| h.join(".cache")))?;
        Some(base.join("claude-cli-nodejs"))
    }
}

fn check_cli_log(report: &mut Report, workdir: &Path) {
    let Some(dir) = cli_cache_dir().map(|c| c.join(project_key(workdir)).join("mcp-logs-agentmux")) else {
        return;
    };
    let newest = std::fs::read_dir(&dir).ok().and_then(|entries| {
        entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
    });
    let Some(file) = newest else {
        report.info(format!("no Claude Code MCP log for the server in {}", dir.display()));
        return;
    };
    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let name = file.file_name().to_string_lossy().into_owned();
    report.info(format!("Claude Code's latest log for the server ({name}): {}", log_outcome(&text)));
}

/// What the CLI's log says happened: the last attempt's error, with what the
/// server printed on stderr (the useful half), or that it connected.
fn log_outcome(text: &str) -> String {
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut outcome = "no connection attempt recorded".to_string();
    let mut stderr: Option<String> = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let debug = v.get("debug").and_then(Value::as_str).unwrap_or_default();
        if debug.starts_with("Starting connection") {
            stderr = None;
        } else if debug.starts_with("Successfully connected") {
            outcome = "connected".to_string();
        } else if let Some(e) = v.get("error").and_then(Value::as_str) {
            match e.strip_prefix("Server stderr:") {
                Some(rest) => stderr = Some(flat(rest)),
                None => {
                    outcome = match &stderr {
                        Some(s) => format!("error: {} (server stderr: {s})", flat(e)),
                        None => format!("error: {}", flat(e)),
                    }
                }
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_key_is_claude_codes_folder_name() {
        assert_eq!(
            project_key(Path::new(r"C:\Users\u\.agentmux\agents\agenty-0629j")),
            "C--Users-u--agentmux-agents-agenty-0629j"
        );
        assert_eq!(project_key(Path::new("/home/u/work")), "-home-u-work");
    }

    #[test]
    fn the_log_outcome_is_the_last_thing_that_happened() {
        let failed = r#"{"debug":"Starting connection with timeout of 30000ms"}
{"error":"Server stderr: error: unrecognized subcommand 'agentmux-mcp'\n\nUsage: agentmux-bashwrap.exe <COMMAND>\n"}
{"error":"Connection failed (CONNECTION_CLOSED): Connection closed"}"#;
        assert_eq!(
            log_outcome(failed),
            "error: Connection failed (CONNECTION_CLOSED): Connection closed (server stderr: error: unrecognized \
             subcommand 'agentmux-mcp' Usage: agentmux-bashwrap.exe <COMMAND>)"
        );
        let ok = r#"{"debug":"Starting connection with timeout of 30000ms"}
{"debug":"Successfully connected (transport: stdio) in 39ms"}"#;
        assert_eq!(log_outcome(ok), "connected");
        assert_eq!(log_outcome(""), "no connection attempt recorded");
    }

    #[test]
    fn the_srv_line_says_what_srv_saw() {
        assert_eq!(srv_health_line(&json!({ "health": null, "auto": true })), "srv hasn't seen this agent's MCP status yet");
        let body = json!({ "auto": false, "health": {
            "status": "failed", "phase": "gave_up", "safe_profile": true, "last_error": "Connection closed" } });
        assert_eq!(
            srv_health_line(&body),
            "srv saw the server failed; repair: gave_up; running in the safe profile (no shell prefix); \
             last error: Connection closed (automatic repair is off)"
        );
    }

    #[test]
    fn a_failed_check_fails_the_run() {
        let mut r = Report::default();
        r.ok("a");
        r.info("b");
        assert!(!r.failed());
        r.fail("c");
        assert!(r.failed());
    }
}
