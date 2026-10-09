// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! PtyShell*: interactive PTY shells.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "PtyShell" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            if block_id.is_empty() {
                anyhow::bail!(
                    "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
                     — cannot associate the shell with a conversation pane. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let cwd = arguments.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
            let rows = arguments.get("rows").and_then(|v| v.as_u64()).map(|n| n as u16);
            let cols = arguments.get("cols").and_then(|v| v.as_u64()).map(|n| n as u16);

            let url = format!("{}/api/v1/ptyshell/create", local_url.trim_end_matches('/'));
            let connection = arguments.get("connection").and_then(|v| v.as_str()).map(str::to_string);
            let req = PtyShellCreateRequest {
                agent_block_id: block_id.to_string(),
                cwd,
                rows,
                cols,
                connection,
                auth: sign_ui_automation_auth().ok(),
            };
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .timeout(crate::srv_http::SHELL_CREATE_TIMEOUT)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/create failed: HTTP {status} — {text}");
            }

            let result: PtyShellCreateResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result.shell_id)
        }
        "PtyShellInput" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let text = arguments
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: text"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/input", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&PtyShellInputRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                    text: text.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/input failed: HTTP {status} — {body}");
            }

            let result: PtyShellInputResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.written {
                format!("wrote to shell {shell_id}")
            } else {
                format!(
                    "shell {shell_id}: write failed — {}",
                    result.error.unwrap_or_else(|| "unknown reason".to_string())
                )
            })
        }
        "PtyShellResize" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let rows = arguments
                .get("rows")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: rows"))? as u16;
            let cols = arguments
                .get("cols")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: cols"))? as u16;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/resize", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&PtyShellResizeRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                    rows,
                    cols,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/resize failed: HTTP {status} — {body}");
            }

            let result: PtyShellResizeResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.resized {
                format!("resized shell {shell_id} to {rows}x{cols}")
            } else {
                format!(
                    "shell {shell_id}: resize failed — {}",
                    result.error.unwrap_or_else(|| "unknown reason".to_string())
                )
            })
        }
        "PtyShellRead" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;
            let tail_lines = arguments.get("tail_lines").and_then(|v| v.as_u64()).map(|n| n as u32);

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/read", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&PtyShellReadRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                    tail_lines,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/read failed: HTTP {status} — {body}");
            }

            let result: PtyShellReadResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let prefix = if result.truncated { "[...output truncated...]\n" } else { "" };
            Ok(format!("{prefix}{}", result.content))
        }
        "PtyShellStatus" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/status", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&PtyShellStatusRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/status failed: HTTP {status} — {body}");
            }

            let result: PtyShellStatusResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.running {
                format!("shell {shell_id} is running")
            } else {
                let code = result.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
                format!("shell {shell_id} is not running — exit_code: {code}")
            })
        }
        "PtyShellStop" => {
            let shell_id = arguments
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: shell_id"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/ptyshell/stop", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&PtyShellStopRequest {
                    shell_id: shell_id.to_string(),
                    agent_block_id: block_id.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("ptyshell/stop failed: HTTP {status} — {body}");
            }

            let result: PtyShellStopResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.released {
                format!("released the agent lock on shell {shell_id} (the shell itself keeps running)")
            } else {
                format!("shell {shell_id}: no active agent lock to release (unrecognized id, not yours, or already unlocked)")
            })
        }
        _ => Err(not_in_family()),
    }
}
