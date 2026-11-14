// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! This machine's agent containers as the frontend sees them: a name, whose they are, a state and an image,
//! pushed as `container-status` when one changes, with the container runtime's own state as
//! `container-runtime`, and read as a whole with `GET /api/v1/containers`
//! (`docs/specs/SPEC_CONTAINER_STATUS_IN_THE_FRONTEND_2026_11_14.md`).
//!
//! Names and states only. There is no field for a log, an environment, a mount or a statistic, so none can
//! leak through it. Docker is the source of truth: [`run`] follows its event stream, rebuilds from a list now
//! and then, and this module keeps what it was told ([`Registry`]) and says what changed ([`Hub`]).
//!
//! Which containers: those named `agentmux-<agent id>`, as `container_name_for_slug` names them. One whose id
//! is no agent's is an orphan (`agent_id` null), because that is the one a person wants to see. Another
//! container is never listed: a name must carry the label new containers get ([`AGENT_LABEL`]), or look like
//! an agent id (a UUID), or be the id of a definition.

pub(crate) mod docker_event;
mod run;
mod source;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use serde::Serialize;

pub use docker_event::{Action, DockerEvent, Listed};
pub use run::{run, Probe, Source};
pub use source::DockerSource;

/// The name every agent container starts with (`container_name_for_slug`).
pub const NAME_PREFIX: &str = "agentmux-";
/// The label a new agent container carries: its agent's id.
pub const AGENT_LABEL: &str = "ai.agentmux.agent";

/// Where a container is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    /// Being made or started: srv is bringing it up.
    Starting,
    Running,
    /// Made and not running.
    Stopped,
    /// Removed. Only ever in an event, once.
    Gone,
}

/// One agent container. `agent_id` and `agent_name` are null for an orphan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ContainerStatus {
    pub name: String,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub state: ContainerState,
    /// The image reference as the container was made from it: name and tag, no credentials.
    pub image: String,
    /// When it entered `state`, in milliseconds since 1970.
    #[ts(type = "number")]
    pub since_ms: u64,
}

/// The container runtime's own state, the fields of `containerruntimeavailable` with the time it was seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ContainerRuntimeStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    #[ts(type = "number")]
    pub at_ms: u64,
}

impl ContainerRuntimeStatus {
    /// Before anything has been looked at: there is no runtime.
    pub fn absent() -> Self {
        Self { available: false, runtime: None, detail: None, at_ms: agentmux_common::time::now_ms() as u64 }
    }
}

/// The answer of `GET /api/v1/containers`: what is there now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ContainersList {
    pub runtime: ContainerRuntimeStatus,
    pub containers: Vec<ContainerStatus>,
}

/// What [`Hub`] says to the outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Published {
    Container(ContainerStatus),
    Runtime(ContainerRuntimeStatus),
}

/// What is known of one container, before it is named for an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Raw {
    state: ContainerState,
    image: String,
    since_ms: u64,
    labelled: bool,
}

/// What Docker (and srv's own start-up) has said, and what was last published from it.
#[derive(Debug, Default)]
pub struct Registry {
    raw: BTreeMap<String, Raw>,
    sent: BTreeMap<String, ContainerStatus>,
    runtime: Option<ContainerRuntimeStatus>,
    ever_available: bool,
}

/// The agent id of a container name, when it is an agent container's.
fn agent_id_of(name: &str) -> Option<&str> {
    name.strip_prefix(NAME_PREFIX).filter(|id| !id.is_empty())
}

/// Whether `id` is shaped like an agent's id (a UUID), as every definition's is.
fn looks_like_agent_id(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl Registry {
    /// One Docker event: the container moves to the state the action names. The same state again changes
    /// nothing, so `since_ms` stays when it was entered.
    pub fn apply(&mut self, event: DockerEvent, now_ms: u64) {
        if agent_id_of(&event.name).is_none() {
            return;
        }
        let state = match event.action {
            Action::Create => ContainerState::Starting,
            Action::Start => ContainerState::Running,
            Action::Stop | Action::Die => ContainerState::Stopped,
            Action::Destroy => {
                self.raw.remove(&event.name);
                return;
            }
        };
        self.set(&event.name, state, &event.image, event.labelled, now_ms);
    }

    /// srv is bringing `name` up: shown as starting before Docker says anything. A container that is
    /// already running stays running.
    pub fn starting(&mut self, name: &str, image: &str, now_ms: u64) {
        if agent_id_of(name).is_none() || self.raw.get(name).is_some_and(|r| r.state == ContainerState::Running) {
            return;
        }
        self.set(name, ContainerState::Starting, image, true, now_ms);
    }

    /// The whole list from Docker: what it names replaces what was known, and a container it does not
    /// name is gone.
    pub fn replace(&mut self, listed: Vec<Listed>, now_ms: u64) {
        let names: std::collections::HashSet<&str> = listed.iter().map(|l| l.name.as_str()).collect();
        self.raw.retain(|name, _| names.contains(name.as_str()));
        for l in &listed {
            if agent_id_of(&l.name).is_some() {
                let state = if l.running { ContainerState::Running } else { ContainerState::Stopped };
                self.set(&l.name, state, &l.image, l.labelled, now_ms);
            }
        }
    }

    fn set(&mut self, name: &str, state: ContainerState, image: &str, labelled: bool, now_ms: u64) {
        let known = self.raw.get(name);
        // A start-up that has no image yet (a restart of a stopped container) keeps the one it had.
        let image = if image.is_empty() { known.map(|r| r.image.clone()).unwrap_or_default() } else { image.to_string() };
        let since_ms = known.filter(|r| r.state == state).map_or(now_ms, |r| r.since_ms);
        let labelled = labelled || known.is_some_and(|r| r.labelled);
        self.raw.insert(name.to_string(), Raw { state, image, since_ms, labelled });
    }

    /// The runtime as just looked at. `Some` when it differs from the last look in anything but the time.
    pub fn set_runtime(&mut self, probe: Probe, now_ms: u64) -> Option<ContainerRuntimeStatus> {
        self.ever_available |= probe.available;
        let now = ContainerRuntimeStatus { available: probe.available, runtime: probe.runtime, detail: probe.detail, at_ms: now_ms };
        let same = self.runtime.as_ref().is_some_and(|r| (r.available, &r.runtime, &r.detail) == (now.available, &now.runtime, &now.detail));
        if same {
            return None;
        }
        self.runtime = Some(now.clone());
        Some(now)
    }

    /// What differs from what was last published, given the agents' names (by id): a changed container, and
    /// a removed one once, as `gone`. Drops what is gone.
    pub fn changes(&mut self, agents: &HashMap<String, String>, now_ms: u64) -> Vec<ContainerStatus> {
        let mut now: BTreeMap<String, ContainerStatus> = BTreeMap::new();
        for (name, raw) in &self.raw {
            let Some(id) = agent_id_of(name) else { continue };
            if !(raw.labelled || looks_like_agent_id(id) || agents.contains_key(id)) {
                continue;
            }
            let agent_name = agents.get(id);
            now.insert(
                name.clone(),
                ContainerStatus {
                    name: name.clone(),
                    agent_id: agent_name.map(|_| id.to_string()),
                    agent_name: agent_name.cloned(),
                    state: raw.state,
                    image: raw.image.clone(),
                    since_ms: raw.since_ms,
                },
            );
        }
        let mut changed: Vec<ContainerStatus> = now.values().filter(|s| self.sent.get(&s.name) != Some(s)).cloned().collect();
        for (name, was) in &self.sent {
            if !now.contains_key(name) {
                changed.push(ContainerStatus { state: ContainerState::Gone, since_ms: now_ms, ..was.clone() });
            }
        }
        self.sent = now;
        changed
    }

    /// The containers as last published, by name.
    pub fn containers(&self) -> Vec<ContainerStatus> {
        self.sent.values().cloned().collect()
    }

    /// The runtime as last seen; "not available" before the first look.
    pub fn runtime(&self, now_ms: u64) -> ContainerRuntimeStatus {
        self.runtime.clone().unwrap_or(ContainerRuntimeStatus { available: false, runtime: None, detail: None, at_ms: now_ms })
    }

    /// Whether the runtime is absent, was never seen and has no container to report: nothing then needs
    /// looking at until something wakes [`Hub::dormant_until_woken`].
    pub fn is_dormant(&self) -> bool {
        !self.ever_available && self.sent.is_empty() && self.raw.is_empty() && self.runtime.as_ref().is_some_and(|r| !r.available)
    }
}

type AgentNames = Box<dyn Fn() -> HashMap<String, String> + Send + Sync>;
type Sink = Box<dyn Fn(Published) + Send + Sync>;

/// The registry with the way out: every change it makes is published, once.
pub struct Hub {
    registry: std::sync::Mutex<Registry>,
    agents: AgentNames,
    sink: Sink,
    wake: tokio::sync::Notify,
}

impl Hub {
    /// `agents` is each agent's name by id, read when a change is worked out; `sink` gets what changed.
    pub fn new(agents: impl Fn() -> HashMap<String, String> + Send + Sync + 'static, sink: impl Fn(Published) + Send + Sync + 'static) -> Self {
        Self { registry: Default::default(), agents: Box::new(agents), sink: Box::new(sink), wake: tokio::sync::Notify::new() }
    }

    fn now_ms() -> u64 {
        agentmux_common::time::now_ms() as u64
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Publishes what `change` made different.
    fn publish_after(&self, change: impl FnOnce(&mut Registry, u64)) {
        let agents = (self.agents)();
        let now = Self::now_ms();
        let changed = {
            let mut registry = self.lock();
            change(&mut registry, now);
            registry.changes(&agents, now)
        };
        for status in changed {
            (self.sink)(Published::Container(status));
        }
    }

    pub fn on_event(&self, event: DockerEvent) {
        self.publish_after(|r, now| r.apply(event, now));
    }

    pub fn on_listed(&self, listed: Vec<Listed>) {
        self.publish_after(|r, now| r.replace(listed, now));
    }

    /// srv is starting `name` from `image`; wakes the watcher if it was asleep.
    pub fn on_starting(&self, name: &str, image: &str) {
        self.publish_after(|r, now| r.starting(name, image, now));
        self.wake.notify_one();
    }

    pub fn on_runtime(&self, probe: Probe) {
        let changed = self.lock().set_runtime(probe, Self::now_ms());
        if let Some(status) = changed {
            (self.sink)(Published::Runtime(status));
        }
    }

    /// What is there now, for the route.
    pub fn list(&self) -> ContainersList {
        let registry = self.lock();
        ContainersList { runtime: registry.runtime(Self::now_ms()), containers: registry.containers() }
    }

    pub fn is_dormant(&self) -> bool {
        self.lock().is_dormant()
    }

    /// Wakes the watcher: a person asked, or a container is being started.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Waits to be woken. A wake that came first is not lost.
    pub async fn dormant_until_woken(&self) {
        self.wake.notified().await;
    }
}

static HUB: OnceLock<Arc<Hub>> = OnceLock::new();

/// Start publishing on `broker` for the rest of the process, and return the hub for [`run`]. The first call
/// wins.
pub fn install(broker: Arc<super::mps::Broker>, agents: impl Fn() -> HashMap<String, String> + Send + Sync + 'static) -> Arc<Hub> {
    use super::mps_events::{ContainerRuntimeChanged, ContainerStatusChanged};
    let hub = Arc::new(Hub::new(agents, move |published| match published {
        Published::Container(status) => broker.publish_event::<ContainerStatusChanged>(&status, vec![], 0),
        Published::Runtime(status) => broker.publish_event::<ContainerRuntimeChanged>(&status, vec![], 0),
    }));
    Arc::clone(HUB.get_or_init(|| hub))
}

/// The hub, once installed.
pub fn hub() -> Option<&'static Arc<Hub>> {
    HUB.get()
}

/// srv is about to bring `container_name` up from `image` (empty when it exists already).
pub fn note_starting(container_name: &str, image: &str) {
    if let Some(hub) = hub() {
        hub.on_starting(container_name, image);
    }
}

#[cfg(test)]
mod tests;
