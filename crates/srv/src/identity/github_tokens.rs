// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! GitHub tokens for agents.
//!
//! An agent that pushes or opens a pull request does it as the AgentMux GitHub App's bot, with
//! an installation token AgentMux Cloud's broker mints (`github_broker::installation_token`),
//! never with the person's own token and never with a token pasted in. This module is srv's
//! side of that:
//!
//! - which installation a repository's owner is (the installations the account has linked);
//! - a token narrowed to the one repository and the permissions an agent needs;
//! - a cache per (agent, owner, repository, workflows), so an agent that runs many `git`/`gh`
//!   commands an hour asks the broker about once, refreshed when less than five minutes remain;
//! - what the person is told when there is no token: the reason, in words, and never a fall-back
//!   to another credential.
//!
//! The broker is behind [`Backend`], so the cache and the choices are tested without a network.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, OnceLock};

use zeroize::Zeroizing;

use crate::backend::storage::store::Store;
use crate::muxbus::github_broker::{self, AppBot, BrokerError, InstallationRequest, LinkedInstallation};

/// A cached token is renewed when it has less than this long to live.
pub const REFRESH_MARGIN_SECS: i64 = 300;
/// How long the list of linked installations is trusted. A fresh install shows up within this.
const INSTALLATIONS_TTL_SECS: i64 = 60;
/// A refusal is remembered this long, so a command that asks many times doesn't ask the broker
/// each time (its mint limit is per hour).
const REFUSAL_TTL_SECS: i64 = 20;
/// If the broker's expiry can't be read, the token is trusted this long.
const FALLBACK_LIFETIME_SECS: i64 = 600;

/// What an agent's token may do: push and open pull requests and issues, read repository
/// metadata. `workflows: write` only for an agent the person allowed to change CI.
pub fn permissions_for(workflows: bool) -> BTreeMap<String, String> {
    let mut p: BTreeMap<String, String> = [("contents", "write"), ("pull_requests", "write"), ("issues", "write"), ("metadata", "read")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if workflows {
        p.insert("workflows".to_string(), "write".to_string());
    }
    p
}

/// A token wanted: for `agent`, on `owner/repo` (names as GitHub spells them; compared without case).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Want {
    pub agent: String,
    pub owner: String,
    pub repo: String,
    pub workflows: bool,
}

impl Want {
    fn normalized(&self) -> Self {
        Self { agent: self.agent.clone(), owner: self.owner.to_ascii_lowercase(), repo: self.repo.to_ascii_lowercase(), workflows: self.workflows }
    }
}

/// GitHub's own rule for an owner or a repository name, narrowed: letters, digits, `-`, `_`, `.`.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name != "." && name != ".." && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A minted token and what to do with it.
#[derive(Clone)]
pub struct Minted {
    pub token: Zeroizing<String>,
    /// Epoch seconds.
    pub expires_at: i64,
    pub bot: AppBot,
}

impl std::fmt::Debug for Minted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Minted").field("expires_at", &self.expires_at).field("bot", &self.bot).finish_non_exhaustive()
    }
}

/// Why there is no token. [`Display`](std::fmt::Display) is what the person (or the agent) reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// The repository's owner isn't an installation this account has linked.
    NoInstallation { owner: String },
    /// `owner` or `repo` isn't a GitHub name.
    BadTarget(String),
    /// The broker or GitHub said no, or couldn't be reached.
    Broker(BrokerError),
    /// This AgentMux account has no GitHub connection to check a stale link with.
    NoGithubAccount,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInstallation { owner } => write!(
                f,
                "AgentMux can't act on {owner}'s repositories: the AgentMux GitHub App isn't installed there, or isn't linked to this AgentMux account. Install it in Connectors → Accounts"
            ),
            Self::BadTarget(what) => write!(f, "{what} isn't a GitHub owner or repository name"),
            Self::Broker(e) => write!(f, "{e}"),
            Self::NoGithubAccount => write!(f, "GitHub isn't connected on this AgentMux account: connect it in Connectors → Accounts"),
        }
    }
}

impl From<BrokerError> for TokenError {
    fn from(e: BrokerError) -> Self {
        Self::Broker(e)
    }
}

/// What a mint asks for, owned so it can be asked twice.
#[derive(Debug, Clone)]
pub struct MintRequest {
    pub installation_id: u64,
    pub repositories: Vec<String>,
    pub permissions: BTreeMap<String, String>,
    pub agent: String,
}

/// The broker, as this module needs it.
pub trait Backend {
    fn installations(&self) -> impl Future<Output = Result<Vec<LinkedInstallation>, BrokerError>> + Send;
    fn mint(&self, request: MintRequest) -> impl Future<Output = Result<github_broker::InstallationToken, BrokerError>> + Send;
    /// Present the person's own fresh GitHub token so the broker re-reads what they can see.
    fn revalidate(&self) -> impl Future<Output = Result<(), TokenError>> + Send;
}

struct Entry {
    minted: Minted,
}

#[derive(Default)]
struct State {
    cache: HashMap<Want, Entry>,
    installations: Option<(i64, Vec<LinkedInstallation>)>,
    refused: HashMap<Want, (i64, TokenError)>,
}

/// The tokens srv has minted for agents.
#[derive(Default)]
pub struct Tokens {
    state: tokio::sync::Mutex<State>,
}

impl Tokens {
    /// A token for `want`: the cached one while it has time left, else a new one.
    pub async fn token_for<B: Backend>(&self, backend: &B, want: &Want, now: i64) -> Result<Minted, TokenError> {
        for (what, name) in [("the owner", &want.owner), ("the repository", &want.repo)] {
            if !valid_name(name) {
                return Err(TokenError::BadTarget(format!("{what} {:?}", name.chars().take(40).collect::<String>())));
            }
        }
        let want = want.normalized();
        // One at a time: an agent's many commands at once must not each mint.
        let mut st = self.state.lock().await;
        if let Some(entry) = st.cache.get(&want) {
            if entry.minted.expires_at - now > REFRESH_MARGIN_SECS {
                return Ok(entry.minted.clone());
            }
        }
        if let Some((until, error)) = st.refused.get(&want) {
            if now < *until {
                return Err(error.clone());
            }
        }
        match Self::mint(backend, &mut st, &want, now).await {
            Ok(minted) => {
                st.refused.remove(&want);
                st.cache.insert(want, Entry { minted: minted.clone() });
                Ok(minted)
            }
            Err(error) => {
                st.cache.remove(&want);
                st.refused.insert(want, (now + REFUSAL_TTL_SECS, error.clone()));
                Err(error)
            }
        }
    }

    async fn mint<B: Backend>(backend: &B, st: &mut State, want: &Want, now: i64) -> Result<Minted, TokenError> {
        let installation = Self::installation_for(backend, st, &want.owner, now).await?;
        let request = MintRequest {
            installation_id: installation.installation_id,
            repositories: vec![want.repo.clone()],
            permissions: permissions_for(want.workflows),
            agent: want.agent.clone(),
        };
        let answer = match backend.mint(request.clone()).await {
            // The link wasn't verified lately: the person's own token proves it again, once.
            Err(BrokerError::LinkStale) => {
                backend.revalidate().await?;
                st.installations = None;
                backend.mint(request).await
            }
            Err(BrokerError::NotLinked) => {
                st.installations = None;
                Err(BrokerError::NotLinked)
            }
            other => other,
        }?;
        let expires_at = answer.expires_at_epoch().unwrap_or(now + FALLBACK_LIFETIME_SECS);
        tracing::info!(target: "identity", agent = %want.agent, owner = %want.owner, repo = %want.repo, installation = installation.installation_id, "identity.github: minted an agent token");
        Ok(Minted { token: Zeroizing::new(answer.token), expires_at, bot: answer.app })
    }

    async fn installation_for<B: Backend>(backend: &B, st: &mut State, owner: &str, now: i64) -> Result<LinkedInstallation, TokenError> {
        let fresh = matches!(&st.installations, Some((at, _)) if now - *at < INSTALLATIONS_TTL_SECS);
        if !fresh {
            let list = backend.installations().await?;
            st.installations = Some((now, list));
        }
        let list = st.installations.as_ref().map(|(_, l)| l.as_slice()).unwrap_or(&[]);
        let found = |list: &[LinkedInstallation]| list.iter().find(|i| i.is_usable() && i.github_account.login.eq_ignore_ascii_case(owner)).cloned();
        if let Some(i) = found(list) {
            return Ok(i);
        }
        // Not in a list this old: an install made in the last minute is worth one look.
        if fresh {
            let list = backend.installations().await?;
            st.installations = Some((now, list.clone()));
            if let Some(i) = found(&list) {
                return Ok(i);
            }
        }
        Err(TokenError::NoInstallation { owner: owner.to_string() })
    }

    /// The tokens held, for tests.
    #[cfg(test)]
    async fn cached(&self) -> usize {
        self.state.lock().await.cache.len()
    }
}

/// Set in the environment of an agent whose `git` and `gh` use the broker's tokens; the `gh`
/// function `agentmux-bashwrap` exports reads it.
pub const BROKER_ENV: &str = "AGENTMUX_GITHUB_BROKER";
const HELPER_KEY: &str = "credential.https://github.com.helper";
const HELPER: &str = "!agentmux-bashwrap git-credential";
const HTTP_PATH_KEY: &str = "credential.https://github.com.useHttpPath";
const REWRITE_KEY: &str = "url.https://github.com/.insteadOf";

/// Point an agent's `git` at the broker's tokens, or take that out again.
///
/// With `enabled`, GitHub over https asks `agentmux-bashwrap git-credential` and nothing else:
/// an empty helper before it clears every other (the person's stored login, a token cache), and
/// the repository's path goes with the request so the token can be narrowed to it. `ssh` remotes
/// to GitHub are rewritten to https, because an ssh key is the person's own credential. Prompts
/// are off, so a refusal ends the command with the reason instead of waiting at a prompt no one
/// sees. Without `enabled`, any of this left in the environment from an earlier start is removed.
pub fn apply_agent_git_in(env: &mut HashMap<String, String>, process: impl Fn(&str) -> Option<String> + Copy, enabled: bool) {
    use crate::backend::publish_guard::set_git_config_list;
    if enabled {
        set_git_config_list(env, process, HELPER_KEY, &["", HELPER]);
        set_git_config_list(env, process, HTTP_PATH_KEY, &["true"]);
        set_git_config_list(env, process, REWRITE_KEY, &["git@github.com:", "ssh://git@github.com/"]);
        env.insert("GIT_TERMINAL_PROMPT".to_string(), "0".to_string());
        env.insert(BROKER_ENV.to_string(), "1".to_string());
    } else if env.contains_key(BROKER_ENV) {
        for key in [HELPER_KEY, HTTP_PATH_KEY, REWRITE_KEY] {
            set_git_config_list(env, process, key, &[]);
        }
        env.remove(BROKER_ENV);
    }
}

static STORE: OnceLock<Arc<Store>> = OnceLock::new();

/// The account store, whose GitHub accounts decide whether agents use the broker.
pub fn attach_store(store: Arc<Store>) {
    let _ = STORE.set(store);
}

/// Whether agents on this machine get the broker's GitHub tokens: the person connected GitHub
/// through AgentMux Cloud (a brokered account that still works), and this isn't a fleet host,
/// where `gh-agent` already gives agents their GitHub identity.
fn agents_use_broker(env: &HashMap<String, String>) -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    if crate::backend::publish_guard::hooks_dir_for(env, |k| std::env::var(k).ok()).is_some() {
        return false;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, answer)) = *cache {
        if at.elapsed() < Duration::from_secs(5) {
            return answer;
        }
    }
    let answer = STORE.get().is_some_and(|store| {
        store
            .identity_list(Some(crate::identity::github_connect::PROVIDER))
            .unwrap_or_default()
            .iter()
            .any(|a| crate::identity::github_connect::is_brokered(a) && a.status != crate::identity::oauth_tokens::STATUS_NEEDS_REAUTH)
    });
    *cache = Some((Instant::now(), answer));
    answer
}

/// [`apply_agent_git_in`] for a spawn: every path that starts an agent process or runs a command for
/// one comes through `gh_guard::apply_gh_guard`, which calls this.
#[cfg_attr(test, allow(dead_code))]
pub fn apply_agent_git(env: &mut HashMap<String, String>) {
    let enabled = agents_use_broker(env);
    apply_agent_git_in(env, |k| std::env::var(k).ok(), enabled);
}

/// srv's tokens: one cache for the process.
pub fn global() -> &'static Tokens {
    static TOKENS: OnceLock<Tokens> = OnceLock::new();
    TOKENS.get_or_init(Tokens::default)
}

/// The broker as this install reaches it: the cloud sign-in and the instance key, and the person's
/// brokered GitHub account for a revalidation.
pub struct Live {
    pub mstore: Arc<Store>,
    pub identity_store: Arc<Store>,
}

impl Backend for Live {
    async fn installations(&self) -> Result<Vec<LinkedInstallation>, BrokerError> {
        let who = github_broker::caller(&self.mstore).await?;
        github_broker::installations(crate::identity::oauth_client::http(), &who).await
    }

    async fn mint(&self, request: MintRequest) -> Result<github_broker::InstallationToken, BrokerError> {
        let who = github_broker::caller(&self.mstore).await?;
        github_broker::installation_token(
            crate::identity::oauth_client::http(),
            &who,
            &InstallationRequest {
                installation_id: request.installation_id,
                repositories: &request.repositories,
                permissions: &request.permissions,
                agent_id: &request.agent,
            },
        )
        .await
    }

    async fn revalidate(&self) -> Result<(), TokenError> {
        let account = self
            .mstore
            .identity_list(Some(crate::identity::github_connect::PROVIDER))
            .unwrap_or_default()
            .into_iter()
            .find(crate::identity::github_connect::is_brokered)
            .ok_or(TokenError::NoGithubAccount)?;
        let token = crate::identity::oauth_tokens::access_token_for(
            &crate::identity::oauth_tokens::LiveHttp,
            &crate::identity::oauth_tokens::KeychainSecrets,
            &self.mstore,
            &self.identity_store,
            &account,
            Some(&self.mstore),
        )
        .await
        .map_err(|e| TokenError::Broker(BrokerError::NotSignedIn(e)))?;
        let who = github_broker::caller(&self.mstore).await?;
        github_broker::revalidate(crate::identity::oauth_client::http(), &who, &token).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::muxbus::github_broker::{InstallationAccount, InstallationToken};
    use std::sync::Mutex;

    fn link(id: u64, login: &str, installation: &str, link: &str) -> LinkedInstallation {
        LinkedInstallation {
            installation_id: id,
            github_account: InstallationAccount { id: id * 10, login: login.to_string() },
            repository_selection: "selected".to_string(),
            installation_status: installation.to_string(),
            link_status: link.to_string(),
            verified_at_ms: 0,
        }
    }

    fn token(expires_at: &str) -> InstallationToken {
        InstallationToken {
            token: "ghs_TOKEN".to_string(),
            expires_at: expires_at.to_string(),
            permissions: BTreeMap::new(),
            repository_ids: vec![1],
            app: AppBot { slug: "muxagent".to_string(), bot_user_id: 77 },
        }
    }

    /// A broker that answers from lists, and counts what it was asked.
    struct Fake {
        installs: Mutex<Vec<Vec<LinkedInstallation>>>,
        mints: Mutex<Vec<Result<InstallationToken, BrokerError>>>,
        asked: Mutex<Vec<MintRequest>>,
        listed: Mutex<u32>,
        revalidated: Mutex<u32>,
        revalidate_ok: bool,
    }

    impl Fake {
        fn new(installs: Vec<Vec<LinkedInstallation>>, mints: Vec<Result<InstallationToken, BrokerError>>) -> Self {
            Self { installs: Mutex::new(installs), mints: Mutex::new(mints), asked: Mutex::default(), listed: Mutex::new(0), revalidated: Mutex::new(0), revalidate_ok: true }
        }
    }

    impl Backend for Fake {
        async fn installations(&self) -> Result<Vec<LinkedInstallation>, BrokerError> {
            *self.listed.lock().unwrap() += 1;
            let mut l = self.installs.lock().unwrap();
            Ok(if l.len() > 1 { l.remove(0) } else { l[0].clone() })
        }
        async fn mint(&self, request: MintRequest) -> Result<InstallationToken, BrokerError> {
            self.asked.lock().unwrap().push(request);
            let mut m = self.mints.lock().unwrap();
            if m.len() > 1 { m.remove(0) } else { m[0].clone().map_err(|e| e) }
        }
        async fn revalidate(&self) -> Result<(), TokenError> {
            *self.revalidated.lock().unwrap() += 1;
            if self.revalidate_ok { Ok(()) } else { Err(TokenError::NoGithubAccount) }
        }
    }

    fn want(owner: &str, repo: &str) -> Want {
        Want { agent: "korp".to_string(), owner: owner.to_string(), repo: repo.to_string(), workflows: false }
    }

    const NOW: i64 = 1_793_178_000;

    fn ok_for(secs: i64) -> InstallationToken {
        let at = chrono::DateTime::from_timestamp(NOW + secs, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        token(&at)
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn entries(e: &HashMap<String, String>) -> Vec<(String, String)> {
        let n: usize = e.get("GIT_CONFIG_COUNT").and_then(|c| c.parse().ok()).unwrap_or(0);
        (0..n).map(|i| (e[&format!("GIT_CONFIG_KEY_{i}")].clone(), e[&format!("GIT_CONFIG_VALUE_{i}")].clone())).collect()
    }

    #[test]
    fn an_agent_with_the_broker_gets_only_its_credential_helper_for_github() {
        let mut e = env(&[("GIT_CONFIG_COUNT", "1"), ("GIT_CONFIG_KEY_0", "user.name"), ("GIT_CONFIG_VALUE_0", "x")]);
        apply_agent_git_in(&mut e, |_| None, true);
        let got = entries(&e);
        // The reset comes right before ours: whatever helper was configured before is not asked.
        let at = got.iter().position(|(k, v)| k == HELPER_KEY && v.is_empty()).expect("the reset");
        assert_eq!(got[at + 1], (HELPER_KEY.to_string(), HELPER.to_string()));
        assert!(got.contains(&(HTTP_PATH_KEY.to_string(), "true".to_string())));
        assert!(got.contains(&(REWRITE_KEY.to_string(), "git@github.com:".to_string())), "ssh remotes use the token too");
        assert!(got.contains(&("user.name".to_string(), "x".to_string())), "other settings stay");
        assert_eq!((e["GIT_TERMINAL_PROMPT"].as_str(), e[BROKER_ENV].as_str()), ("0", "1"));
        // Applied again (a persisted cmd:env): the same, not twice.
        let before = e.clone();
        apply_agent_git_in(&mut e, |_| None, true);
        assert_eq!(e, before);
    }

    #[test]
    fn an_agent_without_the_broker_keeps_its_own_git_and_loses_a_stale_setup() {
        let mut plain = env(&[("GIT_CONFIG_COUNT", "1"), ("GIT_CONFIG_KEY_0", "user.name"), ("GIT_CONFIG_VALUE_0", "x")]);
        let before = plain.clone();
        apply_agent_git_in(&mut plain, |_| None, false);
        assert_eq!(plain, before, "nothing added when GitHub isn't connected");
        // Disconnected since the last start: what was set is taken out, the rest stays.
        let mut e = before.clone();
        apply_agent_git_in(&mut e, |_| None, true);
        apply_agent_git_in(&mut e, |_| None, false);
        assert_eq!(entries(&e), vec![("user.name".to_string(), "x".to_string())]);
        assert!(!e.contains_key(BROKER_ENV));
    }

    #[tokio::test]
    async fn a_token_is_narrowed_to_the_repository_and_the_default_permissions() {
        let fake = Fake::new(vec![vec![link(5, "Octo-Org", "active", "active")]], vec![Ok(ok_for(3600))]);
        let tokens = Tokens::default();
        let minted = tokens.token_for(&fake, &want("octo-org", "Hello"), NOW).await.unwrap();
        assert_eq!((minted.token.as_str(), minted.bot.slug.as_str(), minted.expires_at), ("ghs_TOKEN", "muxagent", NOW + 3600));
        let asked = fake.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!((asked[0].installation_id, asked[0].repositories.clone(), asked[0].agent.as_str()), (5, vec!["hello".to_string()], "korp"));
        assert_eq!(asked[0].permissions, permissions_for(false));
        assert!(!asked[0].permissions.contains_key("workflows"), "no CI changes unless the person allowed them");
        assert!(!format!("{minted:?}").contains("ghs_TOKEN"));
    }

    #[test]
    fn workflows_are_asked_for_only_when_allowed() {
        assert_eq!(permissions_for(true).get("workflows").map(String::as_str), Some("write"));
        assert!(!permissions_for(false).contains_key("workflows"));
        assert_eq!(permissions_for(false).get("metadata").map(String::as_str), Some("read"));
    }

    #[tokio::test]
    async fn the_cache_serves_a_token_until_five_minutes_remain() {
        let fake = Fake::new(vec![vec![link(5, "o", "active", "active")]], vec![Ok(ok_for(3600)), Ok(ok_for(7200))]);
        let tokens = Tokens::default();
        let w = want("o", "r");
        tokens.token_for(&fake, &w, NOW).await.unwrap();
        tokens.token_for(&fake, &w, NOW + 3000).await.unwrap();
        assert_eq!(fake.asked.lock().unwrap().len(), 1, "an agent's many commands share one mint");
        // 299 s left: renewed.
        let renewed = tokens.token_for(&fake, &w, NOW + 3600 - 299).await.unwrap();
        assert_eq!(fake.asked.lock().unwrap().len(), 2);
        assert_eq!(renewed.expires_at, NOW + 7200);
        // Another agent, or workflows on, is another token.
        tokens.token_for(&fake, &Want { agent: "agenty".into(), ..w.clone() }, NOW).await.unwrap();
        tokens.token_for(&fake, &Want { workflows: true, ..w.clone() }, NOW).await.unwrap();
        assert_eq!(fake.asked.lock().unwrap().len(), 4);
        assert_eq!(tokens.cached().await, 3);
    }

    #[tokio::test]
    async fn an_owner_without_an_active_link_gets_a_plain_answer_and_no_mint() {
        let fake = Fake::new(
            vec![vec![link(5, "other", "active", "active"), link(6, "o", "suspended", "active"), link(7, "o", "active", "revoked")]],
            vec![Ok(ok_for(3600))],
        );
        let tokens = Tokens::default();
        let err = tokens.token_for(&fake, &want("o", "r"), NOW).await.unwrap_err();
        assert_eq!(err, TokenError::NoInstallation { owner: "o".to_string() });
        assert!(err.to_string().contains("Install it in Connectors"), "{err}");
        assert!(fake.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refusal_is_remembered_briefly_so_a_command_does_not_ask_over_and_over() {
        let fake = Fake::new(vec![vec![link(5, "o", "active", "active")]], vec![Err(BrokerError::RepositoryNotVisible("r".into())), Ok(ok_for(3600))]);
        let tokens = Tokens::default();
        let w = want("o", "r");
        let first = tokens.token_for(&fake, &w, NOW).await.unwrap_err();
        let again = tokens.token_for(&fake, &w, NOW + 5).await.unwrap_err();
        assert_eq!(first, again);
        assert_eq!(fake.asked.lock().unwrap().len(), 1);
        assert!(tokens.token_for(&fake, &w, NOW + 25).await.is_ok(), "asked again after the refusal expires");
    }

    #[tokio::test]
    async fn a_stale_link_is_revalidated_once_and_the_mint_retried() {
        let fake = Fake::new(vec![vec![link(5, "o", "active", "active")]], vec![Err(BrokerError::LinkStale), Ok(ok_for(3600))]);
        let tokens = Tokens::default();
        let minted = tokens.token_for(&fake, &want("o", "r"), NOW).await.unwrap();
        assert_eq!(minted.expires_at, NOW + 3600);
        assert_eq!((*fake.revalidated.lock().unwrap(), fake.asked.lock().unwrap().len()), (1, 2));
    }

    #[tokio::test]
    async fn a_stale_link_that_cannot_be_revalidated_gives_no_token() {
        let mut fake = Fake::new(vec![vec![link(5, "o", "active", "active")]], vec![Err(BrokerError::LinkStale)]);
        fake.revalidate_ok = false;
        let err = Tokens::default().token_for(&fake, &want("o", "r"), NOW).await.unwrap_err();
        assert_eq!(err, TokenError::NoGithubAccount);
        assert_eq!(fake.asked.lock().unwrap().len(), 1, "no second try without a fresh proof");
    }

    #[tokio::test]
    async fn a_new_install_is_found_without_waiting_for_the_list_to_age() {
        let fake = Fake::new(vec![vec![], vec![link(5, "o", "active", "active")]], vec![Ok(ok_for(3600))]);
        let tokens = Tokens::default();
        // The first look is stale-proof: an empty list, then the owner is installed and asked about again.
        assert!(tokens.token_for(&fake, &want("nobody", "r"), NOW).await.is_err());
        assert!(tokens.token_for(&fake, &want("o", "r"), NOW + 1).await.is_ok());
        assert_eq!(*fake.listed.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn names_that_are_not_github_names_never_reach_the_broker() {
        let fake = Fake::new(vec![vec![link(5, "o", "active", "active")]], vec![Ok(ok_for(3600))]);
        let tokens = Tokens::default();
        for (owner, repo) in [("o", "../x"), ("o/p", "r"), ("", "r"), ("o", ""), ("o", "a b"), ("o", &"x".repeat(101))] {
            let err = tokens.token_for(&fake, &want(owner, repo), NOW).await.unwrap_err();
            assert!(matches!(err, TokenError::BadTarget(_)), "{owner:?}/{repo:?}: {err}");
        }
        assert_eq!((*fake.listed.lock().unwrap(), fake.asked.lock().unwrap().len()), (0, 0));
    }

    #[tokio::test]
    async fn a_broker_that_cannot_mint_yet_says_so_and_gives_no_token() {
        let fake = Fake::new(vec![vec![link(5, "o", "active", "active")]], vec![Err(BrokerError::NotConfigured)]);
        let err = Tokens::default().token_for(&fake, &want("o", "r"), NOW).await.unwrap_err();
        assert_eq!(err, TokenError::Broker(BrokerError::NotConfigured));
        assert!(err.to_string().contains("can't mint GitHub tokens yet"), "{err}");
    }
}
