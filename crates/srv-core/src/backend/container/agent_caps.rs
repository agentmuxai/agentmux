// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent's container is made with besides its mounts: its network mode, the name `host.docker.internal`, and the
//! capabilities it does not get.

use bollard::models::{ContainerInspectResponse, HostConfig, Mount};

/// The capabilities an agent container does not get. `NET_RAW` is the one that lets a process in it forge another
/// container's address on the shared network, and `NET_ADMIN` the one that lets it change its own (an address, a route, a
/// filter): what a gateway token is bound to is the address the container really has (`listener`, `gateway_fwd`), so neither is
/// left to a container. The rest of Docker's default set stays, as agents build and run things in them.
pub(crate) const AGENT_CAP_DROP: &[&str] = &["NET_RAW", "NET_ADMIN"];
/// What an existing container must already drop to be kept. `NET_ADMIN` is not in Docker's default set, so a container made
/// before it was named here lacks it already; recreating every agent's container for a name in a list would only stop its work.
const DRIFT_REQUIRED: &[&str] = &["NET_RAW"];

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

/// Whether an existing container already drops [`DRIFT_REQUIRED`]. Docker cannot drop a capability from a container that
/// exists, so one made before this is recreated, as for the mounts and `extra_hosts` (`container_drift`). Inspect data with
/// no `HostConfig` says nothing, and the container is left alone.
pub(super) fn drops_net_raw(details: &ContainerInspectResponse) -> bool {
    let Some(host_config) = details.host_config.as_ref() else { return true };
    let dropped = host_config.cap_drop.as_deref().unwrap_or_default();
    dropped.iter().any(|c| c.eq_ignore_ascii_case("ALL"))
        || DRIFT_REQUIRED.iter().all(|want| dropped.iter().any(|c| c.trim_start_matches("CAP_").eq_ignore_ascii_case(want)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_container_drops_net_raw_and_stays_unprivileged() {
        let host = agent_host_config(Vec::new());
        assert_eq!(host.cap_drop, Some(vec!["NET_RAW".to_string(), "NET_ADMIN".to_string()]));
        assert!(host.cap_add.is_none(), "nothing is added back");
        assert!(host.pid_mode.is_none() && host.ipc_mode.is_none() && host.userns_mode.is_none(), "no host namespace");
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

    /// The gateway's per-container guarantee rests on two things about an agent's container: it cannot take another's address
    /// (no `NET_RAW`, no `NET_ADMIN`), and it cannot reach the container runtime to make a container of its own on the
    /// agents network. AgentMux mounts no runtime socket into one.
    #[test]
    fn an_agent_container_is_given_no_runtime_socket_and_cannot_change_its_network() {
        use crate::backend::container::{cli_dir_mount, mounts::agent_home_mounts, ContainerMountSpec};
        let spec = ContainerMountSpec {
            claude_config_host_dir: Some("/home/u/.claude-acct".to_string()),
            keychain_config_dir: Some("/tmp/staged".to_string()),
            workspace_host_dir: Some("/home/u/work".to_string()),
            ..Default::default()
        };
        let mut mounts = agent_home_mounts("agentmux-a1", &spec);
        mounts.push(cli_dir_mount("agentmux-a1"));
        assert!(!mounts.is_empty());
        let host = agent_host_config(mounts);
        for m in host.mounts.as_deref().unwrap_or_default() {
            for side in [m.source.as_deref().unwrap_or(""), m.target.as_deref().unwrap_or("")] {
                let side = side.to_ascii_lowercase();
                assert!(!side.contains("docker.sock") && !side.contains("podman.sock") && !side.contains("containerd.sock") && !side.ends_with("/run") && !side.ends_with("/var/run"), "{side}");
            }
        }
        assert!(host.binds.is_none(), "every mount is listed above");
        for cap in ["NET_RAW", "NET_ADMIN"] {
            assert!(AGENT_CAP_DROP.contains(&cap), "{cap}");
        }
        assert!(host.privileged != Some(true) && host.network_mode.as_deref() != Some("host"));
    }
}
