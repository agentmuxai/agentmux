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
    // Asked first: srv's answer says whether a command read from this folder's own file may be started.
    let srv = Srv::from_env();
    let health = match &srv {
        Some(srv) => Some(srv.post("/api/v1/agent/self/mcp-health", Some(&workdir.to_string_lossy())).await),
        None => None,
    };
    let rule = match &health {
        Some(Ok((200, body))) => body.get("folder_rule").and_then(Value::as_bool),
        _ => None,
    };
    let config = config_path(&workdir);
    let entry = check_config(&mut report, &config);
    let entry = match may_start_entry(&config, &workdir, crate::self_keys::in_container(), rule) {
        Ok(()) => entry,
        Err(why) => {
            if entry.is_some() {
                report.fail(why);
            }
            None
        }
    };
    check_handshakes(&mut report, entry.as_ref(), &config);
    match (&srv, health) {
        (Some(srv), Some(answer)) => report_srv(&mut report, srv, answer),
        _ => report.fail("AGENTMUX_LOCAL_URL, AGENTMUX_AUTH_KEY, AGENTMUX_AGENT_TOKEN or AGENTMUX_AGENT_SLUG is not set: run this from the agent's own shell"),
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

/// The config this doctor reads: the workspace file in a folder AgentMux owns, the relocated
/// one in a user's project (#3680), and the bind-mounted one inside a container.
fn config_path(workdir: &Path) -> PathBuf {
    // srv resolves it at spawn, where it knows the data root and the cwd; a pane strips the
    // instance variables a reconstruction would need (#47385).
    let configured = std::env::var_os("AGENTMUX_MCP_CONFIG")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    resolve_config_path(workdir, configured.as_deref(), crate::self_keys::in_container())
}

/// [`config_path`] with its inputs given, so a test can pin them.
fn resolve_config_path(workdir: &Path, configured: Option<&Path>, in_container: bool) -> PathBuf {
    if let Some(configured) = configured {
        return configured.to_path_buf();
    }
    if in_container {
        let mounted = agentmux_common::mcp_config_path::container_config_path();
        if mounted.is_file() {
            return mounted;
        }
    }
    let expanded = agentmux_common::mcp_config_path::expand_home(workdir);
    agentmux_common::mcp_config_path::config_path_for(&expanded)
}

/// `path` as one spelling for comparing: `/` separators, no trailing one, folded on Windows.
fn spelling(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).trim_end_matches('/').to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Whether `file` is in `folder` or beneath it, as the two are written or as they resolve (links followed).
fn is_inside(file: &Path, folder: &Path) -> bool {
    let within = |file: &Path, folder: &Path| {
        let (file, folder) = (spelling(file), spelling(folder));
        !folder.is_empty() && (file == folder || file.starts_with(&format!("{folder}/")))
    };
    let resolved = |path: &Path| std::fs::canonicalize(path).ok();
    within(file, folder) || matches!((resolved(file), resolved(folder)), (Some(file), Some(folder)) if within(&file, &folder))
}

/// Whether doctor may start the command that the `agentmux` entry of `config` names. A config outside the
/// working folder is AgentMux's own file. One inside it is the folder's, and a container agent that can write
/// the folder could have written the entry: `rule` is srv's answer to that (`None` when it gave none), and the
/// command is started on this computer only on a plain no. Inside a container nothing here runs on the host.
fn may_start_entry(config: &Path, workdir: &Path, in_container: bool, rule: Option<bool>) -> Result<(), String> {
    if in_container || !is_inside(config, workdir) {
        return Ok(());
    }
    match rule {
        Some(false) => Ok(()),
        Some(true) => Err(format!(
            "a container agent can write in {} (or has run in it), so no command from its .mcp.json is started on this computer: run doctor inside the container, or from a host agent's own pane",
            workdir.display()
        )),
        None => Err(format!(
            "AgentMux did not say whether a container agent can write in {}, so no command from its .mcp.json is started: run doctor from the agent's own pane",
            workdir.display()
        )),
    }
}

fn check_config(report: &mut Report, path: &Path) -> Option<Entry> {
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
        report.fail(format!("the agentmux entry in {} has no command", path.display()));
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
        report.ok(format!("{} has the agentmux entry ({command}) and its signing key", path.display()));
    } else {
        report.fail(format!(
            "{}'s agentmux entry has no AGENTMUX_JEKT_KEY: messages can't be signed; restart the agent to get one",
            path.display()
        ));
    }
    Some(Entry { command, args })
}

/// Start the server the way the CLI does, and without the prefix too, so a
/// failure says which part broke.
fn check_handshakes(report: &mut Report, entry: Option<&Entry>, config: &Path) {
    let (command, args) = match entry {
        Some(e) => (e.command.clone(), e.args.clone()),
        None => ("agentmux-mcp".to_string(), Vec::new()),
    };
    // Said before it is started, not with the report at the end: the command was read from a file.
    if entry.is_some() {
        let line = std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
        println!("info about to start `{line}`, the agentmux entry of {}", config.display());
        let _ = std::io::stdout().flush();
    }
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
            client: agentmux_common::local_http::builder().timeout(Duration::from_secs(10)).build().ok()?,
            url: var("AGENTMUX_LOCAL_URL")?,
            auth_key: var("AGENTMUX_AUTH_KEY")?,
            token: var("AGENTMUX_AGENT_TOKEN")?,
            slug: var("AGENTMUX_AGENT_SLUG")?,
        })
    }

    /// `folder`: the folder doctor runs for, sent with the health question.
    async fn post(&self, path: &str, folder: Option<&str>) -> Result<(u16, Value), String> {
        let mut body = json!({ "agent": self.slug });
        if let Some(folder) = folder {
            body["folder"] = json!(folder);
        }
        let resp = self
            .client
            .post(format!("{}{path}", self.url.trim_end_matches('/')))
            .header(agentmux_common::AUTH_KEY_HEADER, &self.auth_key)
            .header("X-Agent-Token", self.token.trim())
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        Ok((status, resp.json().await.unwrap_or(Value::Null)))
    }
}

fn report_srv(report: &mut Report, srv: &Srv, answer: Result<(u16, Value), String>) {
    match answer {
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
    match h.get("rung").and_then(Value::as_u64) {
        Some(3) => line.push_str("; repair rewrote the agentmux entry of .mcp.json"),
        Some(4) => line.push_str("; repair rewrote the agentmux entry of .mcp.json, then with the absolute path of agentmux-mcp"),
        _ => {}
    }
    line.push_str(match (h.get("status").and_then(Value::as_str), h.get("hello_ms").and_then(Value::as_u64)) {
        (_, Some(_)) => "; the server has contacted AgentMux",
        (Some("failed"), None) => "; the server has never contacted AgentMux, so the process did not run",
        (Some("connected"), None) => "; the server has not contacted AgentMux (the CLI started it; it may not reach AgentMux, or it predates the heartbeat)",
        _ => "",
    });
    match h.get("selftest").and_then(Value::as_str) {
        Some("ok") => line.push_str("; the launch self-test started it through the shell prefix and it answered"),
        Some(failed) => line.push_str(&format!("; the launch self-test failed: {failed}")),
        None => {}
    }
    line + auto
}

async fn request_heal(report: &mut Report, srv: &Srv) {
    match srv.post("/api/v1/agent/self/mcp-heal", None).await {
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

    /// #3680: a user's project keeps its own `.mcp.json`, so the doctor must read the agent's
    /// relocated config instead; an AgentMux-owned folder still reads the file in it.
    #[test]
    fn the_config_checked_follows_the_agent_where_its_config_lives() {
        // Not `config_path`: that reads AGENTMUX_MCP_CONFIG, which a test run inside an agent
        // pane inherits from the real spawn and would otherwise override this fixture.
        let project = Path::new("/agentmux-test-project");
        let path = resolve_config_path(project, None, false);
        assert!(!path.starts_with(project), "the project file is not the agent's: {}", path.display());
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("mcp.json"));

        let owned = dirs_home().join(".agentmux").join("agents").join("aria");
        assert_eq!(resolve_config_path(&owned, None, false), owned.join(".mcp.json"));
    }

    fn dirs_home() -> PathBuf {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_default()
    }

    /// #47385: the path srv resolved at spawn wins over any reconstruction, and without it the
    /// fallback still relocates a user's project.
    #[test]
    fn the_config_is_the_path_srv_resolved_when_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let configured = dir.path().join("mcp.json");
        assert_eq!(resolve_config_path(Path::new("/agentmux-test-project"), Some(&configured), false), configured);
        let fallback = resolve_config_path(Path::new("/agentmux-test-project"), None, false);
        assert!(!fallback.starts_with("/agentmux-test-project"), "{}", fallback.display());
    }

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
             last error: Connection closed; the server has never contacted AgentMux, so the process did not run \
             (automatic repair is off)"
        );
        let heard = json!({ "health": { "status": "connected", "phase": "ok", "hello_ms": 5 } });
        assert_eq!(srv_health_line(&heard), "srv saw the server connected; repair: ok; the server has contacted AgentMux");
        let tested = json!({ "health": { "status": "failed", "phase": "ok", "selftest": "no response within 10s" } });
        assert!(srv_health_line(&tested).ends_with("; the launch self-test failed: no response within 10s"));
        let passed = json!({ "health": { "status": "connected", "phase": "ok", "hello_ms": 5, "selftest": "ok" } });
        assert!(srv_health_line(&passed).contains("the launch self-test started it through the shell prefix and it answered"));
        let rewrote = json!({ "health": { "status": "failed", "phase": "healing", "rung": 4, "hello_ms": 5 } });
        assert!(srv_health_line(&rewrote).contains("rewrote the agentmux entry of .mcp.json, then with the absolute path"));
        let silent = json!({ "health": { "status": "connected", "phase": "ok" } });
        assert!(srv_health_line(&silent).contains("has not contacted AgentMux"));
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

    #[test]
    fn a_command_from_the_folders_own_file_is_started_only_when_no_container_agent_can_write_the_folder() {
        let folder = tempfile::tempdir().unwrap();
        let own = folder.path().join(".mcp.json");
        std::fs::write(&own, "{}").unwrap();
        assert_eq!(may_start_entry(&own, folder.path(), false, Some(false)), Ok(()), "srv said no container agent can");
        let refused = may_start_entry(&own, folder.path(), false, Some(true)).unwrap_err();
        assert!(refused.contains("a container agent can write in") && refused.contains("no command from its .mcp.json is started"), "{refused}");
        let untold = may_start_entry(&own, folder.path(), false, None).unwrap_err();
        assert!(untold.contains("did not say") && untold.contains("no command from its .mcp.json is started"), "{untold}");
        assert_eq!(may_start_entry(&own, folder.path(), true, Some(true)), Ok(()), "inside the container nothing runs on the host");
    }

    #[test]
    fn a_config_outside_the_folder_is_agentmuxs_own_and_is_started_from() {
        let (folder, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let outside = data.path().join("mcp").join("abc").join("mcp.json");
        for rule in [Some(true), Some(false), None] {
            assert_eq!(may_start_entry(&outside, folder.path(), false, rule), Ok(()), "{rule:?}");
        }
        assert!(is_inside(&folder.path().join("sub").join(".mcp.json"), folder.path()));
        assert!(is_inside(folder.path(), folder.path()));
        assert!(!is_inside(&outside, folder.path()));
        let sibling = folder.path().with_file_name(format!("{}-other", folder.path().file_name().unwrap().to_string_lossy()));
        assert!(!is_inside(&sibling.join(".mcp.json"), folder.path()), "a folder whose name only starts the same");
    }
}
