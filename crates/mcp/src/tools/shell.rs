// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shell, ShellStop, ShellInput, ShellStatus: pipe shells in the agent pane.
//! ConnList: the connections those and PtyShell can run on.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "Shell" => {
            let cmd = arguments
                .get("cmd")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: cmd"))?;

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

            let title = arguments
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(cmd);
            let cwd = arguments.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
            let env = arguments
                .get("env")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            let url = format!(
                "{}/api/v1/shell/create",
                local_url.trim_end_matches('/')
            );
            let capture_stdin = arguments.get("capture_stdin").and_then(|v| v.as_bool());
            let connection = arguments.get("connection").and_then(|v| v.as_str()).map(str::to_string);
            let req = ShellCreateRequest {
                agent_block_id: block_id.to_string(),
                cmd: cmd.to_string(),
                title: Some(title.to_string()),
                cwd,
                env,
                capture_stdin,
                connection,
            };

            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/create failed: HTTP {status} — {text}");
            }

            let result: ShellCreateResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(result.shell_id)
        }
        "ShellStop" => {
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

            let url = format!("{}/api/v1/shell/stop", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&ShellStopRequest { shell_id: shell_id.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/stop failed: HTTP {status} — {text}");
            }

            let result: ShellStopResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.stopped {
                format!("stopped shell {shell_id}")
            } else {
                format!("shell {shell_id} was not running (unknown or already exited)")
            })
        }
        "ShellInput" => {
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

            let url = format!("{}/api/v1/shell/input", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&ShellInputRequest { shell_id: shell_id.to_string(), text: text.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/input failed: HTTP {status} — {body}");
            }

            let result: ShellInputResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.written {
                format!("wrote to shell {shell_id}")
            } else {
                match result.reason {
                    Some(ShellInputFailure::StdinNotCaptured) => format!(
                        "shell {shell_id} is running but was started without capture_stdin=true — \
                         its stdin is /dev/null. Recreate the shell with Shell(..., capture_stdin=true) \
                         to send input."
                    ),
                    Some(ShellInputFailure::WriteFailed) => format!(
                        "shell {shell_id} closed its stdin — input discarded"
                    ),
                    Some(ShellInputFailure::NotRunning) | None => format!(
                        "shell {shell_id} is not running — input discarded"
                    ),
                }
            })
        }
        "ShellStatus" => {
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

            let url = format!("{}/api/v1/shell/status", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&ShellStatusRequest { shell_id: shell_id.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("shell/status failed: HTTP {status} — {body}");
            }

            let result: ShellStatusResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(if result.running {
                format!("shell {shell_id} is running ({} lines so far)", result.line_count)
            } else {
                let code = result.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
                format!("shell {shell_id} has exited — exit_code: {code}, {} lines total", result.line_count)
            })
        }
        "ConnList" => {
            crate::srv_http::require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/conn/list", local_url.trim_end_matches('/'));
            let result = crate::srv_http::srv_get_json(client, &url, auth_key, &[], "ConnList").await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        _ => Err(not_in_family()),
    }
}
