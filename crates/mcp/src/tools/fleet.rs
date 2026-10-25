// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-to-agent messaging and fleet operations. Not advertised: `Agents`
//! (agents_tool.rs), `Message` (message_tool.rs) and `Stop` (stop_tool.rs)
//! call these by their old names.

use super::*;

/// `urgent: true` on a send or broadcast: deliver now, even to a session that
/// has been idle for about an hour (srv holds a routine message for such a
/// session until it is next active).
fn urgent_priority(arguments: &Value) -> Option<String> {
    arguments.get("urgent").and_then(|v| v.as_bool()).unwrap_or(false).then(|| "urgent".to_string())
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, client, .. } = *cx;
    match name {
        "SendMessage" => {
            let to = arguments
                .get("to")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to"))?;
            let message = arguments
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let source_agent = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());

            let url = format!(
                "{}/agentmux/reactive/inject",
                local_url.trim_end_matches('/')
            );
            // Files go into this agent's attachment store first; the jekt
            // names them (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §3.1).
            let files = match super::send_files::files_arg(arguments)? {
                Some(paths) => {
                    let local = super::send_files::check(&paths)?;
                    Some(super::send_files::upload(local, cx).await?)
                }
                None => None,
            };
            let sent_names: Vec<String> = files.iter().flatten().map(|f| f.name.clone()).collect();
            let mut req = signed_jekt_with_files(source_agent, to, message, files);
            req.priority = urgent_priority(arguments);

            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("inject failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            let outcome = send_message_outcome(to, &result)?;
            Ok(if sent_names.is_empty() {
                outcome
            } else {
                // Fewer arrive when a file fails its hash on the way or the
                // receiver withholds them; its message says which.
                let arrived = result.get("files_delivered").and_then(Value::as_u64);
                match arrived.filter(|n| (*n as usize) < sent_names.len()) {
                    Some(n) => format!("{outcome}\nFiles: {} ({n} of {} arrived)", sent_names.join(", "), sent_names.len()),
                    None => format!("{outcome}\nFiles: {}", sent_names.join(", ")),
                }
            })
        }
        "DiscoverAgents" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetList" => {
            // Thin fleet-framed alias of DiscoverAgents — identical call,
            // see this tool's own doc comment for why a separate tool
            // exists despite reusing the exact same endpoint.
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetBroadcast" => {
            let targets = arguments
                .get("targets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: targets"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                anyhow::bail!("targets must be a non-empty array of block_id (host/cross-channel) or agent name (LAN/WAN) strings");
            }
            let message = arguments
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let source_agent = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());

            // Files go into this agent's attachment store once; each target's
            // jekt names them, signed for that target, exactly as `send` does
            // (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §3.1). srv then applies
            // the per-tier rules to each one: a verified sender, the LAN size
            // limits.
            let files = match super::send_files::files_arg(arguments)? {
                Some(paths) => {
                    let local = super::send_files::check(&paths)?;
                    Some(super::send_files::upload(local, cx).await?)
                }
                None => None,
            };
            let sent_names: Vec<String> = files.iter().flatten().map(|f| f.name.clone()).collect();

            // `/agentmux/reactive/inject`'s `target_agent` resolves by
            // agent UID or registered AGENT NAME only (`resolve_target`,
            // crates/srv/src/backend/reactive/handler.rs) — never by
            // block_id, even though this tool's own advertised contract
            // (MESSAGE_TOOL, broadcast) is block_id values from Agents list, to
            // stay consistent with Stop agents' targeting scheme. The
            // WS-RPC path (`fleet_broadcast_impl`, agentmux-srv) already
            // resolves block_id -> agent_id via `get_agent_by_block` before
            // injecting; this MCP path talks to srv over plain HTTP with no
            // access to that in-process registry, so it resolves the same
            // way DiscoverAgents/FleetList already do: read
            // `/agentmux/discovery`'s `host.addressable` (each entry already
            // carries both `agent_id` and a live `block_id`) and map through
            // it before signing/injecting (Codex P1 + reagent P0, PR #2687
            // review — every advertised call failed "agent not found"
            // without this).
            //
            // REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md:
            // `host.addressable` alone missed `host.cross_channel` entries
            // (a different channel on this SAME host — those genuinely have
            // a `block_id` in this host's namespace, discovery just wasn't
            // being read for it) and LAN/WAN entries (which have NO
            // `block_id` at all — only an agent name, since they're not
            // local blocks). Both are folded in below: `cross_channel`
            // extends the same block_id->name map (identical shape), and
            // any target string that doesn't match ANY known block_id falls
            // through to being used AS a literal agent name — safe because
            // `/agentmux/reactive/inject`'s own cross-tier cascade
            // (cross-channel -> LAN -> WAN muxbus relay,
            // `server/reactive.rs`) already resolves by name across every
            // tier; an invalid name just fails the same "not found" way it
            // always did.
            let discovery_url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let discovery_resp = client
                .get(&discovery_url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("discovery request failed: {e}"))?;
            if !discovery_resp.status().is_success() {
                let status = discovery_resp.status();
                let text = discovery_resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }
            let discovery: Value = discovery_resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("discovery response parse failed: {e}"))?;
            let block_to_agent = build_block_to_agent_map(&discovery);

            // Same signed single-target delivery `Message` send uses, looped
            // once per target — this process signs with its copy of the
            // agent's keys (srv mints, stores and serves them), so signing
            // happens here (see this tool's doc comment). Never a single
            // aggregate result: per-target success/failure is collected below regardless of
            // how many targets fail. Sent in chunks with a pause between
            // them — `/agentmux/reactive/inject` shares ReactiveHandler's
            // global rate limiter (10/sec, hard reset per second, not a
            // smooth refill — crates/srv/src/backend/reactive/mod.rs's
            // RATE_LIMIT_MAX), and a tight loop past ~10 targets would
            // otherwise deterministically fail the tail of any larger
            // broadcast with "rate limit exceeded" (Codex P1, same review).
            // Mirrors `fleet_broadcast_impl`'s own chunking constants —
            // kept in sync by hand since this is a separate process/crate
            // with no dependency on agentmux-srv internals.
            const BROADCAST_CHUNK_SIZE: usize = 10;
            const BROADCAST_CHUNK_PAUSE: std::time::Duration = std::time::Duration::from_millis(1100);

            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let priority = urgent_priority(arguments);
            let mut succeeded: Vec<String> = Vec::new();
            let mut held: Vec<serde_json::Value> = Vec::new();
            let mut failed: Vec<serde_json::Value> = Vec::new();
            // Delivered targets that got fewer files than were sent.
            let mut files_short: Vec<serde_json::Value> = Vec::new();
            for (chunk_idx, chunk) in targets.chunks(BROADCAST_CHUNK_SIZE).enumerate() {
                if chunk_idx > 0 {
                    tokio::time::sleep(BROADCAST_CHUNK_PAUSE).await;
                }
                for target in chunk {
                    let target = target.clone();
                    // LAN/WAN discovery entries carry no block_id at all
                    // (they're not local blocks) — a target that doesn't
                    // match any known block_id is used AS the agent name
                    // directly, letting `/agentmux/reactive/inject`'s own
                    // cross-tier cascade attempt it. This can never make a
                    // genuinely-wrong block_id succeed silently: it still
                    // fails, just via the inject endpoint's own "agent not
                    // found" rather than this pre-check.
                    let target_agent = block_to_agent.get(&target).cloned().unwrap_or_else(|| target.clone());
                    let mut req = signed_jekt_with_files(source_agent.clone(), &target_agent, message, files.clone());
                    req.priority = priority.clone();
                    let outcome = async {
                        let resp = client
                            .post(&url)
                            .header(AUTH_KEY_HEADER, auth_key)
                            .json(&req)
                            .send()
                            .await
                            .map_err(|e| format!("request failed: {e}"))?;
                        if !resp.status().is_success() {
                            let status = resp.status();
                            let text = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {status} — {text}"));
                        }
                        let result: Value = resp
                            .json()
                            .await
                            .map_err(|e| format!("response parse failed: {e}"))?;
                        if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
                            Ok(Sent::Delivered(result.get("files_delivered").and_then(Value::as_u64)))
                        } else if result.get("held_reason").and_then(|v| v.as_str()) == Some(crate::tool_helpers::HELD_IDLE) {
                            // Kept for an idle session, not lost: say so apart from the failures.
                            Ok(Sent::HeldIdle)
                        } else {
                            Err(result
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error")
                                .to_string())
                        }
                    }
                    .await;
                    match outcome {
                        Ok(Sent::Delivered(arrived)) => {
                            // Fewer arrive when a file fails its hash on the
                            // way or the receiver withholds them, as in `send`.
                            if let Some(n) = arrived.filter(|n| (*n as usize) < sent_names.len()) {
                                files_short.push(json!({ "id": target, "arrived": n }));
                            }
                            succeeded.push(target);
                        }
                        Ok(Sent::HeldIdle) => held.push(json!({ "id": target, "held": "idle" })),
                        Err(error) => failed.push(json!({ "id": target, "error": error })),
                    }
                }
            }

            let mut result = json!({ "succeeded": succeeded, "failed": failed });
            if !held.is_empty() {
                result["held"] = json!(held);
                result["held_note"] = json!(
                    "held: their session has been idle, so the message waits until it is next active; send it again with urgent: true to wake them now"
                );
            }
            if !sent_names.is_empty() {
                result["files"] = json!(sent_names);
                result["files_short"] = json!(files_short);
            }
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetBulkStop" => {
            let targets = arguments
                .get("targets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: targets"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                anyhow::bail!("targets must be a non-empty array of block_id strings");
            }
            let signal = arguments.get("signal").and_then(|v| v.as_str()).map(str::to_string);
            let staged = arguments.get("staged").cloned();

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/fleet/bulk-stop", local_url.trim_end_matches('/'));
            let mut body = json!({ "targets": targets, "signal": signal });
            if let Some(staged) = staged {
                body["staged"] = staged;
            }
            // Names this agent on the user's banner and in the audit log.
            if let Ok(auth) = sign_ui_automation_auth() {
                body["auth"] = serde_json::to_value(auth).unwrap_or(Value::Null);
            }
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("fleet bulk-stop failed: HTTP {status} — {text}");
            }

            let mut result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            if result["status"] == "pending_user_override" {
                // Each running target's user had 15 s to keep it (§6.5). srv
                // runs the windows in parallel, so waiting on them one after
                // another costs no extra time.
                let pending = result["pending"].as_array().cloned().unwrap_or_default();
                let mut outcomes = Vec::with_capacity(pending.len());
                for p in &pending {
                    let request_id = p["request_id"].as_str().unwrap_or_default();
                    let now = await_shutdown(client, local_url, auth_key, request_id, true).await;
                    outcomes.push((p["block_id"].as_str().unwrap_or_default().to_string(), now));
                }
                let note = result["note"].as_str().map(str::to_string);
                result = merge_bulk_stop_outcomes(result, outcomes);
                add_stop_notes(&mut result, note, &pending);
            } else if result.get("kept_by_user").is_none() {
                // Nothing waited (all refused or already gone): the same shape.
                result["kept_by_user"] = json!([]);
            }

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        _ => Err(not_in_family()),
    }
}

/// How one broadcast target took its jekt.
enum Sent {
    /// Delivered; carries how many of its files arrived, when srv said.
    Delivered(Option<u64>),
    /// Kept for an idle session, not lost.
    HeldIdle,
}

#[cfg(test)]
mod broadcast_files_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    /// A stand-in for srv: discovery knows no one, the attachment store keeps
    /// whatever it is given, and inject records each request and says
    /// `delivered` files arrived.
    struct StubSrv {
        url: String,
        injected: Arc<Mutex<Vec<Value>>>,
    }

    fn read_request(conn: &mut std::net::TcpStream) -> (String, Vec<u8>) {
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        let head_end = loop {
            if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            let n = conn.read(&mut buf).unwrap();
            assert!(n > 0, "connection closed before the headers ended");
            raw.extend_from_slice(&buf[..n]);
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
        let request_line = head.lines().next().unwrap_or_default().to_string();
        let length = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok());
        let chunked = head.contains("transfer-encoding: chunked");
        // An upload is streamed, so it arrives chunked.
        while match length {
            Some(n) => raw.len() < head_end + n,
            None => chunked && !raw.ends_with(b"0\r\n\r\n"),
        } {
            let n = conn.read(&mut buf).unwrap();
            assert!(n > 0, "connection closed before the body ended");
            raw.extend_from_slice(&buf[..n]);
        }
        (request_line, raw[head_end..].to_vec())
    }

    fn stub_srv(delivered: u64) -> StubSrv {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let injected = Arc::new(Mutex::new(Vec::new()));
        let seen = injected.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let mut conn = conn.unwrap();
                let seen = seen.clone();
                std::thread::spawn(move || {
                    let (line, body) = read_request(&mut conn);
                    let answer = if line.contains("/agentmux/discovery") {
                        json!({ "host": { "addressable": [] } })
                    } else if line.contains("/attachments/upload") {
                        json!({ "id": "ab".repeat(32), "bytes": 5 })
                    } else {
                        seen.lock().unwrap().push(serde_json::from_slice(&body).unwrap());
                        json!({ "success": true, "files_delivered": delivered })
                    }
                    .to_string();
                    let _ = write!(
                        conn,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                        answer.len()
                    );
                });
            }
        });
        StubSrv { url, injected }
    }

    async fn broadcast(srv: &StubSrv, args: Value) -> Result<Value> {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: &srv.url, auth_key: "key", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let out = call("FleetBroadcast", &args, &cx).await?;
        Ok(serde_json::from_str(&out)?)
    }

    fn file_in(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("report.txt");
        std::fs::write(&path, b"hello").unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Every target's jekt names the files, under a msgid that binds them, so
    /// a LAN recipient gets what a direct send gives it.
    #[tokio::test]
    async fn each_target_gets_the_files_signed_into_its_jekt() {
        use agentmux_common::jekt_files::{msgid_binds, JektFile};
        let (srv, dir) = (stub_srv(1), tempfile::tempdir().unwrap());
        let out = broadcast(&srv, json!({ "targets": ["lan-a", "lan-b"], "message": "look", "files": [file_in(&dir)] })).await.unwrap();

        assert_eq!(out["succeeded"], json!(["lan-a", "lan-b"]));
        assert_eq!(out["failed"], json!([]));
        assert_eq!(out["files"], json!(["report.txt"]));
        assert_eq!(out["files_short"], json!([]));
        let sent = srv.injected.lock().unwrap();
        assert_eq!(sent.len(), 2);
        for (req, target) in sent.iter().zip(["lan-a", "lan-b"]) {
            assert_eq!(req["target_agent"], target);
            let files: Vec<JektFile> = serde_json::from_value(req["files"].clone()).expect("the jekt carries its files");
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].name, "report.txt");
            assert!(msgid_binds(req["request_id"].as_str().unwrap(), &files));
        }
        assert_ne!(sent[0]["request_id"], sent[1]["request_id"], "each target is signed on its own");
    }

    /// A target that took the message but not every file is named, not
    /// counted as a plain success.
    #[tokio::test]
    async fn a_target_that_got_fewer_files_is_named() {
        let (srv, dir) = (stub_srv(0), tempfile::tempdir().unwrap());
        let out = broadcast(&srv, json!({ "targets": ["lan-a"], "message": "look", "files": [file_in(&dir)] })).await.unwrap();
        assert_eq!(out["succeeded"], json!(["lan-a"]));
        assert_eq!(out["files_short"], json!([{ "id": "lan-a", "arrived": 0 }]));
    }

    /// A file that can't be sent stops the broadcast before any target is
    /// messaged.
    #[tokio::test]
    async fn a_missing_file_sends_nothing() {
        let (srv, dir) = (stub_srv(1), tempfile::tempdir().unwrap());
        let missing = dir.path().join("nope.txt").to_string_lossy().into_owned();
        let err = broadcast(&srv, json!({ "targets": ["lan-a"], "message": "look", "files": [missing] })).await.unwrap_err().to_string();
        assert!(err.contains("nope.txt"), "{err}");
        assert!(srv.injected.lock().unwrap().is_empty());
    }

    /// Without files a broadcast is what it was.
    #[tokio::test]
    async fn a_broadcast_without_files_carries_none() {
        let srv = stub_srv(0);
        let out = broadcast(&srv, json!({ "targets": ["lan-a"], "message": "hi" })).await.unwrap();
        assert_eq!(out, json!({ "succeeded": ["lan-a"], "failed": [] }));
        let sent = srv.injected.lock().unwrap();
        assert!(sent[0].get("files").map_or(true, Value::is_null));
    }
}

#[cfg(test)]
mod urgent_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urgent_true_is_the_only_thing_that_sets_a_priority() {
        assert_eq!(urgent_priority(&json!({ "urgent": true })), Some("urgent".to_string()));
        assert_eq!(urgent_priority(&json!({ "urgent": false })), None);
        assert_eq!(urgent_priority(&json!({})), None);
        assert_eq!(urgent_priority(&json!({ "urgent": "yes" })), None, "not a boolean");
    }
}
