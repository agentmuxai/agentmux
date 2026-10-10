// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity M4d-3: this agent's signing keys, fetched from srv
//! (`GET /agentmux/agents/self/keys`) rather than only read from `.mcp.json`
//! (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.10).
//!
//! The env copy goes stale: srv rotates the host jekt key after 24 h, and an
//! agent running longer keeps signing with the key it was launched with, so
//! every jekt it sends fails srv's check. Fetched at startup and again once
//! the fetched key reaches the expiry srv served with it: srv rotates a key
//! only after that, so until then nothing newer exists, and the fetch that
//! follows it is what rotates it. Checked before every tool call, so a UI
//! automation proof is signed with a live key too.
//!
//! The fetched name-keyed keys are used only when the slug srv served them
//! for is exactly this process's `AGENTMUX_AGENT_ID`: an agent signing under
//! a name its row doesn't have (a collision-suffixed backfill, a #3573 stub)
//! keeps its env keys, which are the ones filed under the name it signs as.
//! The UID-keyed keys are held for M4d-6's v2 signatures. Nothing here is
//! logged.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::AUTH_KEY_HEADER;
use serde::Deserialize;

/// Re-fetch a copy this old even with no expiry to go by (an older srv).
const REFRESH_AFTER: Duration = Duration::from_secs(20 * 60 * 60);

#[derive(Deserialize, Default, Clone)]
struct Wire {
    slug: String,
    jekt_key: Option<String>,
    /// Unix seconds; absent from an srv that predates it.
    #[serde(default)]
    jekt_key_expires_at: Option<i64>,
    lan_key: Option<String>,
    wan_key: Option<String>,
    #[allow(dead_code)] // M4d-6 signs v2 with it.
    uid_lan_key: Option<String>,
}

struct Cached {
    keys: Wire,
    fetched_at: Instant,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// When a fetch was last tried. With no usable copy (an srv without the
/// endpoint, an unreachable one), every tool call would otherwise retry and
/// wait out the request; this spaces the retries.
static LAST_ATTEMPT: Mutex<Option<Instant>> = Mutex::new(None);
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Short, so a slow srv delays a tool call by this at most.
const FETCH_TIMEOUT: Duration = Duration::from_secs(3);

/// Fetch the keys if there is no copy yet, or it is older than 20 h. Errors
/// leave the old copy (or none) in place: signing then falls back to the env
/// keys, exactly as before M4d-3.
pub(crate) async fn refresh_if_stale(client: &reqwest::Client, local_url: &str, auth_key: &str) {
    let fresh = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|c| still_good(c, agentmux_common::time::now_secs()));
    if fresh || local_url.is_empty() || std::env::var("AGENTMUX_AGENT_TOKEN").map_or(true, |t| t.trim().is_empty()) {
        return;
    }
    {
        let mut last = LAST_ATTEMPT.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < RETRY_AFTER) {
            return;
        }
        *last = Some(Instant::now());
    }
    let url = format!("{}/agentmux/agents/self/keys", local_url.trim_end_matches('/'));
    let Ok(resp) = client.get(&url).header(AUTH_KEY_HEADER, auth_key).timeout(FETCH_TIMEOUT).send().await else { return };
    if !resp.status().is_success() {
        return;
    }
    if let Ok(keys) = resp.json::<Wire>().await {
        *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Cached { keys, fetched_at: Instant::now() });
    }
}

/// Whether a cached copy needs no re-fetch at `now` (unix seconds).
fn still_good(c: &Cached, now: i64) -> bool {
    c.fetched_at.elapsed() < REFRESH_AFTER && c.keys.jekt_key_expires_at.is_none_or(|expires| now < expires)
}

/// A fetched name-keyed key, decoded, when srv served it for exactly the name
/// this process signs as.
fn name_key(pick: impl Fn(&Wire) -> Option<&String>) -> Option<Vec<u8>> {
    let own = std::env::var("AGENTMUX_AGENT_ID").ok()?;
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cached = cache.as_ref()?;
    if cached.keys.slug != own {
        return None;
    }
    agentmux_common::jekt_sign::decode_key(pick(&cached.keys)?)
}

/// The host jekt HMAC key: the fetched one when it is ours, else the env's.
pub(crate) fn jekt_key() -> Option<Vec<u8>> {
    name_key(|k| k.jekt_key.as_ref()).or_else(|| env_key("AGENTMUX_JEKT_KEY"))
}

/// The LAN (and cross-channel) private key: fetched when ours, else the env's.
pub(crate) fn lan_key() -> Option<Vec<u8>> {
    name_key(|k| k.lan_key.as_ref()).or_else(|| env_key("AGENTMUX_LAN_KEY"))
}

/// The WAN private key: fetched when ours, else the env's.
pub(crate) fn wan_key() -> Option<Vec<u8>> {
    name_key(|k| k.wan_key.as_ref()).or_else(|| env_key("AGENTMUX_WAN_KEY"))
}

fn env_key(var: &str) -> Option<Vec<u8>> {
    std::env::var(var).ok().filter(|s| !s.is_empty()).and_then(|b64| agentmux_common::jekt_sign::decode_key(&b64))
}

#[cfg(test)]
pub(crate) fn set_for_test(slug: &str, jekt_key: Option<&str>) {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Cached {
        keys: Wire { slug: slug.to_string(), jekt_key: jekt_key.map(str::to_string), ..Default::default() },
        fetched_at: Instant::now(),
    });
}

#[cfg(test)]
pub(crate) fn clear_for_test() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [7; 32] and [1; 32], base64.
    const FETCHED_B64: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
    const ENV_B64: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

    /// The cache and `AGENTMUX_AGENT_ID` / `AGENTMUX_JEKT_KEY` are process-wide.
    static LOCK: Mutex<()> = Mutex::new(());

    fn with_env<T>(agent_id: &str, env_key: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("AGENTMUX_AGENT_ID", agent_id);
        match env_key {
            Some(k) => std::env::set_var("AGENTMUX_JEKT_KEY", k),
            None => std::env::remove_var("AGENTMUX_JEKT_KEY"),
        }
        let out = f();
        clear_for_test();
        std::env::remove_var("AGENTMUX_JEKT_KEY");
        out
    }

    #[test]
    fn a_fetched_key_for_exactly_this_name_wins_over_the_env_copy() {
        let fetched = [7u8; 32];
        let key = with_env("aria", Some(ENV_B64), || {
            set_for_test("aria", Some(FETCHED_B64));
            jekt_key()
        });
        assert_eq!(key.as_deref(), Some(&fetched[..]));
    }

    #[test]
    fn a_key_served_for_another_name_is_not_used() {
        // This agent signs as "aria" but its row's slug is "aria-2": the
        // served key is aria-2's, and srv checks "aria" against aria's.
        let env = [1u8; 32];
        let key = with_env("aria", Some(ENV_B64), || {
            set_for_test("aria-2", Some(FETCHED_B64));
            jekt_key()
        });
        assert_eq!(key.as_deref(), Some(&env[..]));
    }

    #[test]
    fn nothing_fetched_means_the_env_key_as_before() {
        let env = [1u8; 32];
        assert_eq!(with_env("aria", Some(ENV_B64), jekt_key).as_deref(), Some(&env[..]));
        assert_eq!(with_env("aria", None, jekt_key), None);
    }

    #[test]
    fn a_copy_is_good_until_its_key_expires_then_refetched() {
        let cached = |expires: Option<i64>| Cached {
            keys: Wire { slug: "aria".into(), jekt_key_expires_at: expires, ..Default::default() },
            fetched_at: Instant::now(),
        };
        assert!(still_good(&cached(Some(1_000)), 999));
        assert!(!still_good(&cached(Some(1_000)), 1_000), "expired: srv may rotate it now");
        assert!(still_good(&cached(None), 1_000_000), "no expiry served: the 20 h clock");
    }
}
