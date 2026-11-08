// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sign-in of a CLI that has no status command, read from the credential file its login writes.
//!
//! Grok Build and Muse Code sign in with `<cli> login` and keep the result in a file under their
//! config directory (`GROK_HOME`, `XDG_CONFIG_HOME`). Neither has a command that says whether that
//! login is good, so the sign-in check (`run_auth_check`) and the expiry probe ask the file.
//!
//! - `grok`: `<dir>/auth.json`, a map from a scope to `{ key, create_time, expires_at?, refresh_token? }`
//!   (xai-org/grok-build, `xai-grok-login`). A credential without a server expiry lasts 30 days from
//!   `create_time`; the CLI refreshes its own token while a refresh token is held.
//! - `muse`: `<dir>/muse/auth.json`. Meta does not publish the layout, so any non-empty credential
//!   in it counts as signed in and its expiry is left to the CLI.

use std::path::Path;

use super::oauth_probe::OAuthProbeStatus;

/// A credential without a server-given expiry lasts this long (grok's own `TOKEN_TTL`).
const GROK_DEFAULT_LIFETIME_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// The providers whose sign-in is read from a file, by the CLI's file stem.
pub fn provider_of_cli(cli_path: &str) -> Option<&'static str> {
    let stem = Path::new(cli_path).file_stem()?.to_str()?.to_ascii_lowercase();
    match stem.as_str() {
        "grok" => Some("grok"),
        "muse" => Some("muse"),
        _ => None,
    }
}

/// The credential file of `provider` under its config dir `dir`.
fn credential_path(provider: &str, dir: &Path) -> Option<std::path::PathBuf> {
    match provider {
        "grok" => Some(dir.join("auth.json")),
        "muse" => Some(dir.join("muse").join("auth.json")),
        _ => None,
    }
}

fn time_ms(value: Option<&serde_json::Value>) -> Option<i64> {
    let s = value?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis())
}

fn non_empty(value: Option<&serde_json::Value>) -> bool {
    value.and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())
}

fn probe_grok(json: &serde_json::Value, now_ms: i64) -> OAuthProbeStatus {
    let Some(scopes) = json.as_object() else {
        return OAuthProbeStatus::NeedsReauth;
    };
    let mut any_key = false;
    let mut any_refresh = false;
    for entry in scopes.values().filter(|e| non_empty(e.get("key"))) {
        any_key = true;
        let expires = time_ms(entry.get("expires_at"))
            .or_else(|| time_ms(entry.get("create_time")).map(|t| t + GROK_DEFAULT_LIFETIME_MS));
        if expires.is_none_or(|e| e > now_ms) {
            return OAuthProbeStatus::Valid;
        }
        any_refresh |= non_empty(entry.get("refresh_token"));
    }
    match (any_key, any_refresh) {
        (true, true) => OAuthProbeStatus::Expired,
        _ => OAuthProbeStatus::NeedsReauth,
    }
}

/// Whether `json` holds a non-empty string anywhere: a token of some name.
fn holds_a_credential(json: &serde_json::Value) -> bool {
    match json {
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(a) => a.iter().any(holds_a_credential),
        serde_json::Value::Object(o) => o.values().any(holds_a_credential),
        _ => false,
    }
}

/// The sign-in state of `provider` in config dir `dir`, or `None` for a provider read another way.
pub(super) fn probe(provider: &str, dir: &Path, now_ms: i64) -> Option<OAuthProbeStatus> {
    let path = credential_path(provider, dir)?;
    let json: Option<serde_json::Value> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok());
    Some(match (provider, json) {
        (_, None) => OAuthProbeStatus::NeedsReauth,
        ("grok", Some(j)) => probe_grok(&j, now_ms),
        (_, Some(j)) if holds_a_credential(&j) => OAuthProbeStatus::Valid,
        _ => OAuthProbeStatus::NeedsReauth,
    })
}

/// Whether the CLI at `cli_path` is signed in under the config dir `auth_env` gives it (else the
/// provider's shared default dir). `None` when its sign-in is not read from a file.
pub fn signed_in(cli_path: &str, auth_env: &std::collections::HashMap<String, String>) -> Option<bool> {
    let provider = provider_of_cli(cli_path)?;
    let config = crate::backend::providers::get_provider(provider)?;
    let dir = match auth_env.get(config.auth_config_dir_env_var).filter(|d| !d.is_empty()) {
        Some(dir) => dir.clone(),
        None => crate::backend::providers::default_auth_dir(config).ok()?,
    };
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some(probe(provider, Path::new(&dir), now_ms) == Some(OAuthProbeStatus::Valid))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn grok_dir(body: serde_json::Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("auth.json"), body.to_string()).unwrap();
        dir
    }

    fn rfc3339(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    #[test]
    fn grok_is_signed_in_by_a_key_inside_its_lifetime() {
        let fresh = grok_dir(serde_json::json!({ "scope": { "key": "k", "create_time": rfc3339(NOW - 1000) } }));
        assert_eq!(probe("grok", fresh.path(), NOW), Some(OAuthProbeStatus::Valid));
        let old = grok_dir(serde_json::json!({ "scope": { "key": "k", "create_time": rfc3339(NOW - GROK_DEFAULT_LIFETIME_MS - 1000) } }));
        assert_eq!(probe("grok", old.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn a_server_expiry_wins_over_the_default_lifetime_and_a_refresh_token_softens_it() {
        let long = grok_dir(serde_json::json!({ "s": { "key": "k", "create_time": rfc3339(NOW - 90 * 86_400_000), "expires_at": rfc3339(NOW + 1000) } }));
        assert_eq!(probe("grok", long.path(), NOW), Some(OAuthProbeStatus::Valid));
        let lapsed = grok_dir(serde_json::json!({ "s": { "key": "k", "create_time": rfc3339(NOW - 1000), "expires_at": rfc3339(NOW - 5) } }));
        assert_eq!(probe("grok", lapsed.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
        let refreshable = grok_dir(serde_json::json!({ "s": { "key": "k", "create_time": rfc3339(NOW - 1000), "expires_at": rfc3339(NOW - 5), "refresh_token": "r" } }));
        assert_eq!(probe("grok", refreshable.path(), NOW), Some(OAuthProbeStatus::Expired));
    }

    #[test]
    fn grok_with_no_key_or_no_file_is_signed_out() {
        let empty = grok_dir(serde_json::json!({}));
        assert_eq!(probe("grok", empty.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
        let keyless = grok_dir(serde_json::json!({ "s": { "key": "", "refresh_token": "r" } }));
        assert_eq!(probe("grok", keyless.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
        let none = tempfile::tempdir().unwrap();
        assert_eq!(probe("grok", none.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
        let corrupt = tempfile::tempdir().unwrap();
        std::fs::write(corrupt.path().join("auth.json"), "{not json").unwrap();
        assert_eq!(probe("grok", corrupt.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn muse_is_signed_in_by_any_credential_under_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(probe("muse", dir.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
        std::fs::create_dir_all(dir.path().join("muse")).unwrap();
        std::fs::write(dir.path().join("muse").join("auth.json"), "{}").unwrap();
        assert_eq!(probe("muse", dir.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
        std::fs::write(dir.path().join("muse").join("auth.json"), r#"{"tokens":{"access":"a"}}"#).unwrap();
        assert_eq!(probe("muse", dir.path(), NOW), Some(OAuthProbeStatus::Valid));
        // A grok-shaped file in the config root does not sign muse in.
        let other = grok_dir(serde_json::json!({ "s": { "key": "k" } }));
        assert_eq!(probe("muse", other.path(), NOW), Some(OAuthProbeStatus::NeedsReauth));
    }

    #[test]
    fn only_these_clis_are_read_from_a_file() {
        assert_eq!(probe("claude", Path::new("."), NOW), None);
        assert_eq!(provider_of_cli("C:/tools/grok.exe"), Some("grok"));
        assert_eq!(provider_of_cli("/home/u/.grok/bin/grok"), Some("grok"));
        assert_eq!(provider_of_cli("/usr/bin/muse"), Some("muse"));
        assert_eq!(provider_of_cli("pi-acp"), None);
    }

    #[test]
    fn the_check_follows_the_config_dir_the_agent_is_given() {
        let signed = grok_dir(serde_json::json!({ "s": { "key": "k", "expires_at": rfc3339(32_503_680_000_000) } }));
        let env = |d: &Path| std::collections::HashMap::from([("GROK_HOME".to_string(), d.to_string_lossy().into_owned())]);
        assert_eq!(signed_in("grok", &env(signed.path())), Some(true));
        let bare = tempfile::tempdir().unwrap();
        assert_eq!(signed_in("grok", &env(bare.path())), Some(false));
        assert_eq!(signed_in("claude", &env(bare.path())), None);
    }
}
