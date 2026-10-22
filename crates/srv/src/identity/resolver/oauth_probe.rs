// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! OAuth token-file probing: the [`oauth_status`] canonical-value
//! constants, [`OAuthProbeStatus`], and [`probe_oauth_status`] itself.
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change) — self-contained, on-disk OAuth token-file probing
//! with no dependency on the rest of the resolver module.

use std::path::Path;

/// Canonical-value enumeration for OAuth-class `IdentityAccount.status`.
///
/// `IdentityAccount.status` is a `String` (free-form) at the SQLite layer
/// — api-key rows keep using whatever the legacy paths wrote
/// (`"unknown"`, `"ok"`, etc.). For oauth-class bindings we pin a small
/// closed set per spec §4.4 so the frontend status-badge dispatch is
/// deterministic and the resolver's expiry probe can never write an
/// off-the-spec string. Every place the resolver SETS or READS an
/// oauth-class status uses these constants.
pub mod oauth_status {
    /// Token file present and (probed) not expired.
    pub const VALID: &str = "valid";
    /// Access token expired; refresh likely succeeds.
    pub const EXPIRED: &str = "expired";
    /// Refresh rejected / file missing / parse error; user must Reconnect.
    pub const NEEDS_REAUTH: &str = "needs_reauth";
}

/// Result of probing a per-bundle OAuth token directory.
///
/// Computed by [`probe_oauth_status`] reading the CLI's on-disk token
/// file (e.g. `<dir>/.credentials.json` for Claude Code). Maps directly
/// to [`oauth_status`] strings. Returned as an enum so the caller can
/// branch without re-parsing the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProbeStatus {
    Valid,
    Expired,
    NeedsReauth,
}

impl OAuthProbeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => oauth_status::VALID,
            Self::Expired => oauth_status::EXPIRED,
            Self::NeedsReauth => oauth_status::NEEDS_REAUTH,
        }
    }
}

/// Codex's sign-in, from `<dir>/auth.json` (what `codex login` writes under
/// `CODEX_HOME`): a ChatGPT sign-in has `tokens.refresh_token`, an API-key
/// sign-in a non-empty `OPENAI_API_KEY`. Either is `Valid`; expiry is left to
/// the CLI, which refreshes its own token. No file, or neither credential in
/// it, is `NeedsReauth`. SPEC_CODEX_OAUTH_IN_CONNECTORS_AND_EMAIL_LABELS_2026_10_22.md §5.
fn probe_codex_auth(dir: &Path) -> OAuthProbeStatus {
    let path = dir.join("auth.json");
    let json: Option<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let non_empty = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
    let signed_in = json.as_ref().is_some_and(|j| {
        non_empty(j.get("tokens").and_then(|t| t.get("refresh_token"))) || non_empty(j.get("OPENAI_API_KEY"))
    });
    if signed_in {
        OAuthProbeStatus::Valid
    } else {
        tracing::debug!(
            target: "identity",
            path = %path.display(),
            "oauth probe: codex auth.json missing or holds no credential — status=needs_reauth"
        );
        OAuthProbeStatus::NeedsReauth
    }
}

/// Cheap on-disk probe of the per-bundle OAuth token file for a
/// provider. No network calls — just reads + parses the token JSON,
/// then compares `expiresAt` against `now_ms`.
///
/// **Provider token-file shape (spec §4.4 + §4.5):**
/// - `claude` — `<dir>/.credentials.json` with
///   `{ "claudeAiOauth": { "accessToken", "refreshToken", "expiresAt": <ms> } }`
///   (Anthropic's documented format — see
///   `docs/specs/agentmux-isolated-auth.md` §1.6).
/// - `codex` — `<dir>/auth.json`, see [`probe_codex_auth`].
/// - `openclaw` — `<dir>/.credentials.json`. Exact field layout
///   undocumented; falls through to the Claude parser as a best-effort,
///   and a file without a parseable expiry counts as `Valid`.
///
/// **Returns** `Some(status)` on a definitive read, `None` when probing
/// isn't supported for the provider (so the caller skips status
/// updates rather than mis-writing `needs_reauth` for a provider whose
/// file we just don't know how to parse yet).
///
/// **macOS caveat (`claude` only):** the Claude Code CLI stores OAuth
/// credentials in the encrypted macOS Keychain, never in
/// `<dir>/.credentials.json` — confirmed against Claude Code's own docs
/// and empirically on a real per-account auth dir on this machine (zero
/// `.credentials.json` present despite a working session). Unlike Linux
/// and Windows, this is true regardless of `CLAUDE_CONFIG_DIR` — see
/// `docs/retro/retro-macos-keychain-credential-isolation-gap-2026-08-17.md`.
/// So on macOS, a missing `claude` token file is not evidence the account
/// needs reauth — it's the expected, permanent shape, and reporting
/// `NeedsReauth` here would be a standing false positive on every working
/// macOS Claude account. `None` is returned instead (status left alone) —
/// see the `None` variant's doc above for why that's the honest answer
/// when this probe genuinely can't tell. `codex`/`openclaw` are not
/// covered by this carve-out: their macOS credential-storage behavior
/// hasn't been verified, so their existing file-probe semantics are
/// unchanged.
pub fn probe_oauth_status(
    provider: &str,
    dir: &str,
    now_ms: i64,
) -> Option<OAuthProbeStatus> {
    let probe_path: std::path::PathBuf = match provider {
        // Codex keeps its sign-in in `$CODEX_HOME/auth.json`, not
        // `.credentials.json`; reading the latter marked every working
        // Codex account needs_reauth at its first spawn.
        "codex" => return Some(probe_codex_auth(Path::new(dir))),
        // Claude Code + openclaw write to `<config_dir>/.credentials.json`
        // per `docs/specs/provider-auth-isolation.md` (the agentmux-managed
        // dir is what CLAUDE_CONFIG_DIR / OPENCLAW_HOME point at).
        // openclaw's token field-layout is not publicly documented; the
        // parser below treats unrecognised shapes as `Valid` so we don't
        // false-positive a Reconnect on a working session.
        "claude" | "openclaw" => Path::new(dir).join(".credentials.json"),
        _ => return None,
    };

    let contents = match std::fs::read_to_string(&probe_path) {
        Ok(s) => s,
        Err(e) => {
            if provider == "claude" && cfg!(target_os = "macos") {
                tracing::debug!(
                    target: "identity",
                    provider,
                    path = %probe_path.display(),
                    error = %e,
                    "oauth probe: token file unreadable on macOS — Claude Code stores \
                     credentials in Keychain here, not this file, so this is not evidence \
                     of needs_reauth; leaving status unchanged"
                );
                return None;
            }
            tracing::debug!(
                target: "identity",
                provider,
                path = %probe_path.display(),
                error = %e,
                "oauth probe: token file unreadable — status=needs_reauth"
            );
            return Some(OAuthProbeStatus::NeedsReauth);
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(
                target: "identity",
                provider,
                path = %probe_path.display(),
                error = %e,
                "oauth probe: token file parse failed — status=needs_reauth"
            );
            return Some(OAuthProbeStatus::NeedsReauth);
        }
    };

    // Claude shape — `claudeAiOauth.expiresAt` is ms since epoch.
    // Many shape-compatible providers nest under the same key; try
    // that first, then fall back to any top-level `expiresAt` /
    // `expires_at` an alternative provider might use.
    let expires_at_ms = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("expiresAt"))
        .and_then(|v| v.as_i64())
        .or_else(|| json.get("expiresAt").and_then(|v| v.as_i64()))
        .or_else(|| json.get("expires_at").and_then(|v| v.as_i64()));

    let has_refresh = json
        .get("claudeAiOauth")
        .and_then(|o| o.get("refreshToken"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        || json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false);

    match expires_at_ms {
        Some(exp) if exp <= now_ms => {
            // Past expiry. If a refresh token is present, the next
            // CLI call will likely refresh it cleanly → `expired`
            // (transient, not user-actionable). Without a refresh
            // token the user must re-OAuth → `needs_reauth`.
            if has_refresh {
                Some(OAuthProbeStatus::Expired)
            } else {
                Some(OAuthProbeStatus::NeedsReauth)
            }
        }
        Some(_) => Some(OAuthProbeStatus::Valid),
        None => {
            // Shape doesn't expose an expiry we can parse. Treat the
            // file's existence as `Valid` rather than guess — false
            // `needs_reauth` would force the user to reconnect a
            // working session. codex / openclaw fall here today.
            tracing::debug!(
                target: "identity",
                provider,
                path = %probe_path.display(),
                "oauth probe: file present but no parseable expiry — status=valid (best-effort)"
            );
            Some(OAuthProbeStatus::Valid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PR D — OAuth expiry probe + status semantics ───────────────────

    /// Helper: write a Claude-shape `.credentials.json` into a temp dir
    /// and return the dir path. `expires_ms` controls validity; `with_refresh`
    /// toggles the refreshToken field so the resolver can distinguish
    /// `Expired` (refresh present) from `NeedsReauth` (no refresh).
    fn write_claude_creds(
        dir: &std::path::Path,
        expires_ms: i64,
        with_refresh: bool,
    ) {
        std::fs::create_dir_all(dir).unwrap();
        let body = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "test-access",
                "refreshToken": if with_refresh { "test-refresh" } else { "" },
                "expiresAt": expires_ms,
            }
        });
        std::fs::write(
            dir.join(".credentials.json"),
            serde_json::to_string(&body).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn probe_oauth_status_unknown_provider_returns_none() {
        // Probing a provider that isn't in the oauth-class set is a
        // signal to the caller to leave `status` alone — None ≠
        // NeedsReauth. Guards against silent mis-classification of
        // api-key providers if a future caller accidentally feeds
        // them through here.
        let r = probe_oauth_status("github", "/tmp/whatever", 0);
        assert_eq!(r, None);
    }

    #[test]
    fn probe_oauth_status_missing_dir_is_needs_reauth_for_codex() {
        // codex isn't covered by the macOS carve-out (its macOS
        // credential-storage behavior hasn't been verified the way
        // Claude Code's has) — a missing file is still a definitive
        // needs_reauth signal for it, on every platform.
        let r = probe_oauth_status("codex", "/definitely/does/not/exist-xyz-9q", 0);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    /// The failure this guards: Codex's sign-in is `auth.json`, and probing
    /// `.credentials.json` marked every working Codex account needs_reauth.
    #[test]
    fn a_codex_sign_in_is_read_from_auth_json() {
        let probe = |contents: Option<&str>| {
            let tmp = tempfile::tempdir().unwrap();
            if let Some(c) = contents {
                std::fs::write(tmp.path().join("auth.json"), c).unwrap();
            }
            probe_oauth_status("codex", tmp.path().to_str().unwrap(), 0)
        };
        let chatgpt = r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"x","access_token":"y","refresh_token":"z","account_id":"a"},"last_refresh":"2026-10-22T00:00:00Z"}"#;
        assert_eq!(probe(Some(chatgpt)), Some(OAuthProbeStatus::Valid), "a ChatGPT sign-in");
        assert_eq!(probe(Some(r#"{"OPENAI_API_KEY":"sk-test"}"#)), Some(OAuthProbeStatus::Valid), "an API-key sign-in");
        assert_eq!(probe(Some(r#"{"OPENAI_API_KEY":null,"tokens":{"refresh_token":""}}"#)), Some(OAuthProbeStatus::NeedsReauth));
        assert_eq!(probe(Some("not json")), Some(OAuthProbeStatus::NeedsReauth));
        assert_eq!(probe(None), Some(OAuthProbeStatus::NeedsReauth), "signed out: no auth.json");
    }

    #[test]
    fn a_claude_shaped_credentials_file_does_not_sign_codex_in() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"refreshToken":"r","expiresAt":9999999999999}}"#,
        )
        .unwrap();
        let r = probe_oauth_status("codex", tmp.path().to_str().unwrap(), 0);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn probe_oauth_status_missing_dir_is_none_for_claude_on_macos() {
        // Claude Code stores OAuth credentials in the macOS Keychain, not
        // `<dir>/.credentials.json`, regardless of CLAUDE_CONFIG_DIR — a
        // missing file here is the expected, permanent shape on macOS, not
        // evidence of needs_reauth. See
        // docs/retro/retro-macos-keychain-credential-isolation-gap-2026-08-17.md.
        // Asserting `None` (not `NeedsReauth`) is what stops this probe
        // from mislabeling every working macOS Claude account as needing
        // reauth.
        let r = probe_oauth_status("claude", "/definitely/does/not/exist-xyz-9q", 0);
        assert_eq!(r, None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn probe_oauth_status_missing_dir_is_needs_reauth_for_claude_off_macos() {
        // On Linux/Windows, CLAUDE_CONFIG_DIR genuinely relocates
        // `.credentials.json` — a missing file there is a real signal.
        let r = probe_oauth_status("claude", "/definitely/does/not/exist-xyz-9q", 0);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn probe_oauth_status_future_expiry_is_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let now_ms = 1_700_000_000_000;
        write_claude_creds(tmp.path(), now_ms + 3_600_000, true);
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), now_ms);
        assert_eq!(r, Some(OAuthProbeStatus::Valid));
    }

    #[test]
    fn probe_oauth_status_past_expiry_with_refresh_is_expired() {
        let tmp = tempfile::tempdir().unwrap();
        let now_ms = 1_700_000_000_000;
        write_claude_creds(tmp.path(), now_ms - 1, true);
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), now_ms);
        assert_eq!(r, Some(OAuthProbeStatus::Expired));
    }

    #[test]
    fn probe_oauth_status_past_expiry_no_refresh_is_needs_reauth() {
        // No refresh token in the file → the CLI can't auto-refresh
        // and the user has to OAuth again. Maps to `needs_reauth`,
        // NOT `expired` (per spec §4.4).
        let tmp = tempfile::tempdir().unwrap();
        let now_ms = 1_700_000_000_000;
        write_claude_creds(tmp.path(), now_ms - 1, false);
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), now_ms);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn probe_oauth_status_malformed_json_is_needs_reauth() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".credentials.json"), "{ not json").unwrap();
        let r = probe_oauth_status("claude", tmp.path().to_str().unwrap(), 0);
        assert_eq!(r, Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn probe_oauth_status_codex_unknown_shape_is_valid_best_effort() {
        // codex / openclaw token-file layouts aren't publicly
        // documented; our parser falls through to "Valid" when the
        // file exists but lacks any parseable expiry. Better than
        // false `needs_reauth` on a working session — strict parsing
        // is a follow-up once the shape is pinned.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".credentials.json"),
            r#"{"some":"opaque-codex-blob"}"#,
        )
        .unwrap();
        let r = probe_oauth_status("codex", tmp.path().to_str().unwrap(), 0);
        assert_eq!(r, Some(OAuthProbeStatus::Valid));
    }
}
