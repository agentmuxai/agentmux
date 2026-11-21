// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `bind_listeners_and_network` leaves behind at startup: where it listens, what it exports
//! and which background work it starts. Each test holds the env lock and an isolated data folder,
//! because the function reads the data directories and sets `AGENTMUX_LOCAL_URL`.

use super::*;
use crate::data_env_tests::DataEnv;

fn test_config() -> config::Config {
    config::Config {
        auth_key: "test-auth".to_string(),
        lan_key: "test-lan".to_string(),
        host_reg_secret: None,
        data_home: std::path::PathBuf::new(),
        config_home: std::path::PathBuf::new(),
        app_path: String::new(),
        is_dev: true,
        version: "0.0.0-test",
        build_time: "test",
        instance_id: "network-test".to_string(),
    }
}

async fn bind() -> NetworkBundle {
    let parts = crate::test_support::core_parts();
    bind_listeners_and_network(&test_config(), &parts.event_bus, &parts.broker, "0.0.0-test").await
}

#[tokio::test]
async fn startup_listens_on_loopback_only_on_two_different_ports() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    let net = bind().await;
    assert!(net.web_addr.ip().is_loopback(), "{}", net.web_addr);
    assert!(net.ws_addr.ip().is_loopback(), "{}", net.ws_addr);
    assert_ne!(net.web_addr.port(), net.ws_addr.port());
    assert_eq!(net.web_listener.local_addr().unwrap(), net.web_addr);
    assert_eq!(net.ws_listener.local_addr().unwrap(), net.ws_addr);
}

#[tokio::test]
async fn the_local_url_is_127_0_0_1_on_the_web_port_and_is_exported_to_child_processes() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    let saved = std::env::var_os("AGENTMUX_LOCAL_URL");
    let net = bind().await;
    assert_eq!(net.local_web_url, format!("http://127.0.0.1:{}", net.web_addr.port()));
    assert_eq!(std::env::var("AGENTMUX_LOCAL_URL").unwrap(), net.local_web_url);
    match saved {
        Some(v) => std::env::set_var("AGENTMUX_LOCAL_URL", v),
        None => std::env::remove_var("AGENTMUX_LOCAL_URL"),
    }
}

#[tokio::test]
async fn the_bundle_carries_the_host_name_and_the_process_services_it_registered() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    let net = bind().await;
    assert_eq!(net.hostname, whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()));
    assert!(backend::process_tracker::registry::global().is_some());
    assert!(crate::broker::process::global().is_some());
    assert!(Arc::strong_count(&net.lan_discovery) >= 2, "the discovery controller is shared with admission, the supervisor and the watchers");
}

#[tokio::test]
async fn a_locally_registered_agent_gets_its_registry_entry_refreshed_with_the_local_url() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    reactive::get_global_handler()
        .register_agent_full("network-heartbeat-agent", "block-hb", None, 7, None, Some("uid-hb"), "test.network")
        .unwrap();
    let net = bind().await;
    let data_dir = base::get_mux_data_dir();
    let mut entry = None;
    for _ in 0..50 {
        entry = backend::reactive::registry::lookup(&data_dir, "network-heartbeat-agent");
        if entry.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let entry = entry.expect("the heartbeat writes the entry on its first tick");
    assert_eq!(entry.local_url, net.local_web_url);
    assert_eq!(entry.block_id, "block-hb");
    assert_eq!(entry.registration_nonce, 7, "the heartbeat keeps the agent's real registration nonce");
}
