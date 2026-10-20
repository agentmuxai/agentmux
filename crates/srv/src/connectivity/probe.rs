// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The active probe: while a service is believed down, check it on a capped
//! backoff (`tracker::PROBE_FIRST_MS` doubling to `tracker::PROBE_MAX_MS`),
//! so its recovery shows even when nothing else is calling. An online
//! service is never probed.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::{Outcome, Service, Shared};

/// Milliseconds since the epoch; injected so tests run on tokio's paused clock.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Checks one service; resolves to what it found.
pub trait Prober: Send + Sync + 'static {
    fn probe(&self, service: Service) -> Pin<Box<dyn Future<Output = Outcome> + Send>>;
}

/// The longest one probe may take, whatever the prober does: a DNS lookup
/// that hangs mustn't leave the service unprobed.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the probes as they fall due, one at a time per service, until srv
/// exits. Holds no lock across an await; a probe's result carries the ticket
/// it was started with, so one that outlives its schedule changes nothing.
pub(super) async fn drive(shared: Arc<Shared>, prober: Arc<dyn Prober>, clock: Clock) {
    loop {
        let due = shared.tracker().take_due_probes(clock());
        for (service, ticket) in due {
            let (shared, prober, clock) = (shared.clone(), prober.clone(), clock.clone());
            tokio::spawn(async move {
                let outcome = tokio::time::timeout(PROBE_TIMEOUT, prober.probe(service.clone()))
                    .await
                    .unwrap_or_else(|_| Outcome::Unreachable("probe timed out".to_string()));
                tracing::debug!(service = %service.id(), ?outcome, "connectivity: probed");
                let changed = shared.tracker().probe_finished(&service, ticket, outcome, clock());
                shared.publish(changed);
                // The next probe is scheduled now.
                shared.wake.notify_one();
            });
        }
        let next = shared.tracker().next_probe_due();
        match next {
            Some(due_ms) => {
                let wait = Duration::from_millis(u64::try_from(due_ms - clock()).unwrap_or(0));
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = shared.wake.notified() => {}
                }
            }
            None => shared.wake.notified().await,
        }
    }
}

/// Whether srv's requests go through a proxy: reqwest, as srv builds it,
/// reads these variables. A direct TCP connect would then test a path srv
/// doesn't use, so the probe sends only the HEAD, through reqwest.
fn proxy_configured() -> bool {
    ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .any(|var| std::env::var(var).is_ok_and(|v| !v.trim().is_empty()))
}

/// The real probe: a TCP connect to the service's host, then a HEAD to its
/// base URL; any HTTP response is an answer.
pub(super) struct NetProber {
    http: reqwest::Client,
}

impl NetProber {
    pub(super) fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(crate::muxbus::relay::USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(HEAD_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { http }
    }
}

async fn probe_url(http: reqwest::Client, url: String) -> Outcome {
    let parsed = match reqwest::Url::parse(&url) {
        Ok(u) => u,
        Err(e) => return Outcome::Unreachable(format!("bad address {url}: {e}")),
    };
    if !proxy_configured() {
        let (Some(host), Some(port)) = (parsed.host_str(), parsed.port_or_known_default()) else {
            return Outcome::Unreachable(format!("bad address {url}"));
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect((host, port))).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Outcome::Unreachable(e.to_string()),
            Err(_) => return Outcome::Unreachable("connect timed out".to_string()),
        }
    }
    super::http_outcome(&http.head(parsed).send().await)
        .unwrap_or_else(|| Outcome::Unreachable("the probe request couldn't be built".to_string()))
}

impl Prober for NetProber {
    fn probe(&self, service: Service) -> Pin<Box<dyn Future<Output = Outcome> + Send>> {
        Box::pin(probe_url(self.http.clone(), service.probe_url()))
    }
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
