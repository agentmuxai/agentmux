// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! CPU, memory and process limits of a container agent's container.
//! `docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md`.
//!
//! Every container agent is limited, existing ones too. What it gets is the agent's own setting, else the app-wide
//! one, else a default computed from what the Docker daemon has (`docker info`: on Docker Desktop that is the VM, not
//! the machine). The limits are put on a running container with `docker update`: a change never recreates it.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bollard::container::UpdateContainerOptions;
use bollard::models::{ContainerInspectResponse, HostConfig};

use super::ContainerManager;
use crate::backend::storage::store::Store;

/// App-wide default memory limit, in MB, or `"auto"`.
pub const SETTING_MEMORY: &str = "agent:container_memory";
/// App-wide default CPU limit, in CPUs, or `"auto"`.
pub const SETTING_CPUS: &str = "agent:container_cpus";
/// An image label saying how much memory a container from it needs to be useful, in MB: a developer image that builds
/// Rust says so, and auto never gives it less (still bounded by what the daemon has).
pub const RECOMMENDED_MIN_MEMORY_LABEL: &str = "ai.agentmux.recommended-min-memory-mb";

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
/// A runaway fork loop is the third way a build freezes a machine; not a setting.
const PIDS_LIMIT: i64 = 8192;
/// How long what `docker info` said is kept: the daemon's size changes only when Docker is restarted.
const CAPACITY_KEPT: Duration = Duration::from_secs(300);

static CONFIG: OnceLock<Arc<crate::backend::wconfig::ConfigState>> = OnceLock::new();

/// Attach the live settings (bootstrap), so the app-wide limits can be read.
pub fn install_config(config: Arc<crate::backend::wconfig::ConfigState>) {
    let _ = CONFIG.set(config);
}

/// One limit as a person set it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Limit {
    /// Not set here: the next level decides (the agent's, then the app's, then the computed default).
    #[default]
    Auto,
    /// Explicitly no limit.
    Unlimited,
    /// MB of memory, or thousandths of a CPU.
    Fixed(u64),
}

impl Limit {
    /// A stored value: absent or `"auto"` is [`Limit::Auto`], `0` or `"none"` is [`Limit::Unlimited`], a positive number
    /// is `Fixed(number * unit)`.
    pub fn from_json(value: Option<&serde_json::Value>, unit: u64) -> Self {
        let number = match value {
            None | Some(serde_json::Value::Null) => return Self::Auto,
            Some(serde_json::Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                "" | "auto" => return Self::Auto,
                "none" | "unlimited" => return Self::Unlimited,
                other => other.parse::<f64>().ok(),
            },
            Some(v) => v.as_f64(),
        };
        match number {
            Some(n) if n > 0.0 && n.is_finite() => Self::Fixed(((n * unit as f64).round() as u64).max(1)),
            Some(n) if n == 0.0 => Self::Unlimited,
            _ => Self::Auto,
        }
    }
}

/// An agent's own limits (`db_agent_container_limits`). [`Limit::Auto`] means "the app-wide setting decides".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContainerLimits {
    pub memory: Limit,
    /// Thousandths of a CPU.
    pub cpus: Limit,
}

impl ContainerLimits {
    /// The limits stored for `agent_id`; none stored, or the store failing (logged), is all [`Limit::Auto`].
    pub fn of_agent(store: &Store, agent_id: &str) -> Self {
        match store.container_limits_get(agent_id) {
            Ok(Some((memory, cpus))) => Self::from_strings(&memory, &cpus),
            Ok(None) => Self::default(),
            Err(e) => {
                tracing::warn!(agent_id, error = %e, "container limits: cannot read the agent's limits; using the defaults");
                Self::default()
            }
        }
    }

    /// Stored text: empty is auto, `0` is no limit, a number is MB (memory) or CPUs.
    pub fn from_strings(memory: &str, cpus: &str) -> Self {
        let read = |text: &str, unit| Limit::from_json(Some(&serde_json::Value::String(text.to_string())), unit);
        Self { memory: read(memory, 1), cpus: read(cpus, 1000) }
    }

    /// The app-wide defaults.
    fn from_settings() -> Self {
        let Some(config) = CONFIG.get() else { return Self::default() };
        let config = config.get_full_config();
        let extra = &config.settings.extra;
        Self { memory: Limit::from_json(extra.get(SETTING_MEMORY), 1), cpus: Limit::from_json(extra.get(SETTING_CPUS), 1000) }
    }
}

/// A limit as stored: `""` for auto, `"0"` for no limit, else the number (MB, or CPUs). An explicit value that is
/// none of these is an error, where [`Limit::from_json`] is lenient with what it reads back.
pub fn normalize(value: &serde_json::Value, what: &str) -> Result<String, String> {
    let bad = || format!("{what} must be a positive number, 0 for no limit, or \"auto\"");
    match Limit::from_json(Some(value), 1000) {
        Limit::Fixed(_) => Ok(match value {
            serde_json::Value::String(s) => s.trim().to_string(),
            other => other.to_string(),
        }),
        Limit::Unlimited => Ok("0".to_string()),
        Limit::Auto => match value {
            serde_json::Value::Null => Ok(String::new()),
            serde_json::Value::String(s) if s.trim().is_empty() || s.trim().eq_ignore_ascii_case("auto") => Ok(String::new()),
            _ => Err(bad()),
        },
    }
}

/// What the daemon has to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonCapacity {
    pub mem_total: u64,
    pub ncpu: u32,
}

/// What to do about one limit of a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Not known (the daemon could not be asked): leave the container as it is.
    Leave,
    Unlimited,
    /// Bytes of memory, or billionths of a CPU.
    Set(i64),
}

/// The limits a container should carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub memory: Target,
    pub cpus: Target,
}

/// The default memory: half of the daemon's, never under 2 GiB (capped at what the daemon has). The spec's ceiling of
/// "total minus 2 GiB" is never under half from 4 GiB up, and under 4 GiB the floor rules, so it needs no code. An
/// image's recommended minimum raises the default, still bounded by the daemon's total.
pub fn auto_memory_bytes(total: u64, recommended_min: Option<u64>) -> u64 {
    let mut memory = (total / 2).max(2 * GIB);
    if let Some(min) = recommended_min {
        memory = memory.max(min);
    }
    memory.min(total)
}

/// The default CPUs, in thousandths: all but a quarter of the cores, at least one.
pub fn auto_cpu_millis(ncpu: u32) -> u64 {
    u64::from(ncpu.saturating_sub((ncpu / 4).max(1)).max(1)) * 1000
}

impl Wanted {
    /// The limits for an agent set to `agent`, under the app-wide `app`, on a daemon of `capacity`.
    pub fn resolve(agent: ContainerLimits, app: ContainerLimits, capacity: Option<DaemonCapacity>, recommended_min_mb: Option<u64>) -> Self {
        let pick = |own: Limit, shared: Limit| if own == Limit::Auto { shared } else { own };
        let memory = match pick(agent.memory, app.memory) {
            Limit::Unlimited => Target::Unlimited,
            Limit::Fixed(mb) => Target::Set((mb.saturating_mul(MIB)).min(i64::MAX as u64) as i64),
            Limit::Auto => match capacity {
                Some(c) => Target::Set(auto_memory_bytes(c.mem_total, recommended_min_mb.map(|mb| mb.saturating_mul(MIB))) as i64),
                None => Target::Leave,
            },
        };
        let cpus = match pick(agent.cpus, app.cpus) {
            Limit::Unlimited => Target::Unlimited,
            Limit::Fixed(millis) => Target::Set((millis.saturating_mul(1_000_000)).min(i64::MAX as u64) as i64),
            Limit::Auto => match capacity {
                Some(c) => Target::Set((auto_cpu_millis(c.ncpu) * 1_000_000) as i64),
                None => Target::Leave,
            },
        };
        Self { memory, cpus }
    }

    /// The limits, written into the `HostConfig` of a container about to be made. A limit that is not to apply is left out.
    pub fn apply_to(&self, host_config: &mut HostConfig) {
        if let Target::Set(bytes) = self.memory {
            host_config.memory = Some(bytes);
            // Swap as large as memory: Docker's default is twice, which pages the host's swap before it kills.
            host_config.memory_swap = Some(bytes);
        }
        if let Target::Set(nano) = self.cpus {
            host_config.nano_cpus = Some(nano);
        }
        host_config.pids_limit = Some(PIDS_LIMIT);
    }

    /// The `docker update` that takes a container with `current` to these limits, or `None` when it already has them.
    /// Memory and swap travel in one request, so the pair is valid whichever way the limit moves.
    pub fn update_from(&self, current: Option<&HostConfig>) -> Option<UpdateContainerOptions<String>> {
        let have = |f: fn(&HostConfig) -> Option<i64>| current.and_then(f).unwrap_or(0);
        let mut update = UpdateContainerOptions::<String>::default();
        let mut changed = false;
        match self.memory {
            Target::Set(bytes) if have(|h| h.memory) != bytes || have(|h| h.memory_swap) != bytes => {
                update.memory = Some(bytes);
                update.memory_swap = Some(bytes);
                changed = true;
            }
            Target::Unlimited if have(|h| h.memory) != 0 || have(|h| h.memory_swap) > 0 => {
                update.memory = Some(0);
                update.memory_swap = Some(-1);
                changed = true;
            }
            _ => {}
        }
        match self.cpus {
            Target::Set(nano) if have(|h| h.nano_cpus) != nano => {
                update.nano_cpus = Some(nano);
                changed = true;
            }
            Target::Unlimited if have(|h| h.nano_cpus) != 0 => {
                update.nano_cpus = Some(0);
                changed = true;
            }
            _ => {}
        }
        if have(|h| h.pids_limit) != PIDS_LIMIT {
            update.pids_limit = Some(PIDS_LIMIT);
            changed = true;
        }
        changed.then_some(update)
    }
}

/// The recommended minimum memory an image says it needs, in MB, from a container's inspect.
fn recommended_min_mb(details: &ContainerInspectResponse) -> Option<u64> {
    details.config.as_ref()?.labels.as_ref()?.get(RECOMMENDED_MIN_MEMORY_LABEL)?.trim().parse().ok()
}

fn capacity_cache() -> &'static Mutex<Option<(Instant, DaemonCapacity)>> {
    static CACHE: OnceLock<Mutex<Option<(Instant, DaemonCapacity)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

impl ContainerManager {
    /// What the daemon has, from `docker info`; kept for a few minutes. `None` when it can't be read (logged).
    pub async fn daemon_capacity(&self) -> Option<DaemonCapacity> {
        if let Some((at, capacity)) = *capacity_cache().lock().unwrap() {
            if at.elapsed() < CAPACITY_KEPT {
                return Some(capacity);
            }
        }
        let info = match self.inner.docker.info().await {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(error = %e, "container limits: can't read the daemon's size (docker info); not applying a default limit");
                return None;
            }
        };
        let capacity = DaemonCapacity {
            mem_total: info.mem_total.filter(|m| *m > 0)? as u64,
            ncpu: info.ncpu.filter(|n| *n > 0).map(|n| n as u32)?,
        };
        *capacity_cache().lock().unwrap() = Some((Instant::now(), capacity));
        Some(capacity)
    }

    /// The limits `agent` should carry on this daemon, under the app-wide settings.
    pub async fn wanted_limits(&self, agent: ContainerLimits, recommended_min_mb: Option<u64>) -> Wanted {
        Wanted::resolve(agent, ContainerLimits::from_settings(), self.daemon_capacity().await, recommended_min_mb)
    }

    /// The limits for a container about to be made from `image`, whose labels may name a recommended minimum.
    pub(super) async fn wanted_limits_for_new(&self, image: &str, agent: ContainerLimits) -> Wanted {
        let labels = self.inner.docker.inspect_image(image).await.ok().and_then(|i| i.config).and_then(|c| c.labels);
        let recommended = labels.as_ref().and_then(|l| l.get(RECOMMENDED_MIN_MEMORY_LABEL)).and_then(|v| v.trim().parse().ok());
        self.wanted_limits(agent, recommended).await
    }

    /// Puts the limits on an existing container (one made before they existed, or whose setting changed), without
    /// recreating it. A failure is logged and the container is left as it is. `None`: there is no such container (it
    /// takes the limits when it is made); `Some(true)`: it has them now; `Some(false)`: Docker refused the update.
    pub(crate) async fn limits_apply(&self, container_name: &str, agent: ContainerLimits) -> Option<bool> {
        let details = self.inner.docker.inspect_container(container_name, None).await.ok()?;
        let wanted = self.wanted_limits(agent, recommended_min_mb(&details)).await;
        let Some(update) = wanted.update_from(details.host_config.as_ref()) else { return Some(true) };
        match self.inner.docker.update_container(container_name, update).await {
            Ok(()) => {
                tracing::info!(container = container_name, ?wanted, "container limits updated");
                Some(true)
            }
            Err(e) => {
                tracing::warn!(container = container_name, error = %e, ?wanted, "container limits: docker refused the update; the container keeps its limits");
                Some(false)
            }
        }
    }
}

#[cfg(test)]
mod tests;
