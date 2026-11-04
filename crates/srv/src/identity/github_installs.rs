// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Connect with GitHub, desktop side: the installations of the AgentMux GitHub App.
//!
//! An account's agents get tokens for an installation only once the account is *linked* to it
//! (the broker's record, `muxbus::github_broker`). This is what Connectors → Accounts drives
//! (`github.installations`):
//!
//! - **list**: the account's links, as rows;
//! - **start**: open GitHub's install page for the App, and remember which installations the
//!   person's GitHub sign-in could already see, so that a new one is theirs;
//! - **sync**: link an installation that appeared since `start` (called while the person is on
//!   GitHub; the broker checks the installation against their own token);
//! - **refresh**: present a fresh user token to the broker, which re-reads what the account can see
//!   (`links/revalidate`), and link every installation the sign-in sees that isn't linked yet;
//! - **unlink**: end the account's link; the installation stays on GitHub.
//!
//! The GitHub user token is read here and goes to the broker and to GitHub only; nothing of it is
//! stored or shown.

use crate::backend::storage::store::Store;
use crate::muxbus::github_broker::{self, BrokerError, LinkedInstallation};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// What `github.installations` is asked to do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum GithubInstallationsAction {
    #[default]
    List,
    Start,
    Sync,
    Refresh,
    Unlink,
}

/// Request for `github.installations`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct GithubInstallationsReq {
    #[serde(default)]
    pub action: GithubInstallationsAction,
    /// `unlink`: the installation's id, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<String>,
}

/// One of the account's links to an installation, for the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct GithubInstallation {
    /// GitHub's installation id, as text.
    pub installation_id: String,
    /// The GitHub account (a person or an organization) the App is installed on.
    pub account_login: String,
    /// `all` or `selected`: every repository of the account, or the ones chosen.
    pub repository_selection: String,
    /// `active`, `suspended`, `deleted` or `revoked` (the account's link was cut: access lost on GitHub).
    pub status: String,
    /// How many of its repositories this account can see.
    pub visible_count: u32,
    #[ts(type = "number")]
    pub verified_at_ms: i64,
}

/// What `github.installations` answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct GithubInstallations {
    /// Signed in to AgentMux Cloud. Without it there is nothing to ask.
    pub signed_in: bool,
    /// Why the call failed, in words for the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub installations: Vec<GithubInstallation>,
    /// Installations this call linked (ids as text).
    #[serde(default)]
    pub linked: Vec<String>,
    /// `start`: the page that was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_url: Option<String>,
}

/// At most this many installations are linked by one call.
const MAX_LINKS_PER_CALL: usize = 10;
/// GitHub's pages of `GET /user/installations`, and the most read.
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 5;
/// An install trip that nobody finishes is forgotten after this long.
const TRIP_TTL_MS: i64 = 30 * 60 * 1000;

/// An install trip in progress: what the person could see when it began, and the state the broker issued.
struct Trip {
    before: HashSet<u64>,
    state: Option<String>,
    started_ms: i64,
}

fn trip() -> &'static Mutex<Option<Trip>> {
    static TRIP: Mutex<Option<Trip>> = Mutex::new(None);
    &TRIP
}

/// The row for one link.
pub fn row_of(link: &LinkedInstallation) -> GithubInstallation {
    let status = if link.link_status != "active" { "revoked" } else { link.installation_status.as_str() };
    GithubInstallation {
        installation_id: link.installation_id.to_string(),
        account_login: link.github_account.login.clone(),
        repository_selection: link.repository_selection.clone(),
        status: status.to_string(),
        visible_count: link.visible_count.min(u64::from(u32::MAX)) as u32,
        verified_at_ms: link.verified_at_ms,
    }
}

/// The installations on GitHub that the account has no live link to.
pub fn unlinked(on_github: &[u64], linked: &[LinkedInstallation]) -> Vec<u64> {
    on_github.iter().copied().filter(|id| !linked.iter().any(|l| l.installation_id == *id && l.link_status == "active")).collect()
}

/// The installations that were not there when the trip began.
pub fn new_since(before: &HashSet<u64>, on_github: &[u64]) -> Vec<u64> {
    on_github.iter().copied().filter(|id| !before.contains(id)).collect()
}

/// The install page is only opened if it is the App's page on GitHub: a short list of characters a
/// page address needs after the prefix, so nothing in it means anything to the shell that opens it.
pub fn is_install_page(url: &str) -> bool {
    url.strip_prefix("https://github.com/apps/")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || "-._~/?=&".contains(c)))
}

/// The installation ids in a page of `GET /user/installations`.
pub fn installation_ids(page: &serde_json::Value) -> Vec<u64> {
    page.get("installations").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|i| i.get("id").and_then(|id| id.as_u64())).collect()).unwrap_or_default()
}

/// Every installation of the App that this GitHub sign-in can see.
async fn user_installations(token: &str) -> Result<Vec<u64>, String> {
    let mut all = Vec::new();
    for page in 1..=MAX_PAGES {
        let reply = crate::identity::oauth_client::http()
            .get(format!("https://api.github.com/user/installations?per_page={PAGE_SIZE}&page={page}"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| format!("couldn't ask GitHub for your installations: {}", e.without_url()))?;
        let status = reply.status().as_u16();
        if status == 401 {
            return Err("GitHub no longer accepts this connection: connect GitHub again".to_string());
        }
        if !(200..300).contains(&status) {
            return Err(format!("GitHub answered {status} when asked for your installations"));
        }
        let body: serde_json::Value = reply.json().await.map_err(|_| "GitHub's list of your installations wasn't readable".to_string())?;
        let ids = installation_ids(&body);
        let full = ids.len() >= PAGE_SIZE;
        all.extend(ids);
        if !full {
            break;
        }
    }
    Ok(all)
}

/// The person's own GitHub token, from the brokered account (refreshed if it is near its end).
async fn user_token(mstore: &Arc<Store>, identity_store: &Arc<Store>) -> Result<zeroize::Zeroizing<String>, String> {
    let account = mstore
        .identity_list(Some(crate::identity::github_connect::PROVIDER))
        .unwrap_or_default()
        .into_iter()
        .find(crate::identity::github_connect::is_brokered)
        .ok_or("GitHub isn't connected: connect GitHub first")?;
    crate::identity::oauth_tokens::access_token_for(
        &crate::identity::oauth_tokens::LiveHttp,
        &crate::identity::oauth_tokens::KeychainSecrets,
        mstore,
        identity_store,
        &account,
        Some(mstore),
    )
    .await
    .map_err(|e| format!("GitHub needs to be connected again ({e})"))
}

fn text(e: BrokerError) -> String {
    e.to_string()
}

/// A state for the next link: the one the trip holds, else a new one.
async fn state_for_link(http: &reqwest::Client, who: &github_broker::Caller) -> Result<String, BrokerError> {
    let held = trip().lock().ok().and_then(|mut t| t.as_mut().and_then(|t| t.state.take()));
    match held {
        Some(state) => Ok(state),
        None => Ok(github_broker::installation_state(http, who).await?.state),
    }
}

/// Link each of `ids` that the person's token proves they can see; the ones it doesn't are skipped.
async fn link_all(http: &reqwest::Client, who: &github_broker::Caller, token: &str, ids: Vec<u64>) -> Result<Vec<u64>, String> {
    let mut done = Vec::new();
    for id in ids.into_iter().take(MAX_LINKS_PER_CALL) {
        let state = state_for_link(http, who).await.map_err(text)?;
        match github_broker::link_installation(http, who, id, &state, token).await {
            Ok(_) => done.push(id),
            // Not theirs to link (or it just went), too big for the record, or the state is spent: next time.
            Err(BrokerError::InstallationNotVisible | BrokerError::StateInvalid | BrokerError::InstallationTooLarge) => {}
            Err(e) => return Err(text(e)),
        }
    }
    Ok(done)
}

async fn start(http: &reqwest::Client, who: &github_broker::Caller, token: &str) -> Result<String, String> {
    let before: HashSet<u64> = user_installations(token).await?.into_iter().collect();
    let issued = github_broker::installation_state(http, who).await.map_err(text)?;
    if !is_install_page(&issued.install_url) {
        return Err("AgentMux Cloud gave an install page that isn't GitHub's: not opening it".to_string());
    }
    if let Ok(mut t) = trip().lock() {
        *t = Some(Trip { before, state: Some(issued.state), started_ms: agentmux_common::time::now_ms() });
    }
    crate::util::open_browser(&issued.install_url);
    Ok(issued.install_url)
}

/// What the call did, and the account's links after it.
pub async fn run(mstore: &Arc<Store>, identity_store: &Arc<Store>, req: GithubInstallationsReq) -> GithubInstallations {
    let who = match github_broker::caller(mstore).await {
        Ok(who) => who,
        Err(e) => return GithubInstallations { signed_in: false, error: Some(text(e)), ..Default::default() },
    };
    let http = crate::identity::oauth_client::http();
    let mut out = GithubInstallations { signed_in: true, ..Default::default() };
    let done: Result<(), String> = async {
        match req.action {
            GithubInstallationsAction::List => {}
            GithubInstallationsAction::Start => {
                let token = user_token(mstore, identity_store).await?;
                out.install_url = Some(start(http, &who, &token).await?);
            }
            GithubInstallationsAction::Sync => {
                let before = trip().lock().ok().and_then(|t| {
                    t.as_ref().filter(|t| agentmux_common::time::now_ms() - t.started_ms < TRIP_TTL_MS).map(|t| t.before.clone())
                });
                // Nothing was started here (or it was long ago): there is nothing that is new.
                if let Some(before) = before {
                    let token = user_token(mstore, identity_store).await?;
                    let ids = new_since(&before, &user_installations(&token).await?);
                    out.linked = link_all(http, &who, &token, ids).await?.iter().map(u64::to_string).collect();
                }
            }
            GithubInstallationsAction::Refresh => {
                let token = user_token(mstore, identity_store).await?;
                github_broker::revalidate(http, &who, &token).await.map_err(text)?;
                let linked = github_broker::installations(http, &who).await.map_err(text)?;
                let ids = unlinked(&user_installations(&token).await?, &linked);
                out.linked = link_all(http, &who, &token, ids).await?.iter().map(u64::to_string).collect();
            }
            GithubInstallationsAction::Unlink => {
                let id: u64 = req.installation_id.as_deref().and_then(|s| s.parse().ok()).ok_or("unlink needs the installation's id")?;
                github_broker::unlink_installation(http, &who, id).await.map_err(text)?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = done {
        out.error = Some(e);
    }
    match github_broker::installations(http, &who).await {
        Ok(list) => out.installations = list.iter().map(row_of).collect(),
        Err(e) if out.error.is_none() => out.error = Some(text(e)),
        Err(_) => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::muxbus::github_broker::InstallationAccount;

    fn link(id: u64, installation: &str, link: &str) -> LinkedInstallation {
        LinkedInstallation {
            installation_id: id,
            github_account: InstallationAccount { id: id * 10, login: format!("org{id}") },
            repository_selection: "selected".to_string(),
            installation_status: installation.to_string(),
            link_status: link.to_string(),
            verified_at_ms: 5,
            visible_count: 3,
            linked_at_ms: 1,
        }
    }

    #[test]
    fn a_row_says_what_the_person_can_use() {
        let r = row_of(&link(7, "active", "active"));
        assert_eq!((r.installation_id.as_str(), r.account_login.as_str(), r.status.as_str(), r.visible_count), ("7", "org7", "active", 3));
        // A cut link shows as revoked whatever the installation says; a suspended installation says so.
        assert_eq!(row_of(&link(7, "active", "revoked")).status, "revoked");
        assert_eq!(row_of(&link(7, "suspended", "active")).status, "suspended");
        assert_eq!(row_of(&link(7, "deleted", "active")).status, "deleted");
    }

    #[test]
    fn a_new_installation_is_one_that_was_not_there_when_the_trip_began() {
        let before: HashSet<u64> = [1, 2].into_iter().collect();
        assert_eq!(new_since(&before, &[1, 2, 3, 4]), vec![3, 4]);
        assert!(new_since(&before, &[2, 1]).is_empty());
    }

    #[test]
    fn a_refresh_links_what_the_sign_in_sees_and_has_no_live_link() {
        let linked = vec![link(1, "active", "active"), link(2, "active", "revoked")];
        assert_eq!(unlinked(&[1, 2, 3], &linked), vec![2, 3]);
        assert!(unlinked(&[1], &linked).is_empty());
    }

    #[test]
    fn only_githubs_page_for_the_app_is_opened() {
        assert!(is_install_page("https://github.com/apps/muxagent/installations/new?state=abc"));
        for bad in ["http://github.com/apps/muxagent", "https://evil.example/apps/x", "https://github.com/apps/", "https://github.com.evil.example/apps/x", "https://github.com/apps/a@b", "https://github.com/apps/x\" & calc", "https://github.com/apps/x%20y", "https://github.com/apps/x^y", "https://github.com/apps/x\ny", "https://github.com/apps/x'y", "https://github.com/apps/x;y", "https://github.com/apps/x|y"] {
            assert!(!is_install_page(bad), "{bad}");
        }
    }

    #[test]
    fn the_ids_come_out_of_githubs_page() {
        let page = serde_json::json!({ "total_count": 2, "installations": [{ "id": 11, "account": { "login": "a" } }, { "id": 22 }, { "name": "no id" }] });
        assert_eq!(installation_ids(&page), vec![11, 22]);
        assert!(installation_ids(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_broker_s_refusals_read_as_sentences() {
        use crate::muxbus::github_broker::error_for;
        let body = |code: &str| serde_json::json!({ "error": code });
        assert_eq!(error_for(403, &body("installation_not_visible")), BrokerError::InstallationNotVisible);
        assert_eq!(error_for(400, &body("state_invalid")), BrokerError::StateInvalid);
        assert_eq!(error_for(422, &body("installation_too_large")), BrokerError::InstallationTooLarge);
        assert!(BrokerError::StateInvalid.to_string().contains("start it again"));
    }
}
