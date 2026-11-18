// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The gateway forwarder: how a container agent's model-gateway request is tied to its own container where srv cannot see
//! which container it comes from (Docker Desktop: every container arrives from one address).
//!
//! A helper container, [`CONTAINER_NAME`], runs `agentmux-mcp gateway-fwd` on the agents network. Container agents are
//! pointed at it (`key_route`); it sees each one's own address, puts it in a stamp
//! ([`agentmux_common::gateway_stamp`]) on the request, and passes the request to srv. srv accepts the stamp only with the
//! proof made from this launch's secret ([`verify_stamp`]), so the address in it is the one the forwarder saw.
//!
//! The helper gets no agent token, no Vault access, no mount and no capability. The secret reaches it only in its
//! environment at creation, is made for each srv launch, and is never logged or written to a file. It is not restarted, it
//! is removed when it stops, and srv removes it when srv ends ([`remove`]); one left by a srv that did not end cleanly has
//! another launch's secret and is replaced at the next turn.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use agentmux_common::gateway_stamp::{self, Refusal};
use axum::http::HeaderMap;
use bollard::container::{Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions};
use bollard::models::{ContainerInspectResponse, HostConfig, RestartPolicy, RestartPolicyNameEnum};

use super::{ContainerManager, AGENTMUX_DOCKER_NETWORK};

/// The helper's fixed name. Its address on the agents network is found by this name (Docker's own DNS on that network).
pub const CONTAINER_NAME: &str = "agentmux-gateway-fwd";
/// The label that names the helper as AgentMux's, and what the container list says it is.
pub const HELPER_LABEL: &str = "ai.agentmux.helper";
pub const HELPER_TEXT: &str = "AgentMux gateway forwarder";
/// The port it listens on, on the agents network.
pub const PORT: u16 = 4799;
const ENV_SECRET: &str = "AGENTMUX_GWFWD_SECRET";
const ENV_UPSTREAM: &str = "AGENTMUX_GWFWD_UPSTREAM";
const ENV_PORT: &str = "AGENTMUX_GWFWD_PORT";

/// Said when the forwarder cannot be had: nothing falls back to srv's own address.
const NOT_RUNNING: &str = "AgentMux could not start its gateway forwarder, which ties this agent's model access to its container on this machine. Run the agent on this machine instead of in a container, or move it to a plan account, and tell the AgentMux team if this keeps happening.";

/// This launch's secret, made on first use.
fn secret() -> &'static str {
    static SECRET: OnceLock<String> = OnceLock::new();
    SECRET.get_or_init(|| format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple()))
}

/// The forwarder's base URL, as a container on the agents network reaches it.
pub fn url() -> String {
    format!("http://{CONTAINER_NAME}:{PORT}")
}

fn now_secs() -> u64 {
    agentmux_common::time::now_ms_u64() / 1000
}

/// The container a request was stamped for, when its stamp is good for this request now.
pub fn verify_stamp(headers: &HeaderMap, method: &str, target: &str) -> Result<IpAddr, Refusal> {
    verify_stamp_at(headers, method, target, now_secs())
}

fn verify_stamp_at(headers: &HeaderMap, method: &str, target: &str, now: u64) -> Result<IpAddr, Refusal> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    gateway_stamp::verify(secret().as_bytes(), get(gateway_stamp::HEADER_PEER), get(gateway_stamp::HEADER_TS), get(gateway_stamp::HEADER_PROOF), method, target, now)
}

/// The three headers the forwarder would put on a request from `peer` to `target`, made now with this launch's secret. Only so a test of
/// srv's side can send one: the real ones are made in the helper container, which has its own copy of the secret.
#[doc(hidden)]
pub fn stamp_headers(peer: IpAddr, method: &str, target: &str) -> Vec<(&'static str, String)> {
    let ts = now_secs();
    vec![
        (gateway_stamp::HEADER_PEER, peer.to_string()),
        (gateway_stamp::HEADER_TS, ts.to_string()),
        (gateway_stamp::HEADER_PROOF, gateway_stamp::proof(secret().as_bytes(), peer, ts, method, target)),
    ]
}

/// What the helper is made with. No mount, no capability, no restart, nothing but the agents network and the name of the host.
pub(super) fn forwarder_config(image: &str, upstream: &str, secret: &str) -> Config<String> {
    Config {
        image: Some(image.to_string()),
        cmd: Some(vec!["agentmux-mcp".to_string(), "gateway-fwd".to_string()]),
        env: Some(vec![format!("{ENV_SECRET}={secret}"), format!("{ENV_UPSTREAM}={upstream}"), format!("{ENV_PORT}={PORT}")]),
        labels: Some(HashMap::from([(HELPER_LABEL.to_string(), HELPER_TEXT.to_string())])),
        host_config: Some(HostConfig {
            network_mode: Some(AGENTMUX_DOCKER_NETWORK.to_string()),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges".to_string()]),
            readonly_rootfs: Some(true),
            restart_policy: Some(RestartPolicy { name: Some(RestartPolicyNameEnum::NO), maximum_retry_count: None }),
            auto_remove: Some(true),
            pids_limit: Some(128),
            memory: Some(128 * 1024 * 1024),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Whether an existing container is this launch's forwarder for this image and this srv address, and is running.
fn is_current(details: &ContainerInspectResponse, image: &str, upstream: &str) -> bool {
    let running = details.state.as_ref().and_then(|s| s.running).unwrap_or(false);
    let config = details.config.as_ref();
    let env = config.and_then(|c| c.env.as_deref()).unwrap_or_default();
    let has = |entry: String| env.iter().any(|e| *e == entry);
    running && config.and_then(|c| c.image.as_deref()) == Some(image) && has(format!("{ENV_SECRET}={}", secret())) && has(format!("{ENV_UPSTREAM}={upstream}"))
}

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Makes sure this launch's forwarder is running for an agent container's image and returns its base URL, or says why not.
/// `env` is the turn's environment, whose `AGENTMUX_LOCAL_URL` is where srv is reached from a container.
pub(super) async fn ensure(cm: &ContainerManager, agent_container: &str, env: &[(String, String)]) -> Result<String, String> {
    let upstream = env.iter().find(|(k, _)| k == "AGENTMUX_LOCAL_URL").map(|(_, v)| v.trim_end_matches('/').to_string()).filter(|v| !v.is_empty()).ok_or_else(|| NOT_RUNNING.to_string())?;
    let image = cm.inner.docker.inspect_container(agent_container, None).await.ok().and_then(|d| d.config).and_then(|c| c.image).ok_or_else(|| NOT_RUNNING.to_string())?;
    // One at a time, so two turns starting together make one forwarder.
    let _one = LOCK.lock().await;
    if let Ok(existing) = cm.inner.docker.inspect_container(CONTAINER_NAME, None).await {
        if is_current(&existing, &image, &upstream) {
            return Ok(url());
        }
        // Another launch's, another image, or not running.
        let _ = cm.inner.docker.remove_container(CONTAINER_NAME, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
    }
    if !cm.ensure_network_exists().await {
        return Err(NOT_RUNNING.to_string());
    }
    let created = cm.inner.docker.create_container(Some(CreateContainerOptions { name: CONTAINER_NAME, platform: None }), forwarder_config(&image, &upstream, secret())).await;
    let started = match created {
        Ok(c) if !c.id.is_empty() => cm.inner.docker.start_container(&c.id, None::<StartContainerOptions<String>>).await.is_ok(),
        _ => false,
    };
    if !started || !stays_up(cm).await {
        tracing::warn!(image = %image, "gateway forwarder: could not start; a container agent's model access is refused until it can");
        return Err(NOT_RUNNING.to_string());
    }
    tracing::info!(container = CONTAINER_NAME, "gateway forwarder: started on the agents network");
    Ok(url())
}

/// Whether the forwarder is still there after a moment: one that cannot run (an image from before it existed) exits at once,
/// and is removed, as it has `auto_remove`.
async fn stays_up(cm: &ContainerManager) -> bool {
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let running = cm.inner.docker.inspect_container(CONTAINER_NAME, None).await.ok().and_then(|d| d.state).and_then(|s| s.running).unwrap_or(false);
        if !running {
            return false;
        }
    }
    true
}

/// Removes the forwarder, if there is one: when srv ends, and before a launch's own is made. Best effort.
pub async fn remove(cm: &ContainerManager) {
    let _one = LOCK.lock().await;
    if cm.inner.docker.remove_container(CONTAINER_NAME, Some(RemoveContainerOptions { force: true, ..Default::default() })).await.is_ok() {
        tracing::info!(container = CONTAINER_NAME, "gateway forwarder: removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::gateway_stamp::{proof, HEADER_PEER, HEADER_PROOF, HEADER_TS};
    use bollard::models::{ContainerConfig, ContainerState};

    const TARGET: &str = "/api/v1/model/anthropic/v1/messages";

    fn peer() -> IpAddr {
        "172_30_0_9".replace('_', ".").parse().unwrap()
    }

    fn stamped(secret: &str, ts: u64, method: &str, target: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(HEADER_PEER, peer().to_string().parse().unwrap());
        h.insert(HEADER_TS, ts.to_string().parse().unwrap());
        h.insert(HEADER_PROOF, proof(secret.as_bytes(), peer(), ts, method, target).parse().unwrap());
        h
    }

    #[test]
    fn the_forwarder_is_made_with_nothing_to_spare() {
        let config = forwarder_config("agentmux-agent:1", "http://host.docker.internal:7", "s3cret");
        let host = config.host_config.as_ref().unwrap();
        assert!(host.mounts.is_none() && host.binds.is_none(), "no mount, so no Docker socket and no host file");
        assert_eq!(host.cap_drop.as_deref(), Some(&["ALL".to_string()][..]));
        assert_eq!(host.readonly_rootfs, Some(true));
        assert_eq!(host.auto_remove, Some(true), "removed when it stops");
        assert!(matches!(host.restart_policy.as_ref().and_then(|p| p.name.as_ref()), Some(RestartPolicyNameEnum::NO)), "no restart policy");
        assert_eq!(host.network_mode.as_deref(), Some(AGENTMUX_DOCKER_NETWORK));
        assert!(host.privileged.is_none() && host.cap_add.is_none() && host.pid_mode.is_none() && host.volumes_from.is_none());
        assert_eq!(config.labels.as_ref().and_then(|l| l.get(HELPER_LABEL)).map(String::as_str), Some(HELPER_TEXT), "named as AgentMux's gateway forwarder");
        assert_eq!(config.cmd.as_deref().map(|c| c.join(" ")).as_deref(), Some("agentmux-mcp gateway-fwd"));
        let env = config.env.unwrap();
        assert!(env.iter().any(|e| e == &format!("{ENV_SECRET}=s3cret")), "the secret is in its environment, and only there");
        assert!(!config.cmd.unwrap().iter().any(|a| a.contains("s3cret")), "never in a command line");
        for e in &env {
            assert!(!e.contains("AUTH_KEY") && !e.contains("amxc_") && !e.contains("amxg_"), "no agent token or instance key: {}", e.split('=').next().unwrap());
        }
    }

    #[test]
    fn an_existing_forwarder_is_kept_only_for_this_launch_image_and_address() {
        let made = |env: Vec<String>, image: &str, running: bool| ContainerInspectResponse {
            state: Some(ContainerState { running: Some(running), ..Default::default() }),
            config: Some(ContainerConfig { image: Some(image.to_string()), env: Some(env), ..Default::default() }),
            ..Default::default()
        };
        let env = |secret: &str, upstream: &str| vec![format!("{ENV_SECRET}={secret}"), format!("{ENV_UPSTREAM}={upstream}")];
        let up = "http://host.docker.internal:7";
        assert!(is_current(&made(env(secret(), up), "img:1", true), "img:1", up));
        assert!(!is_current(&made(env(secret(), up), "img:1", false), "img:1", up), "not running");
        assert!(!is_current(&made(env("another-launchs", up), "img:1", true), "img:1", up), "another launch's secret");
        assert!(!is_current(&made(env(secret(), up), "img:2", true), "img:1", up), "another image");
        assert!(!is_current(&made(env(secret(), "http://host.docker.internal:8"), "img:1", true), "img:1", up), "srv is at another port");
    }

    #[test]
    fn srv_accepts_a_stamp_only_with_this_launchs_proof_for_this_request() {
        let now = 5000;
        let good = stamped(secret(), now, "POST", TARGET);
        assert_eq!(verify_stamp_at(&good, "POST", TARGET, now), Ok(peer()));
        assert_eq!(verify_stamp_at(&HeaderMap::new(), "POST", TARGET, now), Err(Refusal::Missing), "a request straight to srv has no stamp");
        assert_eq!(verify_stamp_at(&stamped("a-guess", now, "POST", TARGET), "POST", TARGET, now), Err(Refusal::BadProof), "a stamp made without the secret");
        assert_eq!(verify_stamp_at(&good, "GET", TARGET, now), Err(Refusal::BadProof), "moved to another method");
        assert_eq!(verify_stamp_at(&good, "POST", "/api/v1/model/openrouter/v1/messages", now), Err(Refusal::BadProof), "moved to another path");
        assert_eq!(verify_stamp_at(&good, "POST", TARGET, now + 31), Err(Refusal::Expired));
        let outside = "/api/v1/containers";
        assert_eq!(verify_stamp_at(&stamped(secret(), now, "GET", outside), "GET", outside, now), Err(Refusal::OutsidePrefix), "never on another path, even with the secret");
    }

    #[test]
    fn the_forwarder_is_found_by_its_name_on_the_agents_network() {
        assert_eq!(url(), format!("http://agentmux-gateway-fwd:{PORT}"));
        assert!(CONTAINER_NAME.starts_with("agentmux-"), "inside the names AgentMux's container list and cleanup already look at");
    }
}
