// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Credential-state diagnostics for login/logout robustness runs.
//!
//! The auth flow has a long history of bugs that only show up across
//! *repeated* login/logout rounds — a login writing tokens to one dir while
//! the check/spawn reads another, a logout that reports "removed" but leaves
//! the file behind, a stale re-seed sentinel that blocks a fresh import, or a
//! present-but-stale token surviving a logout. None of those are visible from
//! a single "authenticated: true/false" line.
//!
//! [`snapshot`] captures the observable on-disk truth for a Claude config dir
//! as one compact, greppable string. It includes a **fingerprint** of the
//! access token — a non-reversible 32-bit hash, never the token itself — so
//! you can tell "same token as last round" from "new token" while diffing a
//! `muxlog auth` trace, without any secret touching the log.
//!
//! Emit it at the login-check boundary and around logout/removal; the
//! `auth.credstate:` / `identity.delete:` message prefixes are already
//! `muxlog auth` vocabulary.

use std::path::Path;

/// Non-reversible fingerprint of a secret, for round-to-round diffing only.
///
/// FNV-1a over the bytes, low 32 bits as 8 hex chars. Deterministic across
/// process restarts (unlike `DefaultHasher`), so fingerprints stay comparable
/// even if the srv bounces mid-run. 32 bits of a hash of a ~100-char
/// high-entropy token is not a meaningful disclosure — but keep it a
/// fingerprint, never widen it toward the raw value.
fn fingerprint(secret: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for b in secret.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    format!("{:08x}", (h & 0xffff_ffff) as u32)
}

/// A one-line, secret-free snapshot of a Claude credential dir. Fields:
///   `present`  — `.credentials.json` exists
///   `token`    — access-token fingerprint (`none` if absent/empty)
///   `refresh`  — a non-empty refresh token is present
///   `seeded`   — the one-time global-import sentinel exists
///   `mtime`    — credentials.json mtime as unix secs (`0` if unknown)
///
/// `config_dir` is the value of `CLAUDE_CONFIG_DIR` (the isolated/shared
/// provider dir); pass the resolved dir the caller actually reads/writes so
/// the snapshot reflects the SAME location, not a guessed one.
pub fn snapshot(config_dir: &str) -> String {
    let dir = Path::new(config_dir);
    let creds = dir.join(".credentials.json");
    let seeded = dir.join(".agentmux-cred-seeded").exists();

    let (present, token_fp, refresh) = match std::fs::read_to_string(&creds) {
        Ok(content) => {
            let (mut fp, mut has_refresh) = ("none".to_string(), false);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let oauth = json.get("claudeAiOauth");
                if let Some(tok) = oauth
                    .and_then(|o| o.get("accessToken"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    fp = fingerprint(tok);
                }
                has_refresh = oauth
                    .and_then(|o| o.get("refreshToken"))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
            }
            (true, fp, has_refresh)
        }
        Err(_) => (false, "none".to_string(), false),
    };

    let mtime = std::fs::metadata(&creds)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    format!(
        "dir={config_dir} present={present} token={token_fp} refresh={refresh} seeded={seeded} mtime={mtime}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_distinct() {
        // Deterministic (no process-random seed) and secret-free.
        assert_eq!(fingerprint("sk-ant-oat-AAAA"), fingerprint("sk-ant-oat-AAAA"));
        assert_ne!(fingerprint("sk-ant-oat-AAAA"), fingerprint("sk-ant-oat-BBBB"));
        let fp = fingerprint("sk-ant-oat-secretsecret");
        assert_eq!(fp.len(), 8);
        assert!(!fp.contains("secret"));
    }

    #[test]
    fn snapshot_absent_dir() {
        let snap = snapshot("/nonexistent/agentmux-test-dir-xyz");
        assert!(snap.contains("present=false"));
        assert!(snap.contains("token=none"));
        assert!(snap.contains("refresh=false"));
        assert!(snap.contains("seeded=false"));
    }

    #[test]
    fn snapshot_reports_token_fingerprint_and_refresh() {
        let dir = std::env::temp_dir().join(format!("amx-auth-diag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let creds = dir.join(".credentials.json");
        std::fs::write(
            &creds,
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat-TESTTOKEN","refreshToken":"rt-xyz"}}"#,
        )
        .unwrap();

        let snap = snapshot(dir.to_str().unwrap());
        assert!(snap.contains("present=true"), "{snap}");
        assert!(snap.contains("refresh=true"), "{snap}");
        assert!(!snap.contains("TESTTOKEN"), "token must never appear: {snap}");
        assert!(
            snap.contains(&format!("token={}", fingerprint("sk-ant-oat-TESTTOKEN"))),
            "{snap}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
