// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telling a container agent's user that something in the container was stopped for going over its memory limit,
//! instead of a bare exit 137. `docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md` §5.
//!
//! The evidence is the container cgroup's own `oom_kill` counter (`memory.events` on cgroup v2, `memory.oom_control` on
//! v1): read when a turn starts and again when it ends, a rise means the kernel killed a process in this container in
//! between. An exit code of 137 alone is not proof (a stop is SIGKILL too), and the usual victim is not the turn but a
//! child of the CLI, such as the `cargo` of a Bash tool, so the turn can end cleanly and still have been hit.

use futures_util::StreamExt as _;
use std::sync::Arc;

use super::ContainerManager;

/// Prints the cgroup's OOM counters, whichever cgroup version the container is on.
const READ_COUNTER: &str = "cat /sys/fs/cgroup/memory.events 2>/dev/null || cat /sys/fs/cgroup/memory/memory.oom_control 2>/dev/null";

/// The `oom_kill` value in the text of `memory.events` / `memory.oom_control`.
pub fn parse_oom_kill(text: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        (parts.next()? == "oom_kill").then(|| parts.next()?.parse().ok()).flatten()
    })
}

/// A process of the container was killed for memory during a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OomEvent {
    /// The memory limit the container has, in MB; `None` when it has none (the daemon itself ran short).
    pub memory_mb: Option<u64>,
}

impl ContainerManager {
    /// How many processes the kernel has killed in this container for memory since it started; `None` when it can't be read.
    pub async fn oom_kill_count(&self, container_name: &str) -> Option<u64> {
        let session = self.exec(container_name, &["sh".to_string(), "-c".to_string(), READ_COUNTER.to_string()], None, &[], false).await.ok()?;
        let mut text = String::new();
        let mut output = session.output;
        while let Some(Ok(frame)) = output.next().await {
            text.push_str(&frame.to_string());
        }
        parse_oom_kill(&text)
    }

    /// Whether a process was killed for memory since the count was `before`.
    pub async fn oom_since(&self, container_name: &str, before: Option<u64>) -> Option<OomEvent> {
        let before = before?;
        if self.oom_kill_count(container_name).await? <= before {
            return None;
        }
        let limit = self.inner.docker.inspect_container(container_name, None).await.ok().and_then(|d| d.host_config).and_then(|h| h.memory);
        Some(OomEvent { memory_mb: limit.filter(|bytes| *bytes > 0).map(|bytes| bytes as u64 / (1024 * 1024)) })
    }
}

/// What the failure of a turn that ended by SIGKILL says when memory was the cause.
pub fn explain_failure(failure: &mut crate::agents::failure::AgentFailure, event: &OomEvent) {
    if failure.code != crate::agents::failure::FailureClass::Killed {
        return;
    }
    failure.title = "Ran out of memory".to_string();
    failure.detail = match event.memory_mb {
        Some(mb) => format!("The container's memory limit ({}) was exceeded, so the kernel stopped a process in it. Raise the limit in the agent's container settings, or reduce what runs at once.", memory_text(mb)),
        None => "The container ran out of memory, so the kernel stopped a process in it. Docker's own memory may be too small: raise it in Docker Desktop's resource settings, or reduce what runs at once.".to_string(),
    };
}

/// `8192` as `8 GB`, `1536` as `1.5 GB`, `256` as `256 MB`.
pub fn memory_text(mb: u64) -> String {
    if mb < 1024 {
        return format!("{mb} MB");
    }
    let gb = mb as f64 / 1024.0;
    if (gb - gb.round()).abs() < 0.05 { format!("{} GB", gb.round() as u64) } else { format!("{gb:.1} GB") }
}

/// Writes the pane row for an [`OomEvent`] of the pane `block_id`.
pub fn note_in_pane(
    event: &OomEvent,
    block_id: &str,
    broker: &Option<Arc<crate::backend::mps::Broker>>,
    filestore: &Option<Arc<crate::backend::storage::filestore::FileStore>>,
    mstore: &Option<Arc<crate::backend::storage::store::Store>>,
) {
    let (Some(broker), Some(filestore), Some(mstore)) = (broker, filestore, mstore) else { return };
    let meta = |key: &str, default: &str| match mstore.get::<crate::backend::obj::Block>(block_id) {
        Ok(Some(block)) => crate::backend::obj::meta_get_string(&block.meta, key, default),
        _ => default.to_string(),
    };
    let (provider, agent_id, agent_name) = (meta("agentProvider", "claude"), meta("agentId", ""), meta("agentName", ""));
    let frame = crate::backend::cli_notice::container_limit_frame(
        &provider,
        block_id,
        (&agent_id, &agent_name),
        "oom",
        event.memory_mb,
        None,
        None,
        None,
    );
    crate::backend::cli_notice::append_to_pane(broker, filestore, mstore, block_id, &frame);
    tracing::warn!(block_id, memory_mb = ?event.memory_mb, "a process in the container was killed for memory");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_is_read_from_either_cgroup_version() {
        let v2 = "low 0\nhigh 0\nmax 12\noom 3\noom_kill 2\noom_group_kill 0\n";
        assert_eq!(parse_oom_kill(v2), Some(2));
        let v1 = "oom_kill_disable 0\nunder_oom 0\noom_kill 5\n";
        assert_eq!(parse_oom_kill(v1), Some(5));
        assert_eq!(parse_oom_kill("oom_kill_disable 0\nunder_oom 0\n"), None, "an old kernel has no counter");
        assert_eq!(parse_oom_kill(""), None);
    }

    #[test]
    fn a_killed_turn_names_the_limit_and_another_failure_is_left_alone() {
        let killed = || crate::agents::failure::classify(Some(137), None, "", None);
        let mut failure = killed();
        explain_failure(&mut failure, &OomEvent { memory_mb: Some(8192) });
        assert_eq!(failure.title, "Ran out of memory");
        assert!(failure.detail.contains("8 GB"), "{}", failure.detail);
        let mut unlimited = killed();
        explain_failure(&mut unlimited, &OomEvent { memory_mb: None });
        assert!(unlimited.detail.contains("Docker's own memory"), "{}", unlimited.detail);

        let mut other = crate::agents::failure::classify(Some(1), None, "HTTP 429 rate_limit", None);
        let before = other.clone();
        explain_failure(&mut other, &OomEvent { memory_mb: Some(8192) });
        assert_eq!(other, before);
    }

    #[test]
    fn memory_reads_in_the_unit_a_person_would_say() {
        assert_eq!(memory_text(256), "256 MB");
        assert_eq!(memory_text(8192), "8 GB");
        assert_eq!(memory_text(1536), "1.5 GB");
        assert_eq!(memory_text(1024), "1 GB");
    }
}
