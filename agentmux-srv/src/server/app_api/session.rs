use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_session_activity_summary(engine, state);
    register_session_archive_handler(engine, state);
    register_session_restore_handler(engine, state);
    register_session_export_handler(engine, state);
}

fn register_session_archive_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_ARCHIVE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionArchiveData = serde_json::from_value(data)
                    .map_err(|e| format!("session:archive: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:archive");

                let archive_dir = session_archive::default_archive_dir()
                    .ok_or_else(|| "cannot determine home directory".to_string())?;

                let (archived_bytes, archived_at) = session_archive::archive_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                    &archive_dir,
                )?;

                Ok(Some(serde_json::to_value(&SessionArchiveResult {
                    block_id: cmd.block_id,
                    archived_bytes,
                    archived_at,
                }).unwrap()))
            })
        }),
    );
}

fn register_session_restore_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_RESTORE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionRestoreData = serde_json::from_value(data)
                    .map_err(|e| format!("session:restore: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:restore");

                let restored_bytes = session_archive::restore_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                Ok(Some(serde_json::to_value(&SessionRestoreResult {
                    block_id: cmd.block_id,
                    restored_bytes,
                }).unwrap()))
            })
        }),
    );
}

fn register_session_export_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_EXPORT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionExportData = serde_json::from_value(data)
                    .map_err(|e| format!("session:export: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:export");

                let (raw_bytes, line_count) = session_archive::read_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                let byte_count = raw_bytes.len() as u64;
                let content = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

                Ok(Some(serde_json::to_value(&SessionExportResult {
                    content,
                    line_count,
                    byte_count,
                }).unwrap()))
            })
        }),
    );
}

/// Ambient-call purpose tag for the per-turn activity summary. Also usable
/// as the token-usage/cost-dashboard category for this call site.
const AMBIENT_PURPOSE_ACTIVITY_SUMMARY: &str = "activity_summary";

fn empty_summary_result() -> serde_json::Value {
    serde_json::to_value(&ActivitySummaryResult { summary: String::new(), tokens: None }).unwrap()
}

fn register_session_activity_summary(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_ACTIVITY_SUMMARY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandActivitySummaryData = serde_json::from_value(data)
                    .map_err(|e| format!("session:activity_summary: {e}"))?;

                // Admit through the Ambient Model Call gateway BEFORE doing any
                // work: a stale (superseded) request does zero FileStore reads
                // or prompt building, not just skips the CLI spawn. See
                // docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md.
                let key = crate::ambient::AmbientCallKey::new(
                    cmd.block_id.clone(),
                    AMBIENT_PURPOSE_ACTIVITY_SUMMARY,
                );
                let guard = match crate::ambient::gateway().admit(key, cmd.generation) {
                    crate::ambient::Admission::Proceed(guard) => guard,
                    crate::ambient::Admission::StaleOnArrival => {
                        return Ok(Some(empty_summary_result()));
                    }
                };
                let cancel = guard.cancellation();

                let word_target = cmd.word_target.unwrap_or(7).max(3).min(20);

                let block: Block = wstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("session:activity_summary: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                // Read the last 32 KB of agent output from FileStore. EVENT_BLOCK_FILE
                // events have persist: 0 so the ring buffer is always empty — FileStore
                // is the only reliable source. We tail-read to avoid loading multi-MB
                // output files on every turn; 32 KB comfortably covers 30 stream-json lines.
                const TAIL_BYTES: i64 = 32 * 1024;
                let all_lines: Vec<String> = match filestore.stat(&cmd.block_id, "output") {
                    Ok(Some(ref wf)) if wf.size > 0 => {
                        let tail_offset = (wf.size - TAIL_BYTES).max(0);
                        match filestore.read_at(&cmd.block_id, "output", tail_offset, TAIL_BYTES) {
                            Ok((_, bytes)) => {
                                let text = String::from_utf8_lossy(&bytes);
                                text.lines()
                                    .filter(|l| !l.trim().is_empty())
                                    .map(|l| l.to_string())
                                    .collect()
                            }
                            _ => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                };

                let n = all_lines.len();
                let start = n.saturating_sub(30);
                let window: Vec<&str> = all_lines[start..].iter().map(|s| s.as_str()).collect();

                if window.is_empty() {
                    return Ok(Some(empty_summary_result()));
                }

                let extracted = extract_digest_text(&window);
                if extracted.is_empty() {
                    return Ok(Some(empty_summary_result()));
                }

                let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
                if cli_path.is_empty() {
                    tracing::debug!(block_id = %cmd.block_id, "session:activity_summary: no CLI path in meta");
                    return Ok(Some(empty_summary_result()));
                }

                let prompt = format!(
                    "Summarize in {word_target} words or fewer what is currently being worked on. \
                     Use a short terse phrase with no quotes or punctuation.\n\n\
                     Recent activity:\n\n{extracted}"
                );

                let (summary, tokens) =
                    invoke_cli_for_activity(&cli_path, &prompt, &block.meta, cancel).await
                        .unwrap_or_else(|e| {
                            tracing::debug!(block_id = %cmd.block_id, error = %e, "session:activity_summary: CLI failed or was superseded");
                            (String::new(), None)
                        });

                // guard is held until here so the in-flight entry stays
                // registered (and cancellable by a newer request) for the
                // full duration of the CLI call.
                drop(guard);

                // The frontend writes `term:ambient_summary` after receiving this
                // response so it can discard results from turns that were
                // superseded before they returned (belt-and-suspenders on top of
                // the gateway's own cancellation).
                Ok(Some(serde_json::to_value(&ActivitySummaryResult { summary, tokens }).unwrap()))
            })
        }),
    );
}

/// Invoke the Claude CLI with Haiku model for a lightweight per-turn activity
/// summary. Uses `--model claude-haiku-4-5-20251001` and a 15s timeout.
///
/// `cancel` is this call's Ambient Model Call gateway cancellation token — if
/// a newer request for the same `(block_id, "activity_summary")` key is
/// admitted while this one is still running, `cancel` fires and the child
/// process is killed immediately rather than left to run to completion (and
/// keep burning tokens) only to have its result discarded on arrival.
///
/// Returns the summary text plus token usage parsed from the CLI's `result`
/// stream-json line (same `usage` shape the main turn pipeline parses —
/// see `agents::translator::claude::parse_usage`), so ambient calls are
/// never silently excluded from token accounting.
pub(super) async fn invoke_cli_for_activity(
    cli_path: &str,
    prompt: &str,
    meta: &obj::MetaMapType,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(String, Option<crate::agents::TokenCounts>), String> {
    let auth_env: std::collections::HashMap<String, String> = match meta.get("cmd:env") {
        Some(serde_json::Value::Object(obj_map)) => obj_map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        _ => std::collections::HashMap::new(),
    };

    let mut child = crate::server::cli_handlers::make_cli_cmd(cli_path)
        .args(["-p", "--output-format", "stream-json", "--verbose",
               "--model", "claude-haiku-4-5-20251001"])
        .envs(&auth_env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn activity CLI: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await
            .map_err(|e| format!("activity CLI stdin write: {e}"))?;
        stdin.shutdown().await
            .map_err(|e| format!("activity CLI stdin shutdown: {e}"))?;
    }

    // `child.wait()` only borrows (unlike `wait_with_output()`, which
    // consumes `child` by value and would make the cancel branch's
    // `child.kill()` below impossible). Drain stdout concurrently via a
    // separate task — same rationale `wait_with_output()` itself uses
    // internally — so a chatty response can't fill the OS pipe buffer and
    // deadlock the child waiting to write while nothing is reading.
    let mut stdout_pipe = child.stdout.take()
        .ok_or_else(|| "activity CLI: no stdout pipe".to_string())?;
    let stdout_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf).await;
        buf
    });

    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err("cancelled: superseded by a newer activity-summary request".to_string());
        }
        result = tokio::time::timeout(std::time::Duration::from_secs(15), child.wait()) => {
            result.map_err(|_| "activity CLI timed out after 15s".to_string())?
                .map_err(|e| format!("activity CLI wait: {e}"))?
        }
    };

    if !status.success() {
        return Err(format!("activity CLI exited with status {status}"));
    }

    let stdout_bytes = stdout_task.await
        .map_err(|e| format!("activity CLI stdout reader task: {e}"))?;
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let mut last_text = String::new();
    let mut tokens: Option<crate::agents::TokenCounts> = None;
    for line in stdout.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match val.get("type").and_then(|v| v.as_str()) {
            Some("assistant") => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                last_text = text.trim().to_string();
                            }
                        }
                    }
                }
            }
            Some("result") => {
                tokens = Some(crate::agents::translator::claude::parse_usage(val.get("usage")));
            }
            _ => {}
        }
    }

    if last_text.is_empty() {
        return Err("no text in activity CLI response".to_string());
    }

    Ok((last_text, tokens))
}

/// Extract meaningful text from raw stream-json lines for digest summarization.
/// Skips system/result events and raw stream_event deltas; extracts assistant text
/// and tool call summaries.
pub(super) fn extract_digest_text(lines: &[&str]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for line in lines {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };

        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "assistant" => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    parts.push(format!("[assistant] {}", trimmed));
                                }
                            }
                        } else if btype == "tool_use" {
                            let tool_name = block.get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            parts.push(format!("[tool] {}", tool_name));
                        }
                    }
                }
            }
            "user" => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "tool_result" {
                            let is_error = block.get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if is_error {
                                let err_text = block.get("content")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("(error)")
                                    .chars().take(120).collect::<String>();
                                parts.push(format!("[error] {}", err_text));
                            }
                        } else if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    parts.push(format!("[user] {}", trimmed));
                                }
                            }
                        }
                    }
                }
            }
            "result" => {
                if let Some(cost) = val.get("total_cost_usd").and_then(|v| v.as_f64()) {
                    if let Some(turns) = val.get("num_turns").and_then(|v| v.as_u64()) {
                        parts.push(format!("[summary] {} turns, ${:.4} total cost", turns, cost));
                    }
                }
            }
            // Skip: system, stream_event (deltas), rate_limit_event
            _ => {}
        }
    }

    parts.join("\n")
}
