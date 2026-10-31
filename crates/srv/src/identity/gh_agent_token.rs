// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A GitHub token from the host's `gh-agent`, for a container agent.
//!
//! A container agent has no GitHub setup of its own, and AgentMux's own token source (the App
//! broker, `identity::github_tokens`) is not live everywhere yet. Where `gh-agent` is installed on
//! the host, it already holds the App identity the fleet works under, so `gh-agent token --repo
//! owner/repo` gives a short-lived installation token narrowed to the repository. srv runs it for
//! the agent that asked (an agent id and a repository, never a command) and hands the token over
//! the container route; nothing is written into the container.

use std::path::PathBuf;
use std::time::Duration;

use agentmux_common::win32::NoWindow;
use serde::Deserialize;
use zeroize::Zeroizing;

/// How long `gh-agent token` may take: a cached token is instant, a fresh one is a round trip.
const TIMEOUT: Duration = Duration::from_secs(25);

/// A token `gh-agent` minted.
pub struct HostToken {
    pub token: Zeroizing<String>,
    /// Epoch seconds.
    pub expires_at: i64,
    /// The bot the token acts as, without the `[bot]` suffix (`muxagent`).
    pub bot_slug: Option<String>,
}

#[derive(Deserialize)]
struct Output {
    token: String,
    expires_at: Option<i64>,
    acting_as: Option<String>,
}

/// `gh-agent` on the host's PATH, if it is installed.
pub fn installed() -> Option<PathBuf> {
    which::which("gh-agent").ok()
}

/// What an agent is told when `gh-agent` is installed but did not answer in [`TIMEOUT`]: not a missing connection.
pub fn took_too_long() -> String {
    format!("GitHub token took too long: this machine's gh-agent didn't answer in {} s. Try again in a minute", TIMEOUT.as_secs())
}

/// Whether `error` (a [`token_for`] error) is [`took_too_long`].
pub fn is_took_too_long(error: &str) -> bool {
    error.starts_with("GitHub token took too long")
}

/// A GitHub account or repository name: what the request may name, and nothing that could be read as
/// an option or a path.
fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.starts_with(['-', '.']) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The token in `gh-agent token --json`'s output.
fn parse(stdout: &[u8]) -> Result<HostToken, String> {
    let out: Output = serde_json::from_slice(stdout).map_err(|_| "gh-agent gave no token".to_string())?;
    if out.token.is_empty() {
        return Err("gh-agent gave no token".to_string());
    }
    let bot_slug = out.acting_as.map(|a| a.trim_end_matches("[bot]").to_string()).filter(|a| !a.is_empty());
    Ok(HostToken { token: Zeroizing::new(out.token), expires_at: out.expires_at.unwrap_or(0), bot_slug })
}

/// The numeric id in GitHub's answer for a user (`GET /users/<login>`).
fn parse_user_id(body: &[u8]) -> Option<u64> {
    #[derive(Deserialize)]
    struct User {
        id: u64,
    }
    serde_json::from_slice::<User>(body).ok().map(|u| u.id)
}

/// How long a failed lookup is not repeated: offline, or GitHub's unauthenticated rate limit, would
/// otherwise cost every token request the whole timeout.
const LOOKUP_RETRY: Duration = Duration::from_secs(60);

/// What is known of a bot's id: found for good, or failed at `Instant`.
#[derive(Clone, Copy)]
enum Lookup {
    Found(u64),
    Failed(std::time::Instant),
}

/// The answer to give without asking again, if there is one.
fn remembered(entry: Option<Lookup>, now: std::time::Instant) -> Option<Option<u64>> {
    match entry? {
        Lookup::Found(id) => Some(Some(id)),
        Lookup::Failed(at) if now.saturating_duration_since(at) < LOOKUP_RETRY => Some(None),
        Lookup::Failed(_) => None,
    }
}

async fn fetch_bot_user_id(slug: &str) -> Option<u64> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).user_agent("agentmux-srv").build().ok()?;
    let resp = client
        .get(format!("https://api.github.com/users/{slug}%5Bbot%5D"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    parse_user_id(&resp.bytes().await.ok()?)
}

/// The user id of the bot `<slug>[bot]`, which `gh-agent` doesn't say: a commit's author email is
/// `<id>+<slug>[bot]@users.noreply.github.com`. A public lookup, asked once per bot and kept; `None`
/// when GitHub can't be reached (not asked again for a minute), and then the commit keeps its own author.
pub async fn bot_user_id(slug: &str) -> Option<u64> {
    static KNOWN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Lookup>>> = std::sync::OnceLock::new();
    let known = KNOWN.get_or_init(Default::default);
    let entry = known.lock().unwrap_or_else(|e| e.into_inner()).get(slug).copied();
    if let Some(answer) = remembered(entry, std::time::Instant::now()) {
        return answer;
    }
    if !is_name(slug) {
        return None;
    }
    let id = fetch_bot_user_id(slug).await;
    let outcome = id.map_or_else(|| Lookup::Failed(std::time::Instant::now()), Lookup::Found);
    known.lock().unwrap_or_else(|e| e.into_inner()).insert(slug.to_string(), outcome);
    id
}

/// A token for `owner/repo`, minted for the agent `slug`.
pub async fn token_for(slug: &str, owner: &str, repo: &str) -> Result<HostToken, String> {
    if !is_name(owner) || !is_name(repo) {
        return Err("not a GitHub repository".to_string());
    }
    let exe = installed().ok_or_else(|| "gh-agent isn't installed on this machine".to_string())?;
    let mut cmd = tokio::process::Command::new(&exe);
    // No instance identity or key reaches it; the agent id below is the one thing it is told.
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.args(["token", "--repo", &format!("{owner}/{repo}"), "--json"])
        .env("AGENTMUX_AGENT_ID", slug)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    cmd.no_window();
    let out = match tokio::time::timeout(TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("couldn't run gh-agent: {e}")),
        Err(_) => return Err(took_too_long()),
    };
    if !out.status.success() {
        return Err("gh-agent gave no token for that repository".to_string());
    }
    parse(&out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_account_and_repository_names_are_passed_on() {
        for ok in ["agentmuxai", "agentmux-landing", "my_repo.js", "a"] {
            assert!(is_name(ok), "{ok}");
        }
        for bad in ["", "-x", ".hidden", "a b", "a/b", "a;b", "--repo", "a\nb", &"x".repeat(101)] {
            assert!(!is_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_bots_user_id_is_read_from_githubs_answer() {
        assert_eq!(parse_user_id(br#"{"login":"muxagent[bot]","id":331051269,"type":"Bot"}"#), Some(331051269));
        assert_eq!(parse_user_id(br#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_user_id(b"x"), None);
    }

    #[test]
    fn a_failed_lookup_is_not_repeated_for_a_minute() {
        let now = std::time::Instant::now();
        assert_eq!(remembered(None, now), None);
        assert_eq!(remembered(Some(Lookup::Found(7)), now), Some(Some(7)));
        assert_eq!(remembered(Some(Lookup::Failed(now)), now), Some(None));
        if let Some(later) = now.checked_add(LOOKUP_RETRY + Duration::from_secs(1)) {
            assert_eq!(remembered(Some(Lookup::Failed(now)), later), None);
            assert_eq!(remembered(Some(Lookup::Found(7)), later), Some(Some(7)));
        }
    }

    #[test]
    fn the_token_and_its_expiry_are_read_from_the_json() {
        let got = parse(br#"{"token":"ghs_x","expires_at":1793224607,"acting_as":"muxagent[bot]","tier":"shared","owner":"o"}"#).unwrap();
        assert_eq!((got.token.as_str(), got.expires_at, got.bot_slug.as_deref()), ("ghs_x", 1793224607, Some("muxagent")));
        assert!(parse(b"not json").is_err());
        assert!(parse(br#"{"token":""}"#).is_err());
        assert_eq!(parse(br#"{"token":"t"}"#).unwrap().expires_at, 0);
    }
}
