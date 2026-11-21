// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/muxspect/browsers[?sample=SECS][&detail=1]`: which page each renderer of this
//! instance draws, how busy it is, and what the page is doing
//! (docs/specs/SPEC_RELEASE_BUILD_BROWSER_PAGE_DIAGNOSTIC_2026_11_13.md §3).
//!
//! The window host answers what only it knows (`renderer_map` and `page_probe`); this samples the
//! operating system's process table twice, `sample` seconds apart, and joins the three
//! (`backend::tower::browsers`, pure and unit-tested). It reads and stores nothing, and a page that
//! is a person's shows only its origin and three facts: no title, no path, no animation names. A
//! container agent reaches it like `muxspect layout` and sees nothing about a pane that adds to
//! what that already lists, except that origin and those facts.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use agentmux_procstats::ProcInfo;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::backend::tower::agentmux::{own_view_pid, parse_renderer_map, RendererEntry};
use crate::backend::tower::browsers::{browser_rows, instance_pids, others, parse_page_probe, PageProbe, Sample};

/// How long the sample lasts when the caller does not say: the unit the CPU runbook already uses.
const DEFAULT_SAMPLE_SECS: f64 = 3.0;
const MIN_SAMPLE_SECS: f64 = 1.0;
const MAX_SAMPLE_SECS: f64 = 30.0;
/// The host asks every page (1.5 s each, in parallel) before it answers.
const HOST_WAIT: Duration = Duration::from_secs(6);

/// The route's path: `muxspect --channel <name> browsers` forwards it, and this read outlasts the
/// forward layer's usual 5 s (it sleeps the whole sample and asks the host twice).
pub(super) const PATH: &str = "/api/v1/muxspect/browsers";

/// The most a request for `query` can take: its sample, four host calls (the renderer map and the
/// page facts, at each end of the sample, each bounded by [`HOST_WAIT`]), and a margin.
pub(super) fn longest_answer(query: &str) -> Duration {
    let asked = url::form_urlencoded::parse(query.as_bytes()).find(|(k, _)| k == "sample").and_then(|(_, v)| v.parse::<f64>().ok());
    Duration::from_secs_f64(sample_secs(asked)) + HOST_WAIT * 4 + Duration::from_secs(5)
}

#[derive(Debug, Deserialize)]
pub struct BrowsersQuery {
    /// Seconds between the two samples (1 to 30; default 3).
    pub sample: Option<f64>,
    /// `1` or `true`: also read each page's main-thread time counters.
    pub detail: Option<String>,
}

/// The sample length asked for, kept within bounds (a NaN or an absent value is the default).
fn sample_secs(asked: Option<f64>) -> f64 {
    asked.filter(|s| s.is_finite()).unwrap_or(DEFAULT_SAMPLE_SECS).clamp(MIN_SAMPLE_SECS, MAX_SAMPLE_SECS)
}

fn wants_detail(detail: Option<&str>) -> bool {
    matches!(detail, Some("1" | "true" | "yes"))
}

/// One call to the window host; `None` when it cannot be asked (an older host has no `page_probe`).
async fn ask_host(state: &AppState, route: &str, body: Value) -> Option<Value> {
    let host = state.host_ipc.lock().await.clone()?;
    match super::host_http::post_to_host(&state.http_client, &host, route, body, Some(HOST_WAIT)).await {
        Ok(answer) => Some(answer),
        Err(e) => {
            tracing::debug!(route, error = %e, "[muxspect] the window host could not answer");
            None
        }
    }
}

async fn processes() -> Result<Vec<ProcInfo>, String> {
    tokio::task::spawn_blocking(agentmux_procstats::snapshot)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The host's renderer map and page facts at one moment.
async fn read_host(state: &AppState, detail: bool) -> (Vec<RendererEntry>, Option<Vec<PageProbe>>) {
    let entries = ask_host(state, "renderer_map", json!({})).await.map(|a| parse_renderer_map(&a)).unwrap_or_default();
    let probes = ask_host(state, "page_probe", json!({ "detail": detail })).await.map(|a| parse_page_probe(&a));
    (entries, probes)
}

pub async fn handle_muxspect_browsers(State(state): State<AppState>, Query(q): Query<BrowsersQuery>) -> impl IntoResponse {
    let secs = sample_secs(q.sample);
    let detail = wants_detail(q.detail.as_deref());
    if state.host_ipc.lock().await.is_none() {
        let error = "the window host has not registered with this server, so there are no pages to ask about";
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": error }))).into_response();
    }
    // With `detail` the page's counters are read at both ends of the sample.
    let before_probes = if detail { read_host(&state, true).await.1.unwrap_or_default() } else { Vec::new() };
    let (before, started) = match processes().await {
        Ok(p) => (p, Instant::now()),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("process list: {e}") }))).into_response(),
    };
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
    let (entries, probes) = read_host(&state, detail).await;
    let after = match processes().await {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("process list: {e}") }))).into_response(),
    };
    let elapsed = started.elapsed().as_secs_f64();
    let sample = Sample { before: &before, after: &after, secs: elapsed };
    // A renderer reports the pid it sees for itself (the sandbox namespace's on Linux): find the OS's.
    let instance = instance_pids(&after, std::process::id());
    let view_to_host: std::collections::HashMap<u32, u32> = instance.iter().map(|p| (own_view_pid(*p), *p)).collect();
    let host_pid = |reported: u32| view_to_host.get(&reported).copied();
    let rows = browser_rows(&entries, probes.as_deref().unwrap_or_default(), &before_probes, &sample, &host_pid);
    let page_pids: HashSet<u32> = rows.iter().filter_map(|r| r.pid).collect();
    let rest = others(&instance, &page_pids, &sample);
    Json(json!({
        "sample_secs": elapsed,
        "detail": detail,
        "page_facts": probes.is_some(),
        "browsers": rows,
        "others": rest,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_length_is_kept_within_bounds() {
        assert_eq!(sample_secs(None), 3.0);
        assert_eq!(sample_secs(Some(0.0)), 1.0);
        assert_eq!(sample_secs(Some(600.0)), 30.0);
        assert_eq!(sample_secs(Some(f64::NAN)), 3.0);
        assert_eq!(sample_secs(Some(5.0)), 5.0);
    }

    #[test]
    fn a_forwarded_request_may_take_its_whole_sample_and_the_host_reads() {
        // The forward layer's default is 5 s; a 30 s sample must not be cut at that.
        assert!(longest_answer("sample=30") >= Duration::from_secs(30 + 24));
        assert!(longest_answer("") > Duration::from_secs(5 + 3));
        // A bad or huge value is bounded like the handler bounds it.
        assert_eq!(longest_answer("sample=nope"), longest_answer(""));
        assert_eq!(longest_answer("sample=9999"), longest_answer("sample=30"));
    }

    #[test]
    fn detail_is_asked_for_by_name() {
        assert!(wants_detail(Some("1")) && wants_detail(Some("true")));
        assert!(!wants_detail(None) && !wants_detail(Some("0")) && !wants_detail(Some("")));
    }
}
