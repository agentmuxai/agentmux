// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/github/pr-merge`: a container agent's `gh pr merge`, done by srv.
//!
//! A container's GitHub token cannot write contents, and a merge writes contents. So the container's `gh pr merge`
//! (`agentmux-bashwrap github-pr-merge`) asks here instead: srv runs the host's `gh-agent pr merge` with arguments it
//! builds itself from a repository, a number and three options, never from the container's text. The token stays on
//! the host. There is no way to ask for `--admin`: GitHub's own rules (a review, the checks, a merge queue) decide, and
//! the answer says what GitHub said.
//!
//! Who is asking is the agent its `X-Agent-Token` was minted for.

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::identity::gh_agent_token::{self, MergeMethod, MergeRequest};
use crate::server::caller::Caller;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrMergeRequest {
    owner: String,
    repo: String,
    number: u64,
    /// `squash`, `merge` or `rebase`; absent leaves it to the repository (a merge queue).
    #[serde(default)]
    method: Option<MergeMethod>,
    #[serde(default)]
    auto: bool,
    #[serde(default)]
    delete_branch: bool,
}

impl PrMergeRequest {
    fn into_merge(self) -> MergeRequest {
        MergeRequest { owner: self.owner, repo: self.repo, number: self.number, method: self.method, auto: self.auto, delete_branch: self.delete_branch }
    }
}

/// What a machine without `gh-agent` tells a container that asks for a merge.
const NO_GH_AGENT: &str = "merging from a container needs gh-agent on this machine (npm i -g @a5af/gh-agent); ask your leader, or have the pull request merged from the host";

/// `POST /api/v1/github/pr-merge`
pub(crate) async fn handle_pr_merge(State(state): State<AppState>, caller: Option<Extension<Caller>>, Json(req): Json<PrMergeRequest>) -> Response {
    let refuse = |status: StatusCode, message: String| (status, Json(json!({ "error": message }))).into_response();
    let Some(uid) = caller.as_ref().and_then(|Extension(c)| c.uid()).map(str::to_string) else {
        return refuse(StatusCode::UNAUTHORIZED, "no agent token: a merge is for an agent's own command".to_string());
    };
    let mstore = state.mstore.clone();
    let slug = match tokio::task::spawn_blocking(move || mstore.agent_def_get(&uid)).await {
        Ok(Ok(Some(def))) => def.slug,
        Ok(Ok(None)) => return refuse(StatusCode::UNAUTHORIZED, "the agent token names no agent on this machine".to_string()),
        _ => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "couldn't read the agent".to_string()),
    };
    if gh_agent_token::installed().is_none() {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, NO_GH_AGENT.to_string());
    }
    let merge = req.into_merge();
    if let Err(why) = gh_agent_token::merge_request_ok(&merge) {
        return refuse(StatusCode::BAD_REQUEST, why);
    }
    tracing::info!(target: "identity", agent = %slug, owner = %merge.owner, repo = %merge.repo, number = merge.number, "identity.github: a container agent's merge, run by the host's gh-agent");
    match gh_agent_token::merge_pr(&slug, &merge).await {
        Ok(said) => Json(json!({ "ok": true, "message": said })).into_response(),
        Err(why) => refuse(StatusCode::UNPROCESSABLE_ENTITY, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_names_a_pull_request_and_nothing_a_shell_could_read() {
        let r: PrMergeRequest = serde_json::from_str(r#"{"owner":"o","repo":"r","number":7,"method":"squash","auto":true,"delete_branch":true}"#).unwrap();
        let m = r.into_merge();
        assert_eq!((m.number, m.method, m.auto, m.delete_branch), (7, Some(MergeMethod::Squash), true, true));
        let bare: PrMergeRequest = serde_json::from_str(r#"{"owner":"o","repo":"r","number":7}"#).unwrap();
        let m = bare.into_merge();
        assert_eq!((m.method, m.auto, m.delete_branch), (None, false, false));
    }

    use crate::identity::gh_agent_token::fake;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn merge_through_the_route(stdout: &str, code: i32, body: &str) -> (StatusCode, serde_json::Value, Vec<String>) {
        let _guard = fake::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let old = fake::install(dir.path(), stdout, code);
        let state = crate::server::tests::test_state();
        let token = crate::server::tests::token_for(&state, "aria");
        let app = crate::server::build_router(state);
        let req = Request::builder()
            .uri("/api/v1/github/pr-merge")
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("X-Agent-Token", token)
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        std::env::set_var("PATH", old);
        (status, serde_json::from_slice(&bytes).unwrap_or_default(), fake::calls(dir.path()))
    }

    #[tokio::test]
    async fn the_host_runs_the_merge_with_arguments_srv_built() {
        let (status, answer, calls) =
            merge_through_the_route("! Pull request #7 will be added to the merge queue", 0, r#"{"owner":"octo","repo":"hello","number":7,"method":"squash","delete_branch":true}"#).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["ok"], true);
        assert!(answer["message"].as_str().unwrap().contains("merge queue"), "{answer}");
        assert_eq!(calls, ["pr merge 7 --repo octo/hello --squash --delete-branch"]);
    }

    #[tokio::test]
    async fn what_github_refused_is_said_and_nothing_else_is_run() {
        let (status, answer, calls) = merge_through_the_route("Pull request is not mergeable: required reviews are missing", 1, r#"{"owner":"octo","repo":"hello","number":7}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(answer["error"].as_str().unwrap().contains("not mergeable"), "{answer}");
        assert_eq!(calls, ["pr merge 7 --repo octo/hello"]);
    }

    #[tokio::test]
    async fn a_request_with_an_option_srv_does_not_have_never_reaches_the_host() {
        let (status, _, calls) = merge_through_the_route("ok", 0, r#"{"owner":"octo","repo":"hello","number":7,"admin":true}"#).await;
        assert!(status.is_client_error(), "{status}");
        assert!(calls.is_empty(), "{calls:?}");
        let (status, _, calls) = merge_through_the_route("ok", 0, r#"{"owner":"--admin","repo":"hello","number":7}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(calls.is_empty(), "{calls:?}");
    }

    #[test]
    fn an_option_that_is_not_one_of_the_few_is_refused_not_ignored() {
        for body in [
            r#"{"owner":"o","repo":"r","number":7,"admin":true}"#,
            r#"{"owner":"o","repo":"r","number":7,"method":"fast-forward"}"#,
            r#"{"owner":"o","repo":"r","number":"7"}"#,
            r#"{"owner":"o","repo":"r"}"#,
        ] {
            assert!(serde_json::from_str::<PrMergeRequest>(body).is_err(), "{body}");
        }
    }
}
