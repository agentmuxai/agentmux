// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/github/token`: the GitHub token an agent's `git` and `gh` use.
//!
//! `agentmux-bashwrap` asks here, from the agent's own shell, as git's credential helper and
//! before each wrapped command. Who is asking is the agent its `X-Agent-Token` was minted for,
//! never a name in the request; the request says only which repository. The token is the AgentMux
//! GitHub App's installation token, narrowed to that repository by AgentMux Cloud's broker
//! (`identity::github_tokens`). The answer also says who the agent works for (the GitHub account's id
//! and login) and who the App's bot is, for the PR tag and the commit's author. When there is no
//! token, the answer says why, in words an agent can show; nothing here falls back to another credential.

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::identity::github_connect;
use crate::identity::github_tokens::{self, TokenError, Want};
use crate::muxbus::github_broker::BrokerError;
use crate::sagas::credential_notice::{self, Cause};
use crate::server::caller::Caller;

#[derive(Debug, Deserialize)]
pub(crate) struct GithubTokenRequest {
    owner: String,
    repo: String,
    /// What the token is for, as the caller says: `git` from git's credential helper, anything else (or nothing) is a
    /// `gh` call. It only ever narrows: a container that says `git` gets a token that writes contents and nothing
    /// else, one that says anything else gets a token that cannot write contents. It is never the way to a wider token.
    #[serde(default)]
    purpose: Option<String>,
}

/// The scope of the token a container on a gh-agent host is given for `purpose`: only git's own asks for a push token.
fn scope_for(purpose: Option<&str>) -> crate::identity::gh_agent_token::Scope {
    match purpose {
        Some("git") => crate::identity::gh_agent_token::Scope::Push,
        _ => crate::identity::gh_agent_token::Scope::Api,
    }
}

/// Whether a push token that failed is asked for again without workflows write: an App without it cannot give that, and the push
/// token without it still pushes everything else. Not after a gh-agent that is too old (it would be again), and not after one
/// that was slow: a second wait would run past the container's own, and its answer would never arrive.
fn retry_without_workflows(workflows: bool, scope: crate::identity::gh_agent_token::Scope, failure: Option<&String>) -> bool {
    use crate::identity::gh_agent_token::{is_too_old, is_took_too_long, Scope};
    workflows && scope == Scope::Push && failure.is_some_and(|e| !is_too_old(e) && !is_took_too_long(e))
}

/// What an agent is told when the person isn't signed in to AgentMux Cloud (the Connect with GitHub design).
const NEEDS_SIGN_IN_FOR_AGENT: &str = "GitHub needs the person to sign in to AgentMux Cloud (Connectors → Accounts)";

/// The status a refusal is answered with: the request's fault (4xx) or a bad moment (503).
fn status_for(e: &TokenError) -> StatusCode {
    match e {
        TokenError::BadTarget(_) => StatusCode::BAD_REQUEST,
        TokenError::NoInstallation { .. } | TokenError::NoGithubAccount => StatusCode::FORBIDDEN,
        TokenError::Broker(BrokerError::Unavailable(_)) => StatusCode::SERVICE_UNAVAILABLE,
        TokenError::Broker(_) => StatusCode::FORBIDDEN,
    }
}

/// `POST /api/v1/github/token`
pub(crate) async fn handle_github_token(
    State(state): State<AppState>,
    caller: Option<Extension<Caller>>,
    Json(req): Json<GithubTokenRequest>,
) -> Response {
    let refuse = |status: StatusCode, message: String| (status, Json(json!({ "error": message }))).into_response();
    let Some(uid) = caller.as_ref().and_then(|Extension(c)| c.uid()).map(str::to_string) else {
        return refuse(StatusCode::UNAUTHORIZED, "no agent token: GitHub tokens are for an agent's own commands".to_string());
    };
    let mstore = state.mstore.clone();
    let lookup = uid.clone();
    let (slug, in_container) = match tokio::task::spawn_blocking(move || mstore.agent_def_get(&lookup)).await {
        Ok(Ok(Some(def))) => { let in_container = def.runtime().is_container(); (def.slug, in_container) }
        Ok(Ok(None)) => return refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine".to_string()),
        _ => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "couldn't read the agent".to_string()),
    };
    // The person's GitHub account: who the agent works for, and whether this agent may change CI workflows.
    let accounts = state.id_store.clone();
    let account = tokio::task::spawn_blocking(move || {
        accounts.identity_list(Some(github_connect::PROVIDER)).unwrap_or_default().into_iter().find(github_connect::is_brokered)
    })
    .await
    .ok()
    .flatten();
    let owner = account.as_ref().and_then(github_connect::owner_of);
    let workflows = account.as_ref().is_some_and(|a| github_connect::workflows_allowed(a, &slug));
    // A container agent is given a token only where its git setup was switched on (the hooks mounted,
    // `github_tokens::container_git_hooks`), and from the host's `gh-agent` first.
    let mut host_error = None;
    if in_container {
        if crate::identity::github_tokens::container_git_hooks().is_none() {
            tracing::warn!(target: "identity", agent = %slug, "identity.github: a container agent asked for a token where its git setup is off");
            return refuse(StatusCode::FORBIDDEN, "no GitHub token for a container agent on this machine".to_string());
        }
        let scope = scope_for(req.purpose.as_deref());
        let mut asked = crate::identity::gh_agent_token::token_for(&slug, &req.owner, &req.repo, scope, workflows).await;
        if retry_without_workflows(workflows, scope, asked.as_ref().err()) {
            asked = crate::identity::gh_agent_token::token_for(&slug, &req.owner, &req.repo, scope, false).await;
        }
        match asked {
            Ok(host) => {
                let now = chrono::Utc::now().timestamp();
                tracing::info!(target: "identity", agent = %slug, owner = %req.owner, repo = %req.repo, "identity.github: a container agent's token from the host's gh-agent");
                credential_notice::worked(&state, &uid, &slug, Cause::Github);
                return Json(json!({
                    "token": host.token.as_str(),
                    "expiresAt": if host.expires_at > now { host.expires_at } else { now + 300 },
                    "botSlug": host.bot_slug,
                    // gh-agent doesn't say the bot's user id: it is looked up (once), so a commit is the
                    // bot's like a native agent's; the owner is the connected account's, when there is one.
                    "botUserId": match host.bot_slug.as_deref() {
                        Some(bot) => crate::identity::gh_agent_token::bot_user_id(bot).await,
                        None => None,
                    },
                    "ownerGithubId": owner.as_ref().map(|(id, _)| *id),
                    "ownerLogin": owner.as_ref().map(|(_, login)| login.as_str()),
                }))
                .into_response();
            }
            // A gh-agent that cannot give a narrowed token is not a reason to give a container another one.
            Err(e) if crate::identity::gh_agent_token::is_too_old(&e) => {
                tracing::warn!(target: "identity", agent = %slug, owner = %req.owner, repo = %req.repo, error = %e, "identity.github: a container agent's token was refused");
                return refuse(StatusCode::FORBIDDEN, e);
            }
            // Nor is a gh-agent that was only slow: a container that loads the machine must not be able to turn a slow answer
            // into the connection's token (below), which is per repository but can write contents.
            Err(e) if crate::identity::gh_agent_token::is_took_too_long(&e) => {
                tracing::warn!(target: "identity", agent = %slug, owner = %req.owner, repo = %req.repo, error = %e, "identity.github: a container agent's token was refused");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, e);
            }
            // Any other failure of gh-agent (a repository its App does not cover, no identity for the agent) falls through to
            // the person's own GitHub connection, as it always did. That token is not narrowed by this change; it is the
            // Cloud-connected path, which a machine without gh-agent uses throughout.
            Err(e) => host_error = Some(e),
        }
    }
    let backend = github_tokens::Live { mstore: state.id_store.clone(), identity_store: state.identity_store.clone() };
    let want = Want { agent: slug, owner: req.owner, repo: req.repo, workflows };
    match github_tokens::global().token_for(&backend, &want, chrono::Utc::now().timestamp()).await {
        Ok(minted) => {
            // (A native agent never has a line up, so this is a lookup that finds nothing.)
            credential_notice::worked(&state, &uid, &want.agent, Cause::Github);
            Json(json!({
                "token": minted.token.as_str(),
                "expiresAt": minted.expires_at,
                "botSlug": minted.bot.slug,
                "botUserId": minted.bot.bot_user_id,
                "ownerGithubId": owner.as_ref().map(|(id, _)| *id),
                "ownerLogin": owner.as_ref().map(|(_, login)| login.as_str()),
            }))
            .into_response()
        }
        Err(e) => {
            tracing::info!(target: "identity", agent = %want.agent, owner = %want.owner, repo = %want.repo, error = %e, "identity.github: no agent token");
            let (status, message) = refusal_shown(&state, &uid, &want.agent, in_container, host_error, &e);
            refuse(status, message)
        }
    }
}

/// [`refusal_for`], and for a container agent the sentence it was given is also put in its pane, when the person can fix it.
fn refusal_shown(state: &AppState, uid: &str, slug: &str, in_container: bool, host_error: Option<String>, e: &TokenError) -> (StatusCode, String) {
    let shows = in_container && the_persons_to_fix(e, host_error.as_deref());
    let (status, message) = refusal_for(host_error, e);
    if shows {
        credential_notice::refused(state, uid, slug, Cause::Github, &message);
    }
    (status, message)
}

/// Whether a refusal is one the person can fix, so a container agent's pane says so: GitHub isn't connected, the
/// App isn't installed where the repository is, or the Cloud sign-in lapsed. A slow `gh-agent` is only a bad moment.
fn the_persons_to_fix(e: &TokenError, host_error: Option<&str>) -> bool {
    let slow = host_error.is_some_and(crate::identity::gh_agent_token::is_took_too_long);
    !slow && matches!(e, TokenError::NoGithubAccount | TokenError::NoInstallation { .. } | TokenError::Broker(BrokerError::NotSignedIn(_) | BrokerError::NotConnected))
}

/// What a refusal says, and its status, once the broker path has failed too (`host_error`: what `gh-agent` said
/// first, for a container agent).
fn refusal_for(host_error: Option<String>, e: &TokenError) -> (StatusCode, String) {
    // `gh-agent` is there but ran out of time (a busy machine, a slow network): the broker has no account because
    // none was ever needed, so "isn't connected" would send the person to fix what isn't broken. Ask again.
    let nothing_to_connect = matches!(e, TokenError::NoGithubAccount | TokenError::Broker(BrokerError::NotSignedIn(_) | BrokerError::NotConnected));
    if let Some(host) = host_error.as_deref().filter(|h| crate::identity::gh_agent_token::is_took_too_long(h)) {
        if nothing_to_connect {
            return (StatusCode::SERVICE_UNAVAILABLE, host.to_string());
        }
    }
    // A lapsed or missing Cloud sign-in is the person's to fix, in words an agent can show.
    // The reason rides along: "not running yet" or "key can't be read" are not fixed by signing in.
    let said = match e {
        TokenError::Broker(BrokerError::NotSignedIn(why)) => format!("{NEEDS_SIGN_IN_FOR_AGENT} ({why})"),
        other => other.to_string(),
    };
    // A container agent was tried against gh-agent first: say both, in words it can show.
    match host_error {
        Some(host) => (status_for(e), format!("{host}; {said}")),
        None => (status_for(e), said),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_git_asks_for_a_push_token_and_nothing_else_is_wider_than_api() {
        use crate::identity::gh_agent_token::Scope;
        assert_eq!(scope_for(Some("git")), Scope::Push);
        for other in [None, Some("api"), Some(""), Some("push"), Some("GIT"), Some("admin"), Some("installation")] {
            assert_eq!(scope_for(other), Scope::Api, "{other:?}");
        }
    }

    #[test]
    fn a_slow_push_mint_is_not_asked_again_without_workflows() {
        use crate::identity::gh_agent_token::{took_too_long, Scope, GH_AGENT_TOO_OLD};
        let other = "gh-agent gave no token for that repository".to_string();
        assert!(retry_without_workflows(true, Scope::Push, Some(&other)));
        assert!(!retry_without_workflows(true, Scope::Push, Some(&took_too_long())), "a second wait would outlast the container's");
        assert!(!retry_without_workflows(true, Scope::Push, Some(&GH_AGENT_TOO_OLD.to_string())));
        assert!(!retry_without_workflows(false, Scope::Push, Some(&other)));
        assert!(!retry_without_workflows(true, Scope::Api, Some(&other)));
        assert!(!retry_without_workflows(true, Scope::Push, None));
    }

    #[test]
    fn a_request_without_a_purpose_is_still_a_request() {
        let r: GithubTokenRequest = serde_json::from_str(r#"{"owner":"o","repo":"r"}"#).unwrap();
        assert_eq!(r.purpose, None);
        let r: GithubTokenRequest = serde_json::from_str(r#"{"owner":"o","repo":"r","purpose":"git"}"#).unwrap();
        assert_eq!(scope_for(r.purpose.as_deref()), crate::identity::gh_agent_token::Scope::Push);
    }

    #[test]
    fn a_refusal_is_the_requests_fault_unless_it_was_only_a_bad_moment() {
        assert_eq!(status_for(&TokenError::BadTarget("x".into())), StatusCode::BAD_REQUEST);
        assert_eq!(status_for(&TokenError::NoInstallation { owner: "o".into() }), StatusCode::FORBIDDEN);
        assert_eq!(status_for(&TokenError::NoGithubAccount), StatusCode::FORBIDDEN);
        assert_eq!(status_for(&TokenError::Broker(BrokerError::NotConnected)), StatusCode::FORBIDDEN);
        assert_eq!(status_for(&TokenError::Broker(BrokerError::Unavailable("x".into()))), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn a_gh_agent_that_ran_out_of_time_is_not_reported_as_a_missing_connection() {
        let slow = Some(crate::identity::gh_agent_token::took_too_long());
        for e in [TokenError::NoGithubAccount, TokenError::Broker(BrokerError::NotConnected), TokenError::Broker(BrokerError::NotSignedIn("not running yet".into()))] {
            let (status, message) = refusal_for(slow.clone(), &e);
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert!(message.starts_with("GitHub token took too long") && !message.contains("isn't connected") && !message.contains("sign in"), "{message}");
        }
        // Anything else the broker said is still said, after the timeout.
        let (status, message) = refusal_for(slow, &TokenError::NoInstallation { owner: "o".into() });
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(message.starts_with("GitHub token took too long") && message.contains("Try again in a minute; ") && message.contains("isn't installed"), "{message}");
    }

    #[test]
    fn only_a_refusal_the_person_can_fix_is_shown_in_the_agents_pane() {
        let slow = crate::identity::gh_agent_token::took_too_long();
        for e in [TokenError::NoGithubAccount, TokenError::NoInstallation { owner: "o".into() }, TokenError::Broker(BrokerError::NotConnected), TokenError::Broker(BrokerError::NotSignedIn("lapsed".into()))] {
            assert!(the_persons_to_fix(&e, None), "{e:?}");
            assert!(the_persons_to_fix(&e, Some("gh-agent gave no token for that repository")), "{e:?}");
            assert!(!the_persons_to_fix(&e, Some(&slow)), "a slow gh-agent is a bad moment, not a connection to fix: {e:?}");
        }
        assert!(!the_persons_to_fix(&TokenError::BadTarget("x".into()), None));
        assert!(!the_persons_to_fix(&TokenError::Broker(BrokerError::Unavailable("x".into())), None));
    }

    #[test]
    fn without_a_timeout_the_refusals_read_as_before() {
        let (status, message) = refusal_for(None, &TokenError::NoGithubAccount);
        assert_eq!((status, message.as_str()), (StatusCode::FORBIDDEN, "GitHub isn't connected on this AgentMux account: connect it in Connectors → Accounts"));
        let (_, signed_out) = refusal_for(Some("gh-agent gave no token for that repository".into()), &TokenError::Broker(BrokerError::NotSignedIn("lapsed".into())));
        assert!(signed_out.starts_with("gh-agent gave no token for that repository; ") && signed_out.contains(NEEDS_SIGN_IN_FOR_AGENT) && signed_out.ends_with("(lapsed)"), "{signed_out}");
    }
}
