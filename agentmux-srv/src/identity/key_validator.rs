// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Live API-key validation for the Armory.
//!
//! Each supported service has a probe that makes a single minimal
//! authenticated request and maps the response to non-secret metadata
//! (account name, scopes, etc.). This is the only outbound call the key
//! flow makes, and it fires only on the user's explicit Validate click
//! (see docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5.1, §6).
//!
//! The plaintext key is passed in by value, used to build one request, and
//! never logged. Callers must keep it out of logs/transcripts.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::json;

/// Outcome of a validation probe. `metadata` is non-secret JSON safe to
/// persist on the account row + show in the UI; `masked_tail` is the
/// last few chars for the locked display.
#[derive(Debug, Clone)]
pub struct ValidationOutcome {
    pub valid: bool,
    pub metadata: serde_json::Value,
    pub masked_tail: String,
    pub error: Option<String>,
}

impl ValidationOutcome {
    fn invalid(masked_tail: String, error: impl Into<String>) -> Self {
        Self {
            valid: false,
            metadata: json!({}),
            masked_tail,
            error: Some(error.into()),
        }
    }
}

/// Masked hint for the locked display: bullet run + last 4 chars. Stored
/// alongside the account so the panel can render `••••••••3f9a` without the
/// secret. Keys shorter than 4 chars are fully masked (no tail leak).
pub fn masked_tail(secret: &str) -> String {
    let n = secret.chars().count();
    if n <= 4 {
        return "•".repeat(n.max(4));
    }
    let tail: String = secret.chars().skip(n - 4).collect();
    format!("••••••••{tail}")
}

/// Shared HTTP client for outbound provider calls (key validation, model
/// catalog fetch). Reused by `backend::model_catalog` so we don't build a
/// parallel `reqwest::Client` singleton (reagent #1923 P2).
pub(crate) fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("AgentMux")
            .build()
            .expect("reqwest client build failed")
    })
}

/// Validate `key` for `provider`. Makes one outbound HTTPS request. Returns
/// a non-`valid` outcome (never errors the RPC) so the caller can surface a
/// structured message and stay in the entry state.
pub async fn validate(provider: &str, key: &str) -> ValidationOutcome {
    let tail = masked_tail(key);
    match provider {
        "github" => github(key, tail).await,
        "openai" => openai(key, tail).await,
        "anthropic" => anthropic(key, tail).await,
        "slack" => slack(key, tail).await,
        other => ValidationOutcome::invalid(
            tail,
            format!("no validator for provider '{other}' — save without validating"),
        ),
    }
}

async fn github(key: &str, tail: String) -> ValidationOutcome {
    let resp = match client()
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ValidationOutcome::invalid(tail, format!("network error: {e}")),
    };
    if !resp.status().is_success() {
        return ValidationOutcome::invalid(tail, format!("GitHub rejected the token ({})", resp.status()));
    }
    // Scopes come back in a response header, not the body.
    let scopes = resp
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let login = body.get("login").and_then(|v| v.as_str()).unwrap_or("");
    ValidationOutcome {
        valid: true,
        metadata: json!({ "github_username": login, "github_scopes": scopes }),
        masked_tail: tail,
        error: None,
    }
}

async fn openai(key: &str, tail: String) -> ValidationOutcome {
    let resp = match client()
        .get("https://api.openai.com/v1/models")
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ValidationOutcome::invalid(tail, format!("network error: {e}")),
    };
    if !resp.status().is_success() {
        return ValidationOutcome::invalid(tail, format!("OpenAI rejected the key ({})", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let model_count = body.get("data").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    ValidationOutcome {
        valid: true,
        metadata: json!({ "openai_model_count": model_count }),
        masked_tail: tail,
        error: None,
    }
}

async fn anthropic(key: &str, tail: String) -> ValidationOutcome {
    let resp = match client()
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ValidationOutcome::invalid(tail, format!("network error: {e}")),
    };
    if !resp.status().is_success() {
        return ValidationOutcome::invalid(tail, format!("Anthropic rejected the key ({})", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| json!({}));
    let model_count = body.get("data").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    ValidationOutcome {
        valid: true,
        metadata: json!({ "anthropic_model_count": model_count }),
        masked_tail: tail,
        error: None,
    }
}

async fn slack(key: &str, tail: String) -> ValidationOutcome {
    let resp = match client()
        .post("https://slack.com/api/auth.test")
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ValidationOutcome::invalid(tail, format!("network error: {e}")),
    };
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return ValidationOutcome::invalid(tail, format!("bad response: {e}")),
    };
    // Slack returns 200 with { ok: false, error } for bad tokens.
    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = body.get("error").and_then(|v| v.as_str()).unwrap_or("invalid token");
        return ValidationOutcome::invalid(tail, format!("Slack rejected the token: {err}"));
    }
    let team = body.get("team").and_then(|v| v.as_str()).unwrap_or("");
    let user = body.get("user").and_then(|v| v.as_str()).unwrap_or("");
    ValidationOutcome {
        valid: true,
        metadata: json!({ "slack_team": team, "slack_user": user }),
        masked_tail: tail,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_all_but_last_four() {
        assert_eq!(masked_tail("ghp_abcdef123456"), "••••••••3456");
    }

    #[test]
    fn short_keys_fully_masked_no_tail_leak() {
        assert_eq!(masked_tail("ab"), "••••");
        assert_eq!(masked_tail("abcd"), "••••");
    }
}
