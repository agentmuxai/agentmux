// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Panes, tabs and windows: open, name, arrange, focus, close; the agent's own identity and lifecycle.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "OpenEditor" => {
            let connection = connection_arg(arguments);
            let file = arguments
                .get("file")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: file"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let split = arguments
                .get("split")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(*s, "right" | "left" | "down" | "up"))
                .unwrap_or("right");

            let url = format!("{}/api/v1/pane/open", local_url.trim_end_matches('/'));
            // Place the editor relative to the calling agent pane when we know
            // its block id (AGENTMUX_BLOCKID); otherwise the sidecar inserts it
            // at the tab root.
            let (split_direction, split_reference_block_id) = if block_id.is_empty() {
                (None, None)
            } else {
                (Some(split.to_string()), Some(block_id.to_string()))
            };
            let req = PaneOpenRequest {
                view: "editor".to_string(),
                file: Some(file.to_string()),
                focus: Some(true),
                split_direction,
                split_reference_block_id,
                title: arguments.get("title").and_then(|v| v.as_str()).map(str::to_string),
                // `collapse_tree: true` → open with the file-tree sidebar collapsed.
                // Maps to the editor's `tree_expanded` meta (collapsed == not expanded).
                tree_expanded: if arguments.get("collapse_tree").and_then(|v| v.as_bool()) == Some(true) {
                    Some(false)
                } else {
                    None
                },
                // `floating: true` → open in a floating window instead of a docked split.
                floating: if arguments.get("floating").and_then(|v| v.as_bool()) == Some(true) {
                    Some(true)
                } else {
                    None
                },
                url: None,
                cwd: None,
                tab_id: None,
                // Reuse an already-open Editor pane in this agent's own tab
                // instead of always spawning a new one — the explicit opt-in
                // only OpenEditor sets (see reuse_editor_pane's doc comment on
                // PaneOpenRequest for why this can't be inferred from
                // split_reference_block_id alone).
                reuse_editor_pane: Some(true),
                select: None,
                // On an SSH host: srv asks the user to allow it, checking this
                // agent's signed identity (remote terminals spec §8.2).
                auth: if connection.is_some() { Some(sign_ui_automation_auth()?) } else { None },
                connection,
            };

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
                anyhow::bail!("pane.open failed: HTTP {status} — {text}");
            }

            let result: PaneOpenResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(format!("Opened {file} in editor pane (block {})", result.block_id))
        }
        "OpenMedia" => {
            let file = arguments
                .get("file")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: file"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let split = arguments
                .get("split")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(*s, "right" | "left" | "down" | "up"))
                .unwrap_or("right");

            let url = format!("{}/api/v1/pane/open", local_url.trim_end_matches('/'));
            // Place the media pane relative to the calling agent pane when we
            // know its block id (AGENTMUX_BLOCKID); otherwise the sidecar
            // inserts it at the tab root.
            let (split_direction, split_reference_block_id) = if block_id.is_empty() {
                (None, None)
            } else {
                (Some(split.to_string()), Some(block_id.to_string()))
            };
            let req = PaneOpenRequest {
                view: "media".to_string(),
                file: Some(file.to_string()),
                focus: Some(true),
                split_direction,
                split_reference_block_id,
                title: arguments.get("title").and_then(|v| v.as_str()).map(str::to_string),
                // The Media pane has no file-tree sidebar, unlike Editor.
                tree_expanded: None,
                // `floating: true` → open in a floating window instead of a docked split.
                floating: if arguments.get("floating").and_then(|v| v.as_bool()) == Some(true) {
                    Some(true)
                } else {
                    None
                },
                url: None,
                cwd: None,
                tab_id: None,
                reuse_editor_pane: None, // view != "editor" — irrelevant here
                select: None,
                connection: None,
                auth: None,
            };

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
                anyhow::bail!("pane.open failed: HTTP {status} — {text}");
            }

            let result: PaneOpenResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(format!("Opened {file} in media pane (block {})", result.block_id))
        }
        // A Files pane on a folder, optionally with entries selected.
        // Navigation only (SPEC_FILE_BROWSER_PANE_2026_10_01.md §8.1, §9:
        // agents don't mutate through this pane).
        "OpenFiles" => {
            let connection = connection_arg(arguments);
            let path = arguments
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;
            let select: Vec<String> = arguments
                .get("select")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let split = arguments
                .get("split")
                .and_then(|v| v.as_str())
                .filter(|s| matches!(*s, "right" | "left" | "down" | "up"))
                .unwrap_or("right");

            let url = format!("{}/api/v1/pane/open", local_url.trim_end_matches('/'));
            let (split_direction, split_reference_block_id) = if block_id.is_empty() {
                (None, None)
            } else {
                (Some(split.to_string()), Some(block_id.to_string()))
            };
            let req = PaneOpenRequest {
                view: "files".to_string(),
                file: Some(path.to_string()),
                focus: Some(true),
                split_direction,
                split_reference_block_id,
                title: arguments.get("title").and_then(|v| v.as_str()).map(str::to_string),
                tree_expanded: None,
                floating: if arguments.get("floating").and_then(|v| v.as_bool()) == Some(true) {
                    Some(true)
                } else {
                    None
                },
                url: None,
                cwd: None,
                tab_id: None,
                reuse_editor_pane: None, // view != "editor" — irrelevant here
                select: if select.is_empty() { None } else { Some(select) },
                // On an SSH host: srv asks the user to allow it, checking this
                // agent's signed identity (remote terminals spec §8.2).
                auth: if connection.is_some() { Some(sign_ui_automation_auth()?) } else { None },
                connection,
            };

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
                anyhow::bail!("pane.open failed: HTTP {status} — {text}");
            }

            let result: PaneOpenResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(format!("Opened {path} in a Files pane (block {})", result.block_id))
        }
        "OpenAgent" => {
            let agent_id = arguments
                .get("agent_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent_id"))?;
            let tab_id = arguments.get("tab_id").and_then(|v| v.as_str()).map(str::to_string);
            let focus = arguments.get("focus").and_then(|v| v.as_bool());

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/agent/open", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "tab_id": tab_id,
                "focus": focus,
            });
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
                anyhow::bail!("agent open failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "WhoAmI" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            if block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — cannot resolve this agent's pane. Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/self", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .query(&[("block_id", block_id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("self lookup failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SetName" => {
            let target = arguments
                .get("target")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: target"))?;
            let new_name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;
            let target_id = arguments
                .get("target_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            // block_id is only needed for own-context resolution; when
            // target_id is explicit we can reach any element without it.
            if target_id.is_none() && block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — pass target_id to name a specific element, or open this \
                     agent in an AgentMux pane to default to your own."
                );
            }

            // window/tab/workspace POST {"name": …} to /…/name; pane POSTs
            // {"title": …} to /pane/title. `label` and `resp_field` are the
            // human-facing echo and the response key used to surface the
            // server-applied value (e.g. a window name clamped to 64 chars).
            let own_block = if block_id.is_empty() { None } else { Some(block_id.to_string()) };
            let (path, label, resp_field, body) = match target {
                "window" => ("window/name", "Window name", "name", serde_json::to_value(WindowNameRequest {
                    block_id: own_block,
                    name: new_name.to_string(),
                    window_id: target_id,
                })?),
                "tab" => ("tab/name", "Tab name", "name", serde_json::to_value(TabNameRequest {
                    block_id: own_block,
                    tab_id: target_id,
                    name: new_name.to_string(),
                })?),
                "workspace" => ("workspace/name", "Workspace name", "name", serde_json::to_value(WorkspaceNameRequest {
                    block_id: own_block,
                    workspace_id: target_id,
                    name: new_name.to_string(),
                })?),
                "pane" => ("pane/title", "Pane title", "title", serde_json::to_value(PaneTitleRequest {
                    block_id: target_id.or(own_block),
                    title: new_name.to_string(),
                })?),
                other => anyhow::bail!(
                    "invalid target '{other}' — expected one of: window, tab, pane, workspace"
                ),
            };
            let url = format!("{}/api/v1/{path}", local_url.trim_end_matches('/'));
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
                anyhow::bail!("{label} change failed: HTTP {status} — {text}");
            }
            // Surface the server-applied value (e.g. a window name clamped to 64
            // chars) when the endpoint echoes it back; fall back to the request.
            let applied = resp
                .json::<Value>()
                .await
                .ok()
                .and_then(|v| v.get(resp_field).and_then(|s| s.as_str()).map(str::to_string))
                .unwrap_or_else(|| new_name.to_string());
            Ok(format!("{label} set to \"{applied}\""))
        }
        "Layout" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("layout");
            let path = match query {
                "layout" => "layout",
                "windows" => "windows",
                "workspaces" => "workspaces",
                "tabs" => "tabs",
                other => anyhow::bail!(
                    "invalid query '{other}' — expected one of: layout, windows, workspaces, tabs"
                ),
            };
            let url = format!("{}/api/v1/{path}", local_url.trim_end_matches('/'));
            let mut reqb = client.get(&url).header(AUTH_KEY_HEADER, auth_key);
            // The "tabs" query scopes to the caller's own workspace when we know it.
            if query == "tabs" && !block_id.is_empty() {
                reqb = reqb.query(&[("block_id", block_id)]);
            }
            let resp = reqb
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("Layout({query}) failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SetActiveTab" => {
            let tab_id = arguments
                .get("tab_id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: tab_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/tab/activate", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&TabActivateRequest { tab_id: tab_id.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("set active tab failed: HTTP {status} — {text}");
            }
            Ok(format!("Switched to tab {tab_id}"))
        }
        "NewTab" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let name = arguments.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let url = format!("{}/api/v1/tab/new", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&TabNewRequest {
                    block_id: Some(block_id.to_string()),
                    workspace_id: None,
                    name: if name.is_empty() { None } else { Some(name.to_string()) },
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("new tab failed: HTTP {status} — {text}");
            }
            Ok(if name.is_empty() {
                "Opened a new tab".to_string()
            } else {
                format!("Opened new tab \"{name}\"")
            })
        }
        "FocusWindow" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let window_id = arguments.get("window_id").and_then(|v| v.as_str()).unwrap_or("");
            let url = format!("{}/api/v1/window/focus", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&WindowFocusRequest {
                    block_id: Some(block_id.to_string()),
                    window_id: if window_id.is_empty() { None } else { Some(window_id.to_string()) },
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("focus window failed: HTTP {status} — {text}");
            }
            Ok("Focused window".to_string())
        }
        "ClosePane" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let target_block_id = arguments.get("block_id").and_then(|v| v.as_str()).map(str::to_string);
            let reason = arguments.get("reason").and_then(|v| v.as_str()).map(str::to_string);
            let url = format!("{}/api/v1/agent/pane/close", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&ClosePaneRequest { auth, block_id: target_block_id.clone(), reason })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("close failed: HTTP {status} — {text}");
            }
            if resp.status().as_u16() == 202 {
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                let request_id = body["request_id"].as_str().unwrap_or_default();
                return match target_block_id.as_deref() {
                    // Another agent's pane: its user had 15 s to keep it (§6.5).
                    Some(b) => {
                        let now = await_shutdown(client, local_url, auth_key, request_id, true).await;
                        close_pane_outcome(b, &now)
                    }
                    // Yourself (§7): as QuitSelf, done once it's proceeding —
                    // the quit waits for this very turn to end.
                    None => {
                        let now = await_shutdown(client, local_url, auth_key, request_id, false).await;
                        match now["status"].as_str().unwrap_or("") {
                            "" | "pending" => anyhow::bail!("ClosePane: no answer on the pending shutdown {request_id}"),
                            state => quit_self_outcome(state, &now),
                        }
                    }
                };
            }
            match target_block_id {
                Some(b) => Ok(format!("Closed pane {b:?}")),
                // srv's only 200 for the no-argument form (§7): a quit already
                // scheduled or under way.
                None => quit_self_result(200, &Value::Null),
            }
        }
        "QuitSelf" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let field = |k: &str| {
                arguments
                    .get(k)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| anyhow::anyhow!("missing required parameter: {k}"))
            };
            let (reason, user_instruction) = (field("reason")?, field("user_instruction")?);
            let auth = sign_ui_automation_auth()?;
            let url = format!("{}/api/v1/agent/self/quit", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&QuitSelfRequest { auth, reason, user_instruction })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status.as_u16() != 202 || body["status"] != "pending_user_override" {
                return quit_self_result(status.as_u16(), &body);
            }
            // Not the user's own ask: the user has 15 s to keep this agent
            // (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). Poll for the answer —
            // one long request would outlast the client's timeout.
            let request_id = body["request_id"].as_str().unwrap_or_default();
            // `proceeding` is final for QuitSelf: the shutdown waits for this
            // very turn to end, so waiting on here would hold it up.
            let now = await_shutdown(client, local_url, auth_key, request_id, false).await;
            match now["status"].as_str().unwrap_or("") {
                "" | "pending" => anyhow::bail!("QuitSelf: no answer on the pending shutdown {request_id}"),
                state => quit_self_outcome(state, &now),
            }
        }
        "RegisterDevServer" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let project = arguments
                .get("project")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: project"))?
                .to_string();
            let port = arguments
                .get("port")
                .and_then(|v| v.as_u64())
                .filter(|p| *p > 0 && *p <= u16::MAX as u64)
                .ok_or_else(|| anyhow::anyhow!("missing or invalid required parameter: port (must be 1-65535)"))?
                as u16;
            let url = format!(
                "{}/api/v1/agent/dev_server/register",
                local_url.trim_end_matches('/')
            );
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&RegisterDevServerRequest { auth, project, port })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("RegisterDevServer failed: HTTP {status} — {text}");
            }
            let body: RegisterDevServerResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(format!("Registered. Browse to: {}", body.url))
        }
        _ => Err(not_in_family()),
    }
}


/// The `connection` an Open tool was given: an SSH host or WSL name, trimmed;
/// `None` for absent, empty or "local" (this machine).
fn connection_arg(arguments: &Value) -> Option<String> {
    arguments
        .get("connection")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "local")
        .map(str::to_string)
}

#[cfg(test)]
mod connection_arg_tests {
    use super::*;

    #[test]
    fn only_a_named_connection_is_one() {
        let c = |v: Value| connection_arg(&serde_json::json!({ "connection": v }));
        assert_eq!(c(serde_json::json!(" user@box ")).as_deref(), Some("user@box"));
        assert_eq!(c(serde_json::json!("wsl://Ubuntu")).as_deref(), Some("wsl://Ubuntu"));
        assert_eq!(c(serde_json::json!("local")), None);
        assert_eq!(c(serde_json::json!("")), None);
        assert_eq!(connection_arg(&serde_json::json!({})), None);
    }
}
