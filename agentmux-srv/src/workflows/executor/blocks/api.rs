// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! API block — HTTP request via reqwest. Honors `{{...}}` interpolation
//! in url, headers, and body.
//!
//! Config (`node.data`):
//!   * `method` — "GET" / "POST" / "PUT" / "DELETE" / "PATCH" (default GET)
//!   * `url` — required; supports `{{...}}`
//!   * `headers` — optional `{ name: value }` map; values support `{{...}}`
//!   * `body` — optional string body (POST/PUT/PATCH); supports `{{...}}`
//!
//! Output:
//!   ```json
//!   { "status": 200, "body": <parsed-json-or-text>, "headers": { ... } }
//!   ```

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::FlowNode;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    let method_raw = node
        .data
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_uppercase();
    let url_raw = node
        .data
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "API block missing `url`".to_string())?;
    let url = scope.resolve(url_raw);
    if url.trim().is_empty() {
        return Err("API block resolved URL is empty".to_string());
    }

    let headers_map: HashMap<String, String> = match node.data.get("headers") {
        Some(Value::Object(obj)) => obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), scope.resolve(s))))
            .collect(),
        _ => HashMap::new(),
    };

    let body_resolved: Option<String> = node
        .data
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| scope.resolve(s));

    let timeout_ms = node
        .data
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;
    let method = reqwest::Method::from_bytes(method_raw.as_bytes())
        .map_err(|e| format!("invalid method `{method_raw}`: {e}"))?;
    let mut req = client.request(method, &url);
    for (k, v) in &headers_map {
        req = req.header(k, v);
    }
    if let Some(b) = body_resolved {
        if !b.is_empty() {
            req = req.body(b);
        }
    }
    let resp = req.send().await.map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status().as_u16();
    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    // Try to parse as JSON for downstream `{{api.body.field}}` access.
    let body_val: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));

    Ok(json!({
        "status": status,
        "body": body_val,
        "headers": resp_headers,
    }))
}
