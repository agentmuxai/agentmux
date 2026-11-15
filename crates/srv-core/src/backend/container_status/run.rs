// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The watcher: follows Docker's event stream, rebuilds from a list now and then because a stream can drop,
//! and pings the runtime so a runtime that stops answering is said. Docker is behind [`Source`], so a test
//! drives it with a fake.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::StreamExt as _;

use super::{DockerEvent, Hub, Listed};

/// What a look at the container runtime found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub available: bool,
    pub runtime: Option<String>,
    pub detail: Option<String>,
}

/// Where the containers are read from.
pub trait Source: Send + Sync + 'static {
    /// Looks for the runtime and pings it.
    fn ping(&self) -> impl Future<Output = Probe> + Send;
    /// Every agent container Docker has, running or not.
    fn list(&self) -> impl Future<Output = Result<Vec<Listed>, String>> + Send;
    /// The events of agent containers, from now.
    fn events(&self) -> impl Future<Output = Result<BoxStream<'static, Result<DockerEvent, String>>, String>> + Send;
}

/// How often each thing is done.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// A list that rebuilds what the events built, in case the stream missed one.
    pub reconcile: Duration,
    /// A ping of the runtime.
    pub ping: Duration,
    /// Waiting before the stream or the runtime is tried again.
    pub retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { reconcile: Duration::from_secs(60), ping: Duration::from_secs(30), retry: Duration::from_secs(5) }
    }
}

/// Why a session ended.
#[derive(Debug, PartialEq, Eq)]
enum End {
    /// The runtime does not answer.
    Unavailable,
    /// The stream or a list failed.
    Broken,
}

/// One connection: look at the runtime, subscribe, list, then follow until something breaks. Subscribing
/// comes before the list, so a change between the two is not missed.
async fn session<S: Source>(source: &S, hub: &Hub, timing: &Timing) -> End {
    let probe = source.ping().await;
    let available = probe.available;
    hub.on_runtime(probe);
    if !available {
        return End::Unavailable;
    }
    let mut events = match source.events().await {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(%error, "container events: could not subscribe");
            return End::Broken;
        }
    };
    match source.list().await {
        Ok(listed) => hub.on_listed(listed),
        Err(error) => {
            tracing::warn!(%error, "container list failed");
            return End::Broken;
        }
    }
    let start = tokio::time::Instant::now();
    let mut reconcile = tokio::time::interval_at(start + timing.reconcile, timing.reconcile);
    let mut ping = tokio::time::interval_at(start + timing.ping, timing.ping);
    loop {
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => hub.on_event(event),
                Some(Err(error)) => {
                    tracing::warn!(%error, "container events: the stream failed");
                    return End::Broken;
                }
                None => return End::Broken,
            },
            _ = reconcile.tick() => match source.list().await {
                Ok(listed) => hub.on_listed(listed),
                Err(error) => {
                    tracing::warn!(%error, "container list failed");
                    return End::Broken;
                }
            },
            _ = ping.tick() => {
                let probe = source.ping().await;
                let available = probe.available;
                hub.on_runtime(probe);
                if !available {
                    return End::Unavailable;
                }
            }
        }
    }
}

/// Watches for the rest of the process. A machine with no container runtime and no container is looked at
/// once, then left alone until something wakes the hub (a container agent starting, or a reader of the
/// route that finds a runtime).
pub async fn run<S: Source>(source: S, hub: Arc<Hub>, timing: Timing) {
    loop {
        match session(&source, &hub, &timing).await {
            End::Unavailable if hub.is_dormant() => hub.dormant_until_woken().await,
            End::Unavailable => tokio::time::sleep(timing.ping).await,
            End::Broken => tokio::time::sleep(timing.retry).await,
        }
    }
}

#[cfg(test)]
mod tests;
