// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Terminal: one tool for the agent's terminals, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S1;
//! docs/specs/SPEC_AGENT_TERMINAL_DRIVING_2026_10_15.md P1). It replaces
//! Shell*, PtyShell* and ConnList. Their handlers (shell.rs, pty_shell.rs)
//! still make the srv calls whose answer needs no structure; this file does
//! the rest: keys by name, Enter as CR on every platform, reads without ANSI
//! from a cursor, waits, and `run`'s exit code.

use super::*;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

pub(super) const ACTIONS: &[&str] = &["open", "type", "read", "wait", "run", "close", "connections"];

/// What a terminal id names: a PTY shell (the drawer) or a background job (a pipe).
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Pty,
    Background,
}

#[derive(Clone, Debug)]
struct Known {
    kind: Kind,
    connection: Option<String>,
}

#[derive(Default)]
struct State {
    known: HashMap<String, Known>,
    drawer: Option<String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// Per call to the PTY input endpoint: under cmd.exe's console line limit
/// (a 3,000-character line reads back as 511 there), with room for keys.
const CHUNK_CHARS: usize = 400;
const CHUNK_GAP: Duration = Duration::from_millis(15);
const POLL: Duration = Duration::from_millis(200);
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Enough to cover a whole run's output; the result is cut to `MAX_RUN_LINES`.
const READ_ALL_LINES: u32 = 20_000;
const MAX_RUN_LINES: usize = 400;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Terminal" => {
            let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let out = match action {
                "open" => open(arguments, cx).await?,
                "type" => type_action(arguments, cx).await?,
                "read" => read_action(arguments, cx).await?,
                "wait" => wait_action(arguments, cx).await?,
                "run" => run_action(arguments, cx).await?,
                "close" => close(arguments, cx).await?,
                "connections" => return shell::call("ConnList", arguments, cx).await,
                "" => anyhow::bail!("missing required parameter: action (one of {})", ACTIONS.join(", ")),
                other => anyhow::bail!("unknown action {other:?}: use one of {}", ACTIONS.join(", ")),
            };
            Ok(serde_json::to_string_pretty(&out)?)
        }
        _ => Err(not_in_family()),
    }
}

// ── Which terminal ───────────────────────────────────────────────────────

fn str_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// The platform and shell a terminal runs on, so an agent that needs shell
/// syntax reads it instead of guessing. Local: this machine; srv runs the
/// Windows PTY shell as cmd.exe (http_pty_shell.rs).
fn target(connection: Option<&str>) -> Value {
    match connection.filter(|c| *c != "local") {
        None => {
            let shell = if cfg!(windows) {
                "cmd".to_string()
            } else {
                std::env::var("SHELL")
                    .ok()
                    .and_then(|s| std::path::Path::new(&s).file_name().map(|f| f.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "sh".to_string())
            };
            json!({ "platform": std::env::consts::OS, "shell": shell, "connection": "local" })
        }
        Some(c) if c.starts_with("wsl://") => json!({ "platform": "linux", "shell": "login shell", "connection": c }),
        Some(c) => json!({ "platform": "remote", "shell": "login shell", "connection": c }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ShellFamily {
    Cmd,
    Posix,
    /// fish has no `$?`: its last status is `$status`.
    Fish,
}

fn shell_family(connection: Option<&str>) -> ShellFamily {
    match connection.filter(|c| *c != "local") {
        None if cfg!(windows) => ShellFamily::Cmd,
        None if target(None)["shell"] == "fish" => ShellFamily::Fish,
        _ => ShellFamily::Posix,
    }
}

fn remember(id: &str, kind: Kind, connection: Option<String>) {
    let mut st = STATE.lock().unwrap();
    st.known.insert(id.to_string(), Known { kind: kind.clone(), connection });
    if kind == Kind::Pty {
        st.drawer = Some(id.to_string());
    }
}

fn known(id: &str) -> Known {
    STATE
        .lock()
        .unwrap()
        .known
        .get(id)
        .cloned()
        .unwrap_or(Known { kind: Kind::Pty, connection: None })
}

/// The `terminal` an action names (`shell_id` is accepted too), else the
/// agent's drawer, opened if this process hasn't opened it yet.
async fn resolve(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    if let Some(id) = str_arg(arguments, "terminal").or_else(|| str_arg(arguments, "shell_id")) {
        return Ok(id.to_string());
    }
    if let Some(id) = STATE.lock().unwrap().drawer.clone() {
        return Ok(id);
    }
    let opened = open(arguments, cx).await?;
    Ok(opened["terminal"].as_str().unwrap_or_default().to_string())
}

// ── open / close ─────────────────────────────────────────────────────────

async fn open(arguments: &Value, cx: &ToolCtx<'_>) -> Result<Value> {
    let id = pty_shell::call("PtyShell", arguments, cx).await?;
    let connection = str_arg(arguments, "connection").map(str::to_string);
    remember(&id, Kind::Pty, connection.clone());
    let mut out = target(connection.as_deref());
    out["terminal"] = json!(id);
    out["kind"] = json!("drawer");
    out["note"] = json!("Your pane's shell drawer: the same terminal the user sees there.");
    Ok(out)
}

async fn close(arguments: &Value, cx: &ToolCtx<'_>) -> Result<Value> {
    let id = resolve(arguments, cx).await?;
    let args = json!({ "shell_id": id });
    let message = match known(&id).kind {
        Kind::Background => shell::call("ShellStop", &args, cx).await?,
        Kind::Pty => pty_shell::call("PtyShellStop", &args, cx).await?,
    };
    Ok(json!({ "terminal": id, "result": message }))
}

// ── type ─────────────────────────────────────────────────────────────────

async fn type_action(arguments: &Value, cx: &ToolCtx<'_>) -> Result<Value> {
    let id = resolve(arguments, cx).await?;
    let text = arguments.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let keys = key_list(arguments)?;
    let submit = arguments.get("submit").and_then(|v| v.as_bool()).unwrap_or(false);
    if text.is_empty() && keys.is_empty() && !submit {
        anyhow::bail!("type needs text, keys or submit");
    }

    if known(&id).kind == Kind::Background {
        if !keys.is_empty() {
            anyhow::bail!("keys go to a terminal; a background job takes text on stdin only");
        }
        let result = shell::call("ShellInput", &json!({ "shell_id": id, "text": text }), cx).await?;
        return Ok(json!({ "terminal": id, "result": result }));
    }

    if let (Some(rows), Some(cols)) = (arguments.get("rows"), arguments.get("cols")) {
        pty_shell::call("PtyShellResize", &json!({ "shell_id": id, "rows": rows, "cols": cols }), cx).await?;
    }
    let start = read_log(cx, &id, None, 1).await?.end;
    let mut bytes = to_cr(text);
    for k in &keys {
        bytes.push_str(k);
    }
    if submit {
        bytes.push('\r');
    }
    write(cx, &id, &bytes).await?;

    let mut out = json!({ "terminal": id, "since": start });
    if arguments.get("until").is_some() {
        // Skip the echo of typed text (whether or not it was submitted), and
        // only then: keys or a bare Enter echo no line of their own.
        let waited = wait_on(arguments, cx, &id, start, !text.is_empty()).await?;
        out["wait"] = waited;
    } else {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let tail_lines = arguments.get("tail_lines").and_then(|v| v.as_u64()).unwrap_or(15) as u32;
    if tail_lines > 0 {
        let log = read_log(cx, &id, Some(start), tail_lines).await?;
        out["output"] = json!(strip_ansi(&log.content));
        out["since"] = json!(log.end);
    }
    Ok(out)
}

/// `keys` as the bytes each names. An unknown name is an error, never typed.
fn key_list(arguments: &Value) -> Result<Vec<String>> {
    let Some(list) = arguments.get("keys") else { return Ok(vec![]) };
    let names: Vec<&str> = match list {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
        _ => anyhow::bail!("keys is a list of key names, e.g. [\"Ctrl+C\"] or [\"Down\", \"Enter\"]"),
    };
    names
        .iter()
        .map(|n| {
            key_bytes(n).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown key {n:?}: use Enter, Tab, Shift+Tab, Esc, Backspace, Delete, Insert, Space, \
                     Up, Down, Left, Right, Home, End, PageUp, PageDown, F1-F12, Ctrl+<letter>, Alt+<key>"
                )
            })
        })
        .collect()
}

/// The bytes a terminal expects for a named key. Enter is CR on every
/// platform: Windows consoles (ConPTY) end a line only on CR, and a Unix tty
/// maps CR to newline as it does for a Return key.
pub(super) fn key_bytes(name: &str) -> Option<String> {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    let fixed = match lower.as_str() {
        "enter" | "return" => "\r",
        "tab" => "\t",
        "shift+tab" => "\x1b[Z",
        "esc" | "escape" => "\x1b",
        "backspace" => "\x7f",
        "delete" | "del" => "\x1b[3~",
        "insert" => "\x1b[2~",
        "space" => " ",
        "up" => "\x1b[A",
        "down" => "\x1b[B",
        "right" => "\x1b[C",
        "left" => "\x1b[D",
        "home" => "\x1b[H",
        "end" => "\x1b[F",
        "pageup" => "\x1b[5~",
        "pagedown" => "\x1b[6~",
        "f1" => "\x1bOP",
        "f2" => "\x1bOQ",
        "f3" => "\x1bOR",
        "f4" => "\x1bOS",
        "f5" => "\x1b[15~",
        "f6" => "\x1b[17~",
        "f7" => "\x1b[18~",
        "f8" => "\x1b[19~",
        "f9" => "\x1b[20~",
        "f10" => "\x1b[21~",
        "f11" => "\x1b[23~",
        "f12" => "\x1b[24~",
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.to_string());
    }
    if let Some(rest) = lower.strip_prefix("ctrl+") {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return match c {
                'a'..='z' => Some(((c as u8 - b'a' + 1) as char).to_string()),
                '[' => Some("\x1b".to_string()),
                '\\' => Some("\x1c".to_string()),
                ']' => Some("\x1d".to_string()),
                '@' | ' ' => Some("\0".to_string()),
                _ => None,
            };
        }
    }
    if lower.starts_with("alt+") {
        let rest = &trimmed[4..];
        if rest.chars().count() == 1 {
            return Some(format!("\x1b{rest}"));
        }
        if let Some(k) = key_bytes(rest) {
            return Some(format!("\x1b{k}"));
        }
    }
    None
}

/// Typed text with every line break as CR, what a Return key sends: a lone
/// LF doesn't end a line in a Windows console.
pub(super) fn to_cr(text: &str) -> String {
    text.replace("\r\n", "\r").replace('\n', "\r")
}

/// Splits `s` into pieces of at most `max` characters, on char boundaries.
pub(super) fn chunks(s: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(max.max(1)).map(|c| c.iter().collect()).collect()
}

async fn write(cx: &ToolCtx<'_>, id: &str, bytes: &str) -> Result<()> {
    for (i, piece) in chunks(bytes, CHUNK_CHARS).iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(CHUNK_GAP).await;
        }
        let res = pty_shell::call("PtyShellInput", &json!({ "shell_id": id, "text": piece }), cx).await?;
        if !res.starts_with("wrote to shell") {
            anyhow::bail!("{res}");
        }
    }
    Ok(())
}

// ── read ─────────────────────────────────────────────────────────────────

struct Log {
    content: String,
    end: u64,
    truncated: bool,
}

async fn read_log(cx: &ToolCtx<'_>, id: &str, since: Option<u64>, tail_lines: u32) -> Result<Log> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let url = format!("{}/api/v1/ptyshell/read", cx.local_url.trim_end_matches('/'));
    let req = PtyShellReadRequest {
        shell_id: id.to_string(),
        agent_block_id: cx.block_id.to_string(),
        tail_lines: Some(tail_lines.max(1)),
        since,
    };
    let body = srv_post_json(cx.client, &url, cx.auth_key, &serde_json::to_value(&req)?, "Terminal read").await?;
    let r: PtyShellReadResponse = serde_json::from_value(body)?;
    Ok(Log { content: r.content, end: r.end, truncated: r.truncated })
}

async fn read_action(arguments: &Value, cx: &ToolCtx<'_>) -> Result<Value> {
    let id = resolve(arguments, cx).await?;
    let (running, exit_code) = status(cx, &id).await?;
    if known(&id).kind == Kind::Background {
        return Ok(json!({
            "terminal": id, "kind": "background", "running": running, "exit_code": exit_code,
            "note": "A background job's output streams into its shell row in this conversation; this reads its state only.",
        }));
    }
    let since = arguments.get("since").and_then(|v| v.as_u64());
    let tail_lines = arguments.get("tail_lines").and_then(|v| v.as_u64()).unwrap_or(80) as u32;
    let log = read_log(cx, &id, since, tail_lines).await?;
    let raw = arguments.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(json!({
        "terminal": id,
        "output": if raw { log.content } else { strip_ansi(&log.content) },
        "truncated": log.truncated,
        "since": log.end,
        "running": running,
        "exit_code": exit_code,
    }))
}

async fn status(cx: &ToolCtx<'_>, id: &str) -> Result<(bool, Option<i32>)> {
    require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
    let base = cx.local_url.trim_end_matches('/');
    match known(id).kind {
        Kind::Background => {
            let body = serde_json::to_value(ShellStatusRequest { shell_id: id.to_string() })?;
            let v = srv_post_json(cx.client, &format!("{base}/api/v1/shell/status"), cx.auth_key, &body, "Terminal status").await?;
            let r: ShellStatusResponse = serde_json::from_value(v)?;
            Ok((r.running, r.exit_code))
        }
        Kind::Pty => {
            let body = serde_json::to_value(PtyShellStatusRequest {
                shell_id: id.to_string(),
                agent_block_id: cx.block_id.to_string(),
            })?;
            let v = srv_post_json(cx.client, &format!("{base}/api/v1/ptyshell/status"), cx.auth_key, &body, "Terminal status").await?;
            let r: PtyShellStatusResponse = serde_json::from_value(v)?;
            Ok((r.running, r.exit_code))
        }
    }
}

/// Terminal output as plain text: escape sequences (CSI, OSC, DCS, two-byte
/// escapes) removed, a cursor move to another row read as a line break, CRLF as LF, a lone CR starting its line over (a
/// progress bar reads as its last state), backspace applied, other control
/// characters dropped.
pub(super) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut line = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for d in chars.by_ref() {
                        if ('@'..='~').contains(&d) {
                            // ConPTY often starts a new line by moving the cursor
                            // (CUP, HVP, CNL, VPA, CUD) instead of writing CRLF.
                            if matches!(d, 'H' | 'f' | 'E' | 'd' | 'B') && !line.trim().is_empty() {
                                out.push_str(line.trim_end());
                                out.push('\n');
                                line.clear();
                            }
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('_') | Some('^') => {
                    chars.next();
                    while let Some(d) = chars.next() {
                        if d == '\x07' {
                            break;
                        }
                        if d == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    line.clear();
                }
            }
            '\n' => {
                out.push_str(line.trim_end());
                out.push('\n');
                line.clear();
            }
            '\x08' => {
                line.pop();
            }
            '\t' => line.push('\t'),
            c if c.is_control() => {}
            c => line.push(c),
        }
    }
    out.push_str(&line);
    out
}

// ── wait ─────────────────────────────────────────────────────────────────

async fn wait_action(arguments: &Value, cx: &ToolCtx<'_>) -> Result<Value> {
    let id = resolve(arguments, cx).await?;
    let since = match arguments.get("since").and_then(|v| v.as_u64()) {
        Some(s) => s,
        None if known(&id).kind == Kind::Pty => read_log(cx, &id, None, 1).await?.end,
        None => 0,
    };
    let mut out = wait_on(arguments, cx, &id, since, false).await?;
    out["terminal"] = json!(id);
    Ok(out)
}

#[derive(Debug)]
enum Until {
    Pattern(regex::Regex),
    Quiet,
    Prompt,
    Exit,
}

fn parse_until(arguments: &Value) -> Result<Until> {
    let until = arguments.get("until").and_then(|v| v.as_str()).unwrap_or("");
    let pattern = arguments.get("pattern").and_then(|v| v.as_str());
    match until {
        "pattern" => {
            let p = pattern.ok_or_else(|| anyhow::anyhow!("until \"pattern\" needs pattern"))?;
            let fixed = arguments.get("fixed").and_then(|v| v.as_bool()).unwrap_or(false);
            let src = if fixed { regex::escape(p) } else { p.to_string() };
            Ok(Until::Pattern(regex::Regex::new(&src).map_err(|e| anyhow::anyhow!("pattern: {e}"))?))
        }
        "quiet" => Ok(Until::Quiet),
        "prompt" => Ok(Until::Prompt),
        "exit" => Ok(Until::Exit),
        "" => anyhow::bail!("wait needs until: \"pattern\", \"quiet\", \"prompt\" or \"exit\""),
        other => anyhow::bail!("unknown until {other:?}: use \"pattern\", \"quiet\", \"prompt\" or \"exit\""),
    }
}

/// Blocks (polling srv) until the condition holds or the timeout passes.
/// `skip_echo`: the output starts with the echo of text just typed, which a
/// pattern must not match (`type` with `text`).
async fn wait_on(arguments: &Value, cx: &ToolCtx<'_>, id: &str, since: u64, skip_echo: bool) -> Result<Value> {
    let until = parse_until(arguments)?;
    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);
    let quiet_ms = arguments.get("quiet_ms").and_then(|v| v.as_u64()).unwrap_or(1500);
    let background = known(id).kind == Kind::Background;
    let exit = matches!(until, Until::Exit);
    if background && !exit {
        anyhow::bail!("a background job can only be waited on with until \"exit\"");
    }
    let family = shell_family(known(id).connection.as_deref());
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    let mut last_len = usize::MAX;
    let mut last_change = Instant::now();
    loop {
        let elapsed = started.elapsed().as_millis() as u64;
        if exit {
            let (running, exit_code) = status(cx, id).await?;
            if !running {
                return Ok(json!({ "how": "exited", "exit_code": exit_code, "elapsed_ms": elapsed }));
            }
        } else {
            let log = read_log(cx, id, Some(since), READ_ALL_LINES).await?;
            let text = strip_ansi(&log.content);
            if text.len() != last_len {
                last_len = text.len();
                last_change = Instant::now();
            }
            let quiet_for = last_change.elapsed().as_millis() as u64;
            let searched = if skip_echo { after_first_line(&text) } else { text.as_str() };
            let found = match &until {
                Until::Pattern(re) => re.captures(searched).map(|c| {
                    c.iter().map(|m| m.map(|m| m.as_str().to_string())).collect::<Vec<_>>()
                }),
                Until::Quiet => (quiet_for >= quiet_ms).then(Vec::new),
                // The prompt is the terminal's last line now, whenever it was
                // printed: a shell already idle at it counts, not only output
                // that came after the call.
                Until::Prompt if quiet_for >= 300 => {
                    let now = strip_ansi(&read_log(cx, id, None, 5).await?.content);
                    at_prompt(&now, family).then(Vec::new)
                }
                Until::Prompt => None,
                Until::Exit => None,
            };
            if let Some(groups) = found {
                let how = match until {
                    Until::Pattern(_) => "matched",
                    Until::Quiet => "quiet",
                    _ => "prompt",
                };
                let mut out = json!({ "how": how, "elapsed_ms": elapsed, "since": log.end, "tail": tail(&text, 20) });
                if !groups.is_empty() {
                    out["match"] = json!(groups);
                }
                return Ok(out);
            }
            if Instant::now() >= deadline {
                return Ok(json!({ "how": "timeout", "elapsed_ms": elapsed, "since": log.end, "tail": tail(&text, 20) }));
            }
        }
        if Instant::now() >= deadline {
            return Ok(json!({ "how": "timeout", "elapsed_ms": started.elapsed().as_millis() as u64 }));
        }
        tokio::time::sleep(POLL).await;
    }
}

pub(super) fn after_first_line(text: &str) -> &str {
    text.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
}

pub(super) fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Whether the last non-empty line looks like the shell's prompt: cmd's
/// `C:\path>`, PowerShell's `PS C:\path>`, or a Unix prompt ending in $ # % > ❯.
pub(super) fn at_prompt(text: &str, family: ShellFamily) -> bool {
    let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) else { return false };
    let last = last.trim_end();
    match family {
        ShellFamily::Cmd => {
            let b = last.as_bytes();
            (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' && last.ends_with('>'))
                || (last.starts_with("PS ") && last.ends_with('>'))
        }
        ShellFamily::Posix | ShellFamily::Fish => ["$", "#", "%", ">", "❯"].iter().any(|p| last.ends_with(p)),
    }
}

// ── run ──────────────────────────────────────────────────────────────────

/// The command with an exit-code marker after it, in the shell's own syntax,
/// tagged with this run's `nonce` so no earlier run's marker is taken for it.
/// cmd expands `%ERRORLEVEL%` when it reads the line, before the command runs,
/// so `call` with `%^ERRORLEVEL%` defers it; fish has `$status` for `$?`,
/// braced so the variable name ends there. The typed line itself never
/// matches [`exit_marker`]: it has `%^ERRORLEVEL%`, `$?` or `{$status}`, not digits.
pub(super) fn with_exit_marker(command: &str, family: ShellFamily, nonce: &str) -> String {
    match family {
        ShellFamily::Cmd => format!("{command} & call echo __AMX_RC_{nonce}_%^ERRORLEVEL%__"),
        ShellFamily::Posix => format!("{command}; echo \"__AMX_RC_{nonce}_$?__\""),
        ShellFamily::Fish => format!("{command}; echo \"__AMX_RC_{nonce}_{{$status}}__\""),
    }
}

/// `command` run in `cwd`: the drawer is a long-lived shell, so `run`'s cwd
/// is a `cd` in front of the command (cmd's `/d` changes the drive too).
pub(super) fn in_dir(command: &str, cwd: &str, family: ShellFamily) -> String {
    match family {
        ShellFamily::Cmd => format!("cd /d \"{cwd}\" && {command}"),
        ShellFamily::Posix | ShellFamily::Fish => format!("cd \"{cwd}\" && {command}"),
    }
}

/// The key that clears a half-typed input line before `run` types: Esc in
/// cmd, Ctrl+U in a Unix shell (readline, zsh's line editor, fish).
pub(super) fn clear_line(family: ShellFamily) -> &'static str {
    match family {
        ShellFamily::Cmd => "\x1b",
        ShellFamily::Posix | ShellFamily::Fish => "\x15",
    }
}

/// Finds this run's marker in output after the echo of the typed command:
/// the command's own output (between the echo and the marker) and its exit code.
pub(super) fn exit_marker(text: &str, nonce: &str) -> Option<(String, i64)> {
    let re = regex::Regex::new(&format!(r"__AMX_RC_{}_(-?\d+)__", regex::escape(nonce))).ok()?;
    let body = after_first_line(text);
    let caps = re.captures(body)?;
    let m = caps.get(0)?;
    let code = caps[1].parse().ok()?;
    // Everything before the marker is output, including a last line that
    // didn't end in a newline (`printf x` puts the marker right after the x).
    let output = &body[..m.start()];
    Some((output.trim_end().to_string(), code))
}

async fn run_action(arguments: &Value, cx: &ToolCtx<'_>) -> Result<Value> {
    let command = str_arg(arguments, "command").ok_or_else(|| anyhow::anyhow!("run needs command"))?;
    if arguments.get("background").and_then(|v| v.as_bool()).unwrap_or(false) {
        let mut args = arguments.clone();
        args["cmd"] = json!(command);
        let id = shell::call("Shell", &args, cx).await?;
        remember(&id, Kind::Background, str_arg(arguments, "connection").map(str::to_string));
        return Ok(json!({
            "terminal": id,
            "kind": "background",
            "note": "Runs as a background job; its output streams into a shell row in this conversation. \
                     wait with until \"exit\" waits for it; close stops it.",
        }));
    }
    if command.contains('\n') {
        anyhow::bail!("run takes one command line; join steps with && (or use type for a script)");
    }
    let mut id = resolve(arguments, cx).await?;
    // A connection other than the drawer's goes through open, so srv answers
    // as it does for open: the drawer switches, or a conflict says why not.
    // Without this the command would run on whatever the drawer is on.
    if let Some(wanted) = str_arg(arguments, "connection") {
        let current = known(&id).connection.unwrap_or_else(|| "local".to_string());
        if wanted != current && str_arg(arguments, "terminal").is_none() {
            id = open(arguments, cx).await?["terminal"].as_str().unwrap_or_default().to_string();
        }
    }
    let family = shell_family(known(&id).connection.as_deref());
    let line = match str_arg(arguments, "cwd") {
        Some(cwd) => in_dir(command, cwd, family),
        None => command.to_string(),
    };
    let nonce = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    write(cx, &id, clear_line(family)).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let start = read_log(cx, &id, None, 1).await?.end;
    write(cx, &id, &format!("{}\r", with_exit_marker(&line, family, &nonce))).await?;
    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(120_000)
        .min(MAX_TIMEOUT_MS);
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    loop {
        let log = read_log(cx, &id, Some(start), READ_ALL_LINES).await?;
        let text = strip_ansi(&log.content);
        if let Some((output, exit_code)) = exit_marker(&text, &nonce) {
            let lines: Vec<&str> = output.lines().collect();
            let truncated = lines.len() > MAX_RUN_LINES;
            let output = lines[lines.len().saturating_sub(MAX_RUN_LINES)..].join("\n");
            return Ok(json!({
                "terminal": id,
                "exit_code": exit_code,
                "output": output,
                "truncated": truncated,
                "duration_ms": started.elapsed().as_millis() as u64,
                "since": log.end,
            }));
        }
        if Instant::now() >= deadline {
            return Ok(json!({
                "terminal": id,
                "how": "timeout",
                "note": "Still running (or waiting for input). Read it, type into it, or send keys [\"Ctrl+C\"].",
                "tail": tail(&text, 20),
                "since": log.end,
            }));
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
#[path = "../tests/terminal_tests.rs"]
mod tests;
