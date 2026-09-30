// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser pane control.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "BrowserNavigate" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let url = arguments
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: url"))?;
            let req_url = format!("{}/api/v1/ui/browser/navigate", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserNavigateRequest { auth, url: url.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("navigate failed: HTTP {status} — {text}");
            }
            Ok(format!("Navigated to {url:?}"))
        }
        "BrowserBack" | "BrowserForward" | "BrowserReload" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let ignore_cache = arguments.get("ignore_cache").and_then(|v| v.as_bool());
            let route = match name {
                "BrowserBack" => "back",
                "BrowserForward" => "forward",
                _ => "reload",
            };
            let req_url = format!("{}/api/v1/ui/browser/{route}", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserHistoryRequest { auth, ignore_cache })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("{route} failed: HTTP {status} — {text}");
            }
            Ok(format!("Browser {route} ok"))
        }
        "BrowserEval" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let script = arguments
                .get("script")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: script"))?;
            let await_promise = arguments.get("await_promise").and_then(|v| v.as_bool());
            let req_url = format!("{}/api/v1/ui/browser/eval", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserEvalRequest {
                    auth,
                    script: script.to_string(),
                    await_promise,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("eval failed: HTTP {status} — {text}");
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let data = body.get("data").cloned().unwrap_or(json!({}));
            Ok(serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string()))
        }
        "BrowserDispatchKey" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments.get("selector").and_then(|v| v.as_str()).map(str::to_string);
            let text = arguments.get("text").and_then(|v| v.as_str()).map(str::to_string);
            let key = arguments.get("key").and_then(|v| v.as_str()).map(str::to_string);
            if text.is_some() == key.is_some() {
                anyhow::bail!("BrowserDispatchKey requires exactly one of `text` or `key`");
            }
            let req_url = format!("{}/api/v1/ui/browser/dispatch_key", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserDispatchKeyRequest { auth, selector, text, key })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("dispatch_key failed: HTTP {status} — {text}");
            }
            Ok("Dispatched".to_string())
        }
        "BrowserFocusElement" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: selector"))?;
            let req_url = format!("{}/api/v1/ui/browser/focus_element", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserFocusElementRequest { auth, selector: selector.to_string() })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("focus_element failed: HTTP {status} — {text}");
            }
            Ok(format!("Focused {selector:?}"))
        }
        "BrowserFocusInfo" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let req_url = format!("{}/api/v1/ui/browser/focus_info", local_url.trim_end_matches('/'));
            let resp = client
                .post(&req_url)
                .header("X-AuthKey", auth_key)
                .json(&UiBrowserFocusInfoRequest { auth })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("focus_info failed: HTTP {status} — {text}");
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let focused = body
                .get("data")
                .and_then(|d| d.get("focused"))
                .cloned()
                .unwrap_or(Value::Null);
            Ok(serde_json::to_string_pretty(&focused).unwrap_or_else(|_| focused.to_string()))
        }
        _ => Err(not_in_family()),
    }
}
