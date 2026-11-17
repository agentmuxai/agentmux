// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent's container is made with besides its mounts: its network mode, the name `host.docker.internal`, and the
//! capabilities it does not get.

use bollard::models::{ContainerInspectResponse, HostConfig, Mount};

/// The capabilities an agent container does not get. `NET_RAW` is the one that lets a process in it forge another
/// container's address on the shared network, which is what a key account's token is bound to (`listener`); the rest of
/// Docker's default set stays, as agents build and run things in them.
pub(crate) const AGENT_CAP_DROP: &[&str] = &["NET_RAW"];

/// The host config of an agent's container: its mounts, no host network or privileged mode, the name `host.docker.internal`
/// resolvable, and [`AGENT_CAP_DROP`] dropped.
pub(super) fn agent_host_config(mounts: Vec<Mount>) -> HostConfig {
    HostConfig {
        mounts: Some(mounts),
        // Security: no host network, no privileged mode.
        network_mode: Some("bridge".to_string()),
        // Lets the container reach agentmux-srv via `host.docker.internal` (`run_agent_turn`/`agent_io`
        // rewrite AGENTMUX_LOCAL_URL to it, #2939). Docker Desktop resolves the name itself; `host-gateway`
        // is what makes it resolve on native Linux's default bridge network. A no-op where it already does.
        extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
        cap_drop: Some(AGENT_CAP_DROP.iter().map(|c| c.to_string()).collect()),
        ..Default::default()
    }
}

/// Whether an existing container already drops [`AGENT_CAP_DROP`]. Docker cannot drop a capability from a container that
/// exists, so one made before this is recreated, as for the mounts and `extra_hosts` (`container_drift`). Inspect data with
/// no `HostConfig` says nothing, and the container is left alone.
pub(super) fn drops_net_raw(details: &ContainerInspectResponse) -> bool {
    let Some(host_config) = details.host_config.as_ref() else { return true };
    let dropped = host_config.cap_drop.as_deref().unwrap_or_default();
    dropped.iter().any(|c| c.eq_ignore_ascii_case("ALL"))
        || AGENT_CAP_DROP.iter().all(|want| dropped.iter().any(|c| c.trim_start_matches("CAP_").eq_ignore_ascii_case(want)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_container_drops_net_raw_and_stays_unprivileged() {
        let host = agent_host_config(Vec::new());
        assert_eq!(host.cap_drop, Some(vec!["NET_RAW".to_string()]));
        assert_ne!(host.privileged, Some(true));
        assert_eq!(host.network_mode.as_deref(), Some("bridge"));
        assert!(host.extra_hosts.unwrap_or_default().iter().any(|h| h == "host.docker.internal:host-gateway"));
    }

    #[test]
    fn a_container_made_before_the_drop_is_recreated_and_one_that_has_it_is_not() {
        let with = |cap_drop: Option<Vec<&str>>| ContainerInspectResponse {
            host_config: Some(HostConfig { cap_drop: cap_drop.map(|c| c.into_iter().map(String::from).collect()), ..Default::default() }),
            ..Default::default()
        };
        assert!(!drops_net_raw(&with(None)), "an older container drops nothing");
        assert!(!drops_net_raw(&with(Some(vec!["MKNOD"]))), "another capability is not this one");
        assert!(drops_net_raw(&with(Some(vec!["NET_RAW"]))));
        assert!(drops_net_raw(&with(Some(vec!["CAP_NET_RAW", "MKNOD"]))));
        assert!(drops_net_raw(&with(Some(vec!["ALL"]))), "dropping everything includes it");
        assert!(drops_net_raw(&ContainerInspectResponse::default()), "no host config in the answer: left alone");
    }
}
