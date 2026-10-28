// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether each service AgentMux depends on can be reached
//! (docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.1).
//!
//! **Inputs.** Code that already makes requests reports what happened
//! ([`report`], [`report_http`], [`report_ws_connect`],
//! [`observe_claude_frame`]): any HTTP response means the service is
//! reachable, whatever its status; a transport failure (DNS, connect, reset,
//! TLS, timeout) means it isn't. While a service is believed down, and only
//! then, [`probe`] checks it on its own with capped backoff, so recovery
//! shows even when nothing else is calling.
//!
//! **States** ([`tracker`]): `offline` after [`tracker::OFFLINE_AFTER`]
//! transport failures in a row or a failed probe; `degraded` while some
//! fail and some answer; `online` on an answer. The internet counts as
//! offline only when two services are and none answers.
//!
//! **Outputs.** The `connectivity:changed` event on every change and
//! `connectivity.status` for a window that opens later; both carry a rising
//! `revision`, since they can arrive out of order.

mod endpoint;
mod netchange;
mod probe;
mod tracker;

use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::Notify;

use crate::backend::eventbus::EventBus;
use tracker::Tracker;

/// The WebSocket event carrying a changed [`ConnectivityStatus`].
pub const EVENT_CONNECTIVITY_CHANGED: &str = "connectivity:changed";

/// Whether a service can be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum Reachability {
    Online,
    /// Some requests fail at the transport level, some are answered.
    Degraded,
    /// Every recent request failed before any answer came back.
    Offline,
    /// Not checked yet: a model API srv tracks, between startup and its
    /// first probe or request. Not offline, and not known to answer.
    Unknown,
}

/// One service's reachability.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ServiceStatus {
    /// `internet`, `muxbus`, or `model_api:<vendor>`.
    pub service: String,
    /// What the status bar calls it ("Anthropic API").
    pub label: String,
    pub state: Reachability,
    /// When `state` began, in ms since the epoch.
    #[ts(type = "number")]
    pub since_ms: i64,
    /// The last transport error, while not online.
    pub last_error: Option<String>,
}

/// `connectivity:changed`'s payload and `connectivity.status`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ConnectivityStatus {
    /// Rises with every change; only an order, not a time.
    #[ts(type = "number")]
    pub revision: u64,
    /// `internet` first, then every service that has reported, online or not.
    pub services: Vec<ServiceStatus>,
}

/// A model vendor's API: where to probe it, and its name in the status bar.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelApi {
    /// The vendor id, as `ProviderConfig::supported_vendors` names it.
    pub vendor: &'static str,
    pub label: &'static str,
    pub default_base_url: &'static str,
    /// The variable that points the provider's CLI elsewhere, if any.
    pub base_url_env: Option<&'static str>,
}

/// The API Claude Code calls. Only the persistent controller, which only
/// Claude uses, reports today; another provider adds its own.
pub static ANTHROPIC: ModelApi = ModelApi {
    vendor: "anthropic",
    label: "Anthropic API",
    default_base_url: "https://api.anthropic.com",
    base_url_env: Some("ANTHROPIC_BASE_URL"),
};

/// The API Codex calls.
pub static OPENAI: ModelApi = ModelApi {
    vendor: "openai",
    label: "OpenAI API",
    default_base_url: "https://api.openai.com",
    base_url_env: Some("OPENAI_BASE_URL"),
};

/// The API Gemini CLI calls.
pub static GOOGLE: ModelApi = ModelApi {
    vendor: "google",
    label: "Google AI API",
    default_base_url: "https://generativelanguage.googleapis.com",
    base_url_env: Some("GOOGLE_GEMINI_BASE_URL"),
};

/// The model APIs whose state srv tracks from startup: each is listed as
/// [`Reachability::Unknown`] and probed at once, so a window can tell "not
/// checked yet" from "answers" before any agent has made a request.
pub static TRACKED_MODEL_APIS: [&ModelApi; 1] = [&ANTHROPIC];

/// A service AgentMux depends on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Service {
    ModelApi(&'static ModelApi),
    /// A model API at an address other than the one srv itself would call (`ModelApi`'s
    /// [`Service::probe_url`]): a pane whose CLI is pointed at a proxy or a gateway. It is its own
    /// service, so its state, and the probe of it, are about the host that pane really uses. The
    /// address is [`endpoint::normalise`]d.
    ModelApiAt(&'static ModelApi, String),
    /// The relay and its sign-in server.
    Muxbus,
}

impl Service {
    const INTERNET_ID: &'static str = "internet";
    const INTERNET_LABEL: &'static str = "Internet";

    pub fn id(&self) -> String {
        match self {
            Service::ModelApi(api) => format!("model_api:{}", api.vendor),
            Service::ModelApiAt(api, url) => format!("model_api:{}@{}", api.vendor, endpoint::host_of(url)),
            Service::Muxbus => "muxbus".to_string(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Service::ModelApi(api) => api.label.to_string(),
            Service::ModelApiAt(api, url) => format!("{} ({})", api.label, endpoint::host_of(url)),
            Service::Muxbus => "MuxBus".to_string(),
        }
    }

    /// Where a probe goes: srv's own environment for a `ModelApi` (a pane that sets its own
    /// address is a [`Service::ModelApiAt`], probed at that address).
    pub fn probe_url(&self) -> String {
        match self {
            Service::ModelApiAt(_, url) => url.clone(),
            Service::ModelApi(api) => api
                .base_url_env
                .and_then(|var| std::env::var(var).ok())
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| api.default_base_url.to_string()),
            Service::Muxbus => crate::muxbus::relay::rest_base_url(),
        }
    }
}

/// The service a pane's calls to `api` go to, given the address its CLI was started with
/// (`None` or empty: the CLI's default). The same address srv itself would call is just `api`.
pub fn service_for(api: &'static ModelApi, configured: Option<&str>) -> Service {
    let Some(url) = configured.map(str::trim).filter(|u| !u.is_empty()) else {
        return Service::ModelApi(api);
    };
    let url = endpoint::normalise(url);
    if url == endpoint::normalise(&Service::ModelApi(api).probe_url()) {
        Service::ModelApi(api)
    } else {
        Service::ModelApiAt(api, url)
    }
}

/// [`service_for`] with the address read from the environment a pane's CLI was started with.
pub fn service_for_env(api: &'static ModelApi, env: &std::collections::HashMap<String, String>) -> Service {
    service_for(api, api.base_url_env.and_then(|var| env.get(var)).map(String::as_str))
}

/// The model API a provider's CLI calls, from its first vendor, if srv tracks it.
pub fn model_api_of(provider: &str) -> Option<&'static ModelApi> {
    let vendor = *crate::backend::providers::get_provider(provider)?.supported_vendors.first()?;
    [&ANTHROPIC, &OPENAI, &GOOGLE].into_iter().find(|api| api.vendor == vendor)
}

/// What a pane keeps in its block meta (`agent:api_endpoint`) when its model API is not at srv's own
/// address: its service id, so the pane reads its own state, and the address.
pub fn endpoint_meta(service: &Service) -> Option<serde_json::Value> {
    match service {
        Service::ModelApiAt(_, url) => Some(serde_json::json!({ "service": service.id(), "url": url })),
        _ => None,
    }
}

/// What one request, or probe, found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Some HTTP response came back.
    Answered,
    /// No response: DNS, connect, reset, TLS or timeout (the error).
    Unreachable(String),
}

/// The tracker, the probe driver's wake-up, and where changes go.
pub(crate) struct Shared {
    tracker: Mutex<Tracker>,
    wake: Notify,
    sink: OnceLock<Arc<EventBus>>,
}

impl Shared {
    fn new(now_ms: i64) -> Self {
        let first_revision = u64::try_from(now_ms).unwrap_or(0).saturating_mul(1000);
        Self { tracker: Mutex::new(Tracker::new(first_revision, now_ms)), wake: Notify::new(), sink: OnceLock::new() }
    }

    fn tracker(&self) -> std::sync::MutexGuard<'_, Tracker> {
        self.tracker.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lists every tracked model API as not checked yet, due for a probe.
    fn seed_at(&self, now_ms: i64) {
        let mut changed = None;
        for api in TRACKED_MODEL_APIS {
            changed = self.tracker().seed(Service::ModelApi(api), now_ms).or(changed);
        }
        self.publish(changed);
    }

    /// Lists `service` as not checked yet, due for a probe, unless it has reported.
    fn track_at(&self, service: Service, now_ms: i64) {
        let changed = self.tracker().seed(service, now_ms);
        self.publish(changed);
    }

    fn report_at(&self, service: Service, outcome: Outcome, now_ms: i64) {
        let changed = self.tracker().report(service, outcome, now_ms);
        self.publish(changed);
    }

    /// The OS reported a network change: every service is probed again, now.
    fn network_changed_at(&self, now_ms: i64) {
        if self.tracker().network_changed(now_ms) {
            tracing::info!("connectivity: the network changed, probing again");
            self.wake.notify_one();
        }
    }

    fn publish(&self, changed: Option<ConnectivityStatus>) {
        let Some(status) = changed else { return };
        let down: Vec<String> = status
            .services
            .iter()
            .filter(|s| s.state != Reachability::Online)
            .map(|s| format!("{}={:?}", s.service, s.state))
            .collect();
        tracing::info!(down = %down.join(","), "connectivity: changed");
        // A service that left `online` has a probe to schedule.
        self.wake.notify_one();
        if let Some(bus) = self.sink.get() {
            bus.broadcast_event(&EventBus::typed_ws_event::<crate::backend::mps_events::ConnectivityChanged>(&status, String::new()));
        }
    }
}

static SHARED: LazyLock<Arc<Shared>> = LazyLock::new(|| Arc::new(Shared::new(agentmux_common::time::now_ms())));

/// Where changes go, and the start of the probe driver. Call once at
/// startup; reports before it are kept, just not sent.
pub fn install(event_bus: Arc<EventBus>) {
    if SHARED.sink.set(event_bus).is_err() || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    SHARED.seed_at(agentmux_common::time::now_ms());
    let clock: probe::Clock = Arc::new(agentmux_common::time::now_ms);
    tokio::spawn(probe::drive(SHARED.clone(), Arc::new(probe::NetProber::new()), clock));
    netchange::start(SHARED.clone());
}

/// The current state, for `connectivity.status`.
pub fn status() -> ConnectivityStatus {
    SHARED.tracker().status()
}

/// Starts tracking `service` now: listed as not checked yet and probed at once, as the
/// [`TRACKED_MODEL_APIS`] are at startup. For a pane's own address, when its CLI starts.
pub fn track(service: Service) {
    SHARED.track_at(service, agentmux_common::time::now_ms());
}

/// What a request to `service` found.
pub fn report(service: Service, outcome: Outcome) {
    SHARED.report_at(service, outcome, agentmux_common::time::now_ms());
}

/// The outcome a reqwest `send()` result says, or `None` for a request that
/// was never sent (it couldn't be built).
pub fn http_outcome(result: &Result<reqwest::Response, reqwest::Error>) -> Option<Outcome> {
    match result {
        Ok(_) => Some(Outcome::Answered),
        Err(e) if e.is_builder() => None,
        Err(e) => Some(Outcome::Unreachable(e.to_string())),
    }
}

/// Reports a reqwest `send()` result.
pub fn report_http(service: Service, result: &Result<reqwest::Response, reqwest::Error>) {
    if let Some(outcome) = http_outcome(result) {
        report(service, outcome);
    }
}

/// The outcome of a WebSocket connect: an HTTP answer to the handshake, even
/// a refusal, means the server was reached.
pub fn ws_connect_outcome(error: Option<&tokio_tungstenite::tungstenite::Error>) -> Option<Outcome> {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        None => Some(Outcome::Answered),
        Some(e @ (Error::Io(_) | Error::Tls(_))) => Some(Outcome::Unreachable(e.to_string())),
        Some(Error::Url(_)) => None,
        Some(_) => Some(Outcome::Answered),
    }
}

/// Reports a WebSocket connect: `None` when it succeeded.
pub fn report_ws_connect(service: Service, error: Option<&tokio_tungstenite::tungstenite::Error>) {
    if let Some(outcome) = ws_connect_outcome(error) {
        report(service, outcome);
    }
}

/// What a Claude CLI stdout frame says about the API, if anything: an
/// `api_retry` with no HTTP status is a failed connection, one with a status
/// was answered, and so was a reply from a model; a turn's error `result`
/// the failure classifier calls `Network` is a failed connection. A
/// `<synthetic>` message is the CLI's own (its "API Error" text), not the API's.
pub fn claude_frame_outcome(frame: &serde_json::Value) -> Option<Outcome> {
    use serde_json::Value;
    match frame.get("type").and_then(Value::as_str)? {
        "system" if frame.get("subtype").and_then(Value::as_str) == Some("api_retry") => {
            if frame.get("error_status").and_then(Value::as_u64).is_some() {
                return Some(Outcome::Answered);
            }
            let error = frame.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).unwrap_or("connection failed");
            Some(Outcome::Unreachable(format!("Claude CLI retrying: {error}")))
        }
        "assistant" => {
            let model = frame.get("message")?.get("model")?.as_str()?;
            (model != "<synthetic>").then_some(Outcome::Answered)
        }
        "result" if frame.get("is_error").and_then(Value::as_bool) == Some(true) => {
            let failure = crate::agents::failure::classify(None, None, "", Some(frame));
            (failure.code == crate::agents::failure::FailureClass::Network).then_some(Outcome::Unreachable(failure.title))
        }
        _ => None,
    }
}

/// The API of a provider whose turns end only in an exit, with no frames to read: Codex and Gemini. They join the
/// status when their first turn reports, and are probed from then on while down. Claude reports through
/// [`observe_claude_frame`]; other providers are not tracked.
fn exit_reported_api(provider: &str) -> Option<&'static ModelApi> {
    let vendor = *crate::backend::providers::get_provider(provider)?.supported_vendors.first()?;
    [&OPENAI, &GOOGLE].into_iter().find(|api| api.vendor == vendor)
}

/// What a finished turn of `provider` says about its model API: a clean turn, or a failure the API itself returned
/// (limit, auth, overload), reached it; a network failure did not. Any other failure says nothing.
pub fn observe_provider_turn(provider: &str, failure: Option<&crate::agents::failure::AgentFailure>, configured: Option<&str>) {
    if let Some((api, outcome)) = provider_turn_outcome(provider, failure) {
        report(service_for(api, configured), outcome);
    }
}

fn provider_turn_outcome(
    provider: &str,
    failure: Option<&crate::agents::failure::AgentFailure>,
) -> Option<(&'static ModelApi, Outcome)> {
    use crate::agents::failure::FailureClass::{Auth, ContextExceeded, Network, Overloaded, RateLimited, UsageLimit};
    let api = exit_reported_api(provider)?;
    let outcome = match failure {
        None => Outcome::Answered,
        Some(f) => match f.code {
            Network => Outcome::Unreachable(f.title.clone()),
            RateLimited | Overloaded | UsageLimit | Auth | ContextExceeded => Outcome::Answered,
            _ => return None,
        },
    };
    Some((api, outcome))
}

/// Reads one Claude CLI stdout frame (`persistent::stdout_reader`).
pub fn observe_claude_frame(frame: &serde_json::Value, service: &Service) {
    if let Some(outcome) = claude_frame_outcome(frame) {
        report(service.clone(), outcome);
    }
}

#[cfg(test)]
mod tests;
