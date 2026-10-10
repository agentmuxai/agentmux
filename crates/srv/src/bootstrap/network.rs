// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of the single-file bootstrap module unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;

/// Output of [`bind_listeners_and_network`].
pub struct NetworkBundle {
    pub web_listener: TcpListener,
    pub ws_listener: TcpListener,
    pub web_addr: std::net::SocketAddr,
    pub ws_addr: std::net::SocketAddr,
    pub local_web_url: String,
    /// OS hostname, resolved once here because LAN discovery needs it for its
    /// mDNS TXT records. Carried on the bundle so `build_app_state` can put it
    /// on `AppState` too, rather than resolving it a second time.
    pub hostname: String,
    pub lan_discovery: Arc<backend::lan_discovery::LanDiscoveryController>,
    /// Owns the LAN-facing listeners so `network:lan_discovery` takes effect
    /// live, instead of only at the next startup — see
    /// `backend::lan_listeners`. Needs its router handed to it by `main.rs`
    /// after `build_router`.
    pub lan_listeners: Arc<backend::lan_listeners::LanListenerSupervisor>,
    pub lsp_supervisor: Arc<backend::lsp::LspSupervisor>,
    pub process_tracker: Arc<backend::process_tracker::registry::AgentProcessRegistry>,
    pub process_broker: Arc<crate::broker::ProcessBroker>,
}

/// Step 5: Bind 2 TCP listeners (web + ws — separate ports matching Go), then
/// bring up LAN discovery (mDNS), the LSP supervisor, stale registry cleanup,
/// and the process tracker/broker.
pub async fn bind_listeners_and_network(
    config: &config::Config,
    event_bus: &Arc<EventBus>,
    broker: &Arc<Broker>,
    version: &str,
) -> NetworkBundle {
    // Startup listeners are ALWAYS loopback-only, regardless of the
    // `network:lan_discovery` setting. LAN reachability is owned entirely by
    // `backend::lan_listeners::LanListenerSupervisor`, which binds one extra
    // socket per non-loopback interface address on these same ports and tears
    // them down when the setting flips off. `main.rs` calls `apply()` with the
    // current setting immediately after the router exists, so a user who
    // already had LAN enabled is reachable from boot — just via the
    // supervisor's sockets rather than a wildcard one here.
    //
    // Do NOT reintroduce a conditional `0.0.0.0:0` bind. A wildcard socket
    // holding the port makes the supervisor's per-address binds fail with
    // EADDRINUSE on Linux/macOS (they succeed on Windows, which lacks
    // SO_EXCLUSIVEADDRUSE by default — so the bug reproduces on only two of
    // three platforms). On Linux/macOS that leaves `active` empty, which drives
    // `sync_advertising` to `discovery.apply(false)` and silently switches off
    // the mDNS advertising the user had enabled — an ON→OFF regression on every
    // restart for exactly the users who already opted in. On Windows it instead
    // leaves two sockets on one port with ambiguous accept. Keeping startup
    // strictly loopback makes the supervisor the single owner of every LAN
    // socket on both counts. [reagent #3021 P0]
    //
    // The scoped `lan_key` (X-AuthKey header, broadcast in the mDNS TXT record)
    // gates only the three LAN-forwarding routes (`lan_or_full_auth_middleware`)
    // — not the full auth_key previously broadcast here, which gated the entire
    // API surface (see Config::lan_key's doc comment).
    // Loopback, OS-chosen ports — or, headless only, the fixed ports it was
    // given (`--web-port` / `--ws-port`, SPEC_SRV_HEADLESS_MODE_2026_09_26.md).
    //
    // Not headless: the next two free ports of `lan_ports::LAN_PORT_RANGE`, so
    // one firewall rule on that range covers every build and update
    // (SPEC_LAN_FIREWALL_SETUP_2026_10_01.md §4.1); OS-chosen only if the range
    // is exhausted. Headless keeps the fixed ports it was given.
    let (web_listener, ws_listener) = if crate::headless::active() {
        let web_bind = crate::headless::startup_bind_addr(crate::headless::Listener::Web);
        let ws_bind = crate::headless::startup_bind_addr(crate::headless::Listener::Ws);
        let web = TcpListener::bind(&web_bind)
            .await
            .unwrap_or_else(|e| panic!("failed to bind web listener on {web_bind}: {e}"));
        let ws = TcpListener::bind(&ws_bind)
            .await
            .unwrap_or_else(|e| panic!("failed to bind ws listener on {ws_bind}: {e}"));
        (web, ws)
    } else {
        let bound = backend::lan_ports::bind_startup_listeners().await;
        if !bound.in_range {
            tracing::warn!(
                range = ?backend::lan_ports::LAN_PORT_RANGE,
                "LAN port range exhausted: listening on OS-chosen ports, which a firewall rule on the range does not cover"
            );
        }
        (bound.web, bound.ws)
    };

    let web_addr = web_listener.local_addr().unwrap();
    let ws_addr = ws_listener.local_addr().unwrap();
    // Always use 127.0.0.1 for the local URL regardless of bind address.
    // When bound to 0.0.0.0, local_addr() returns 0.0.0.0:PORT which is not a
    // valid connect destination (fails on Windows and some Linux configs).
    let local_web_url = format!("http://127.0.0.1:{}", web_addr.port());

    // Make local backend URL available to child processes (PTY shells).
    // the muxbus client (agentbus-client package) reads AGENTMUX_LOCAL_URL for local PTY delivery
    // instead of routing through the cloud muxbus relay.
    std::env::set_var("AGENTMUX_LOCAL_URL", &local_web_url);

    // LAN discovery via mDNS — opt-in to avoid Windows Firewall prompt.
    // mDNS binds 0.0.0.0:5353 UDP which triggers the firewall dialog.
    // The setting defaults to false; users opt in via the HostPopover toggle
    // (or by editing settings.json). The controller supports live start/stop
    // so flipping the setting does not require an app restart.
    // See docs/specs/lan-discovery-toggle.md.
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    let lan_discovery = Arc::new(backend::lan_discovery::LanDiscoveryController::new(
        config.instance_id.clone(),
        hostname.clone(),
        version.to_string(),
        web_addr.port(),
        event_bus.clone(),
        config.lan_key.clone(),
    ));
    // The LAN tier of one-live-instance-per-agent asks these peers.
    backend::agent_admission::set_lan_discovery(lan_discovery.clone());
    // Notices an instance that hears peers but cannot be heard (no IPv4 mDNS
    // socket: `lan_mdns_health`), rebuilds the daemon, and tells the indicator.
    // Idle while LAN discovery is off.
    lan_discovery.clone().spawn_health_watchdog();
    // Deliberately NOT applied here. The supervisor below is the single driver
    // of `lan_discovery.apply`, so mDNS can never advertise an endpoint before
    // (or without) a socket actually listening on it. `main.rs` reads the
    // setting and calls `lan_listeners.apply(..)` as soon as the router exists,
    // which reconciles listeners and then gates advertising on the result.

    // LAN listeners. Startup bound loopback only (always — see the bind comment
    // above); this supervisor adds/removes the LAN-facing listeners on the SAME
    // ports so the setting takes effect live rather than at the next restart.
    // `apply` is deferred to `main.rs`, which is where the router it needs to
    // serve finally exists.
    let lan_listeners = Arc::new(backend::lan_listeners::LanListenerSupervisor::new(
        web_addr.port(),
        ws_addr.port(),
    ));
    // Headless srv is loopback-only, whatever the channel's LAN setting says.
    if crate::headless::active() {
        lan_listeners.forbid_lan();
    }
    // The supervisor owns the advertise/reachable pairing: it re-gates mDNS on
    // every reconcile so we never advertise an address nothing is listening on,
    // and it performs the boot-time setting read too (via `main.rs`).
    lan_listeners.set_discovery(lan_discovery.clone());
    // Tells the status bar whether the OS firewall would let LAN peers in
    // (Windows only for now). Read-only.
    backend::lan_firewall::spawn_watcher(
        lan_discovery.clone(),
        web_addr.port(),
        ws_addr.port(),
        event_bus.clone(),
    );

    // LSP supervisor — owns LSP server child processes. Nothing spawned
    // until the editor pane calls `lspstart`. Spec:
    // docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md
    let lsp_supervisor = Arc::new(backend::lsp::LspSupervisor::new(event_bus.clone()));

    // Clean up stale cross-instance agent registry entries (entries older than 4h).
    backend::reactive::registry::cleanup_stale(
        &base::get_mux_data_dir(),
        4 * 60 * 60 * 1000,
    );

    // Same sweep for the host-global shared registry (Tier 2b, issue #1916)
    // — additionally drops entries whose owning PID no longer exists on this
    // host, since a channel that crashed without a clean unregister can
    // leave an entry behind indefinitely otherwise (no other channel's
    // startup would ever revisit it).
    if let Some(shared_dir) = registry::resolve_shared_reactive_dir() {
        backend::reactive::registry::cleanup_stale_shared(&shared_dir, 4 * 60 * 60 * 1000);
    }

    // Periodic re-registration heartbeat for every LOCALLY-registered agent's
    // Tier 2 (per-channel) and Tier 2b (host-global shared) registry
    // entries. Without this, an entry's `updated_at` is stamped once at
    // registration and never refreshed — so `should_evict_on_forward_failure`
    // (registry.rs)'s grace-period check is only ever true in the ~60s right
    // after an agent registers, useless for any steady-state agent (i.e.
    // most of them). reagent P1 round 3 on PR #2640: the PID+age eviction
    // guard from the earlier commits on that PR doesn't actually protect a
    // live, long-running agent from a single transient forward failure,
    // reproducing the exact bug the PR claims to fix. This heartbeat closes
    // that gap: `reactive::get_global_handler().list_agents()` is the
    // in-memory Tier-1 registry, updated synchronously on every
    // register/unregister — so it's always accurate, with no staleness
    // window of its own. Re-writing from it keeps a live agent's on-disk
    // entries fresh indefinitely; a genuinely-dead agent simply stops
    // appearing in `list_agents()` (its own register/unregister lifecycle
    // already handles that), so its entries age past the grace period and
    // become evictable again within one heartbeat interval. The interval is
    // well under `FORWARD_FAILURE_GRACE_MS` so a live entry's age never
    // approaches the grace threshold between heartbeats. See
    // docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
    {
        let local_web_url = local_web_url.clone();
        tokio::spawn(async move {
            const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;
                let data_dir = base::get_mux_data_dir();
                for reg in reactive::get_global_handler().list_agents() {
                    // _with_nonce, not the plain write/write_shared_from_env
                    // — those hardcode registration_nonce: 0, which would
                    // silently break remove_if_nonce/
                    // remove_shared_from_env_if_nonce's compare-and-remove
                    // on this agent's own clean-exit cleanup (reagent P1
                    // round 4 on PR #2640: every 20s heartbeat tick would
                    // zero out the real nonce already recorded in
                    // `reg.registration_nonce`).
                    backend::reactive::registry::write_with_nonce(
                        &data_dir,
                        &reg.agent_id,
                        &local_web_url,
                        &reg.block_id,
                        reg.registration_nonce,
                    );
                    backend::reactive::registry::write_shared_from_env_with_nonce(
                        &reg.agent_id,
                        &local_web_url,
                        &reg.block_id,
                        reg.registration_nonce,
                    );
                }
            }
        });
    }

    // Linux: join a delegated systemd user scope first, so each agent gets a
    // cgroup holding everything it starts (process_tracker::cgroup_linux).
    // Before the registry exists, so no agent can spawn outside it.
    #[cfg(target_os = "linux")]
    backend::process_tracker::cgroup_linux::init().await;

    // Tracks agent-spawned OS processes per block. Registered trackers
    // live as long as their agent pane; the background poller emits
    // delta events (`agent:process-added`/`-exited`) to the frontend.
    let process_tracker = std::sync::Arc::new(
        backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
    );
    backend::process_tracker::registry::set_global(process_tracker.clone());
    backend::process_tracker::registry::spawn_poller(process_tracker.clone());

    // Process Broker (Phase A) — unified read path over blockcontroller +
    // process_tracker. See docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md.
    let process_broker = std::sync::Arc::new(crate::broker::ProcessBroker::new(Some(broker.clone())));
    crate::broker::process::set_global(process_broker.clone());

    NetworkBundle {
        web_listener,
        ws_listener,
        web_addr,
        ws_addr,
        local_web_url,
        hostname,
        lan_discovery,
        lan_listeners,
        lsp_supervisor,
        process_tracker,
        process_broker,
    }
}
