// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LAN-facing listener management.
//!
//! Background: `bootstrap.rs` resolves the srv bind address **once at
//! startup** — `0.0.0.0` when `network:lan_discovery` is on, `127.0.0.1`
//! otherwise. Toggling the setting at runtime therefore starts/stops mDNS
//! (live, via `LanDiscoveryController::apply`) but leaves the listeners where
//! they were, producing the worst possible state: peers discover this instance
//! and then cannot reach it. See
//! `docs/reports/REPORT_LAN_TOGGLE_WITHOUT_RESTART_2026_09_06.md` and
//! `docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md`.
//!
//! The fix does **not** require rebinding an active axum server (the reason
//! the in-code comment gave for deferring). A *specific* non-loopback address
//! can be bound on a port already held by loopback — so LAN reachability is
//! added by binding ADDITIONAL listeners alongside the untouched loopback
//! ones, and removed by dropping them.
//!
//! That claim is load-bearing, so it is asserted by a test here
//! (`loopback_and_specific_ip_can_share_a_port`) rather than assumed. It runs
//! on every CI platform, so a platform where it stops holding fails the build
//! instead of shipping a silently broken toggle.
//!
//! Note what is deliberately NOT claimed: that `0.0.0.0:PORT` conflicts with an
//! existing `127.0.0.1:PORT`. That is true on Linux/macOS but **false on
//! Windows** (measured 2026-09-06 — absent `SO_EXCLUSIVEADDRUSE` the wildcard
//! binds alongside loopback). An earlier draft asserted the universal version
//! and the test failed on Windows, which is what caught it. The consequence is
//! that "just bind the wildcard when enabled" is not a portable shortcut:
//! on Windows it would leave two sockets on one port with ambiguous accept
//! behaviour. Binding specific addresses has no such ambiguity anywhere. See
//! `wildcard_vs_loopback_conflict_is_platform_dependent`.

/// Every non-loopback address this host is currently reachable on.
///
/// Used to decide which addresses to bind LAN listeners on. Enumerates real
/// interfaces rather than taking only the primary outbound route
/// ([`primary_lan_ipv4`]) because a multi-homed host — Ethernet *and* Wi-Fi
/// both up, a common docked-laptop setup — is reachable by peers on either,
/// and binding only one would make the instance invisible to half the network.
///
/// `if-addrs` is already in the dependency tree via `mdns-sd`, so declaring it
/// directly adds no new supply-chain surface.
///
/// Excluded deliberately:
/// - **loopback** — already bound, and adds no LAN reachability.
/// - **IPv6 link-local (`fe80::/10`)** — requires a scope id to be usable as a
///   bind/connect target, and peers advertise reachable addresses via mDNS
///   anyway. Including them would produce listeners nothing can dial.
/// The address srv's own startup listeners bind, unconditionally.
///
/// **Loopback-only is load-bearing, not a default.** This supervisor is the
/// sole owner of every LAN-facing socket; startup must not pre-empt it with a
/// wildcard bind. A `0.0.0.0:PORT` socket holding the port makes
/// [`LanListenerSupervisor::spawn_pair`]'s per-address binds fail with
/// `EADDRINUSE` on Linux/macOS, which leaves `active` empty and drives
/// [`LanListenerSupervisor::sync_advertising`] to switch mDNS *off* — silently
/// disabling LAN on every restart for users who had already enabled it. (On
/// Windows the binds succeed instead and you get two sockets on one port with
/// ambiguous accept — see `wildcard_vs_loopback_conflict_is_platform_dependent`
/// and `wildcard_then_specific_conflict_is_platform_dependent` below.)
/// [reagent #3021 P0]
pub const STARTUP_BIND_ADDR: &str = "127.0.0.1:0";

pub fn lan_bind_addresses() -> Vec<std::net::IpAddr> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        tracing::warn!("could not enumerate network interfaces; no LAN listeners will bind");
        return Vec::new();
    };
    let mut out: Vec<std::net::IpAddr> = ifaces
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| match ip {
            std::net::IpAddr::V4(_) => true,
            // See doc comment: link-local v6 needs a scope id to be dialable.
            std::net::IpAddr::V6(v6) => !v6.is_unicast_link_local(),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Best-effort discovery of this host's *primary outbound* non-loopback IPv4
/// address.
///
/// Retained alongside [`lan_bind_addresses`] because it answers a different
/// question — "which single address routes outward" rather than "every address
/// peers might reach us on". The supervisor uses the latter; this is what the
/// tests use to pick one definitely-real, definitely-routable LAN address.
///
/// Uses the standard "connect a UDP socket and read back its local address"
/// trick: `connect` on UDP sends no packets, it only fixes the socket's peer so
/// the kernel commits to an outbound interface.
///
/// Returns `None` when there is no usable route (offline, or a loopback-only CI
/// container) — callers must treat that as "no LAN address today", not an error.
#[allow(dead_code)]
pub fn primary_lan_ipv4() -> Option<std::net::IpAddr> {
    // 203.0.113.0/24 is TEST-NET-3 (RFC 5737) — reserved for documentation and
    // guaranteed never routable, so this cannot accidentally reach a real
    // host. No packet is sent either way; only the routing decision matters.
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("203.0.113.1:80").ok()?;
    let addr = sock.local_addr().ok()?.ip();
    if addr.is_loopback() || addr.is_unspecified() {
        return None;
    }
    Some(addr)
}

/// Interval for the periodic reconcile sweep — see the supervisor's
/// "self-healing" note for why polling rather than OS interface events.
const RECONCILE_INTERVAL_SECS: u64 = 20;

/// Owns the LAN-facing listeners and keeps them reconciled against two inputs:
/// the `network:lan_discovery` setting, and the host's *current* interface set.
///
/// **Additive, never rebinding.** The loopback listeners created at startup are
/// never touched, so the frontend's own WebSocket is never dropped by a toggle.
/// LAN reachability is added by binding the SAME ports on specific non-loopback
/// addresses — verified possible by `loopback_and_specific_ip_can_share_a_port`
/// — and removed by cancelling those listeners. Ports stay stable, which
/// matters because they are already published in the mDNS TXT record and
/// `authkey.dev`.
///
/// **Self-healing.** A periodic sweep re-reconciles against the live interface
/// list, so a DHCP renewal, Wi-Fi↔Ethernet handoff, or VPN coming up is picked
/// up without a toggle or a restart. Polling rather than OS interface-change
/// events is a deliberate trade: no new platform-specific dependency, and the
/// worst case is `RECONCILE_INTERVAL_SECS` of staleness on a path that is
/// already best-effort peer discovery.
///
/// **Partial failure is survivable.** One address failing to bind (port taken,
/// permission denied, link dropped mid-bind) is logged and skipped; the others
/// still come up. Contrast the startup path, whose `.expect()` is fine at boot
/// and would be unacceptable for a runtime toggle.
pub struct LanListenerSupervisor {
    /// Filled by `main.rs` once the router exists — `build_router` consumes
    /// `AppState`, which owns this supervisor, so the router cannot be
    /// available at construction time.
    router: std::sync::OnceLock<axum::Router>,
    web_port: u16,
    ws_port: u16,
    /// Currently-bound LAN addresses → their shutdown handle. `std::sync::Mutex`
    /// (not tokio's) because every critical section is a short map mutation
    /// with no `.await` inside.
    active: std::sync::Mutex<
        std::collections::HashMap<std::net::IpAddr, tokio_util::sync::CancellationToken>,
    >,
    /// Last value passed to `apply`, so the periodic sweep knows which way it
    /// should be converging.
    enabled: std::sync::atomic::AtomicBool,
    /// The mDNS controller, so advertising can be gated on actually being
    /// reachable. Optional only because the two are constructed separately;
    /// in production it is always set.
    discovery: std::sync::OnceLock<std::sync::Arc<super::lan_discovery::LanDiscoveryController>>,
}

impl LanListenerSupervisor {
    pub fn new(web_port: u16, ws_port: u16) -> Self {
        Self {
            router: std::sync::OnceLock::new(),
            web_port,
            ws_port,
            active: std::sync::Mutex::new(std::collections::HashMap::new()),
            enabled: std::sync::atomic::AtomicBool::new(false),
            discovery: std::sync::OnceLock::new(),
        }
    }

    /// Give the supervisor the mDNS controller so it can keep advertising in
    /// step with reachability (see `reconcile`). Called once at startup.
    pub fn set_discovery(
        &self,
        discovery: std::sync::Arc<super::lan_discovery::LanDiscoveryController>,
    ) {
        let _ = self.discovery.set(discovery);
    }

    /// Hand the supervisor the router. Called once from `main.rs` after
    /// `build_router`. Until this lands, `apply(true)` logs and defers — there
    /// is nothing to serve yet, and the next sweep picks it up.
    pub fn set_router(&self, router: axum::Router) {
        let _ = self.router.set(router);
    }

    /// Idempotent live toggle, mirroring `LanDiscoveryController::apply` so both
    /// can be driven from one call site and cannot drift apart.
    pub fn apply(&self, enabled: bool) {
        self.enabled.store(enabled, std::sync::atomic::Ordering::SeqCst);
        self.reconcile();
    }

    /// True when at least one LAN listener is currently bound.
    ///
    /// mDNS advertising should be gated on this: advertising while nothing is
    /// listening is exactly the "discovered but unreachable" state this module
    /// exists to eliminate.
    pub fn has_lan_listener(&self) -> bool {
        self.active.lock().map(|a| !a.is_empty()).unwrap_or(false)
    }

    /// Converge the bound set toward what the setting and current interfaces
    /// say it should be. Safe to call repeatedly; a no-op when already correct.
    pub fn reconcile(&self) {
        let enabled = self.enabled.load(std::sync::atomic::Ordering::SeqCst);
        let desired: std::collections::HashSet<std::net::IpAddr> = if enabled {
            lan_bind_addresses().into_iter().collect()
        } else {
            std::collections::HashSet::new()
        };

        let Ok(mut active) = self.active.lock() else {
            tracing::warn!("LAN listener map poisoned; skipping reconcile");
            return;
        };

        // Drop listeners no longer wanted — toggled off, or the interface went
        // away. Cancelling triggers axum's graceful shutdown.
        let stale: Vec<std::net::IpAddr> = active
            .keys()
            .filter(|ip| !desired.contains(*ip))
            .copied()
            .collect();
        for ip in stale {
            if let Some(token) = active.remove(&ip) {
                token.cancel();
                tracing::info!(%ip, "LAN listener stopped");
            }
        }

        // Both early exits below still have to re-sync advertising: on the
        // disable path that is precisely what stops us advertising, and on the
        // no-router path it stops us claiming reachability we can't deliver.
        if !enabled {
            drop(active);
            self.sync_advertising();
            return;
        }
        let Some(router) = self.router.get() else {
            tracing::warn!("LAN enabled before the router existed; will bind on the next sweep");
            drop(active);
            self.sync_advertising();
            return;
        };

        for ip in desired {
            if active.contains_key(&ip) {
                continue;
            }
            let token = tokio_util::sync::CancellationToken::new();
            // Both ports must come up for an address to be useful: a peer needs
            // the web port to find us and the ws port to talk. Bind as a unit
            // and roll back if either fails, rather than registering a
            // half-usable address as active.
            match Self::spawn_pair(router.clone(), ip, self.web_port, self.ws_port, token.clone())
            {
                Ok(()) => {
                    active.insert(ip, token);
                    tracing::info!(%ip, web = self.web_port, ws = self.ws_port, "LAN listener started");
                }
                Err(e) => {
                    token.cancel();
                    tracing::warn!(%ip, error = %e, "could not bind LAN listener; skipping this address");
                }
            }
        }
        drop(active);
        self.sync_advertising();
    }

    /// Keep mDNS advertising in step with actual reachability.
    ///
    /// Advertising while nothing is listening is the "discovered but
    /// unreachable" state this module exists to eliminate — a peer finds us,
    /// dials, and gets connection-refused. It is strictly worse than not
    /// advertising, because the peer has no way to tell the difference from a
    /// transient failure and will keep retrying.
    ///
    /// So the rule is: advertise only when the setting is on AND at least one
    /// LAN listener is actually bound. Called from `reconcile`, which means the
    /// periodic sweep re-evaluates it too — if every interface goes away we
    /// stop advertising, and when one returns we start again, with no toggle.
    fn sync_advertising(&self) {
        let Some(discovery) = self.discovery.get() else {
            return;
        };
        let enabled = self.enabled.load(std::sync::atomic::Ordering::SeqCst);
        let reachable = self.has_lan_listener();
        // `apply` is idempotent, so this is a no-op whenever nothing changed.
        discovery.apply(enabled && reachable);
    }

    /// Bind + serve both ports for one address. Binding happens synchronously
    /// via `std::net` before anything is spawned, so a failure is returned to
    /// the caller instead of vanishing inside a task — that is what makes
    /// per-address rollback possible.
    fn spawn_pair(
        router: axum::Router,
        ip: std::net::IpAddr,
        web_port: u16,
        ws_port: u16,
        token: tokio_util::sync::CancellationToken,
    ) -> std::io::Result<()> {
        let web = Self::bind_std(ip, web_port)?;
        let ws = Self::bind_std(ip, ws_port)?;
        Self::serve(router.clone(), web, token.clone());
        Self::serve(router, ws, token);
        Ok(())
    }

    fn bind_std(ip: std::net::IpAddr, port: u16) -> std::io::Result<std::net::TcpListener> {
        let l = std::net::TcpListener::bind((ip, port))?;
        l.set_nonblocking(true)?;
        Ok(l)
    }

    fn serve(
        router: axum::Router,
        std_listener: std::net::TcpListener,
        token: tokio_util::sync::CancellationToken,
    ) {
        tokio::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else {
                tracing::warn!("could not adopt LAN listener into the tokio runtime");
                return;
            };
            let served = axum::serve(listener, router)
                .with_graceful_shutdown(async move { token.cancelled().await })
                .await;
            if let Err(e) = served {
                tracing::warn!(error = %e, "LAN listener exited with an error");
            }
        });
    }

    /// Periodic self-healing sweep. Spawned once at startup, runs for the
    /// process lifetime, and is a cheap no-op whenever nothing has changed.
    pub fn spawn_reconcile_loop(self: std::sync::Arc<Self>) {
        tokio::spawn(async move {
            let mut tick =
                tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                self.reconcile();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// **The one assumption the LAN-listener design rests on:** a specific
    /// non-loopback address can be bound on a port already held by loopback.
    ///
    /// That is what lets us become LAN-reachable by ADDING a listener, rather
    /// than rebinding the live server (which would drop the frontend's own
    /// loopback WebSocket) or changing the port (which would invalidate the
    /// mDNS TXT record and `authkey.dev`).
    ///
    /// Skips (rather than fails) when the host has no non-loopback address —
    /// an offline machine or loopback-only CI container can't exercise it, and
    /// failing there would be noise, not signal.
    #[test]
    fn loopback_and_specific_ip_can_share_a_port() {
        let Some(lan_ip) = primary_lan_ipv4() else {
            eprintln!("skipping: no non-loopback IPv4 on this host");
            return;
        };

        // Hold loopback on an ephemeral port, exactly as srv does today when
        // LAN discovery is off.
        let loopback = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = loopback.local_addr().unwrap().port();

        let lan = TcpListener::bind((lan_ip, port));
        assert!(
            lan.is_ok(),
            "binding {lan_ip}:{port} alongside 127.0.0.1:{port} must succeed \
             — the add-a-listener design depends on it; got {:?}",
            lan.err()
        );
    }

    /// Records a **platform difference**, deliberately without asserting a
    /// universal rule — measured, 2026-09-06.
    ///
    /// The design docs originally claimed `0.0.0.0:PORT` universally conflicts
    /// with an existing `127.0.0.1:PORT`. On Linux/macOS it does. **On Windows
    /// it does not** — absent `SO_EXCLUSIVEADDRUSE`, the wildcard bind
    /// succeeds alongside the loopback one. This test was written asserting
    /// the conflict and failed on Windows, which is how the claim was caught
    /// before it reached an implementation.
    ///
    /// Why it matters: it rules out "just bind the wildcard on toggle" as a
    /// portable shortcut. On Windows two sockets would hold the same port with
    /// ambiguous accept behaviour — precisely the port-hijack hazard
    /// `SO_EXCLUSIVEADDRUSE` exists to prevent. Binding SPECIFIC addresses (the
    /// chosen design) has no such ambiguity on any platform.
    ///
    /// Asserts nothing about which way it goes; it exists to keep the observed
    /// behaviour visible and to fail loudly if it ever becomes uniform.
    #[test]
    fn wildcard_vs_loopback_conflict_is_platform_dependent() {
        let loopback = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = loopback.local_addr().unwrap().port();
        let wildcard = TcpListener::bind(("0.0.0.0".parse::<std::net::IpAddr>().unwrap(), port));

        let conflicts = wildcard.is_err();
        eprintln!(
            "0.0.0.0:{port} vs existing 127.0.0.1:{port} — conflicts: {conflicts} \
             (expected: true on Linux/macOS, false on Windows)"
        );

        #[cfg(windows)]
        assert!(
            !conflicts,
            "Windows was measured as ALLOWING the wildcard bind alongside \
             loopback; if it now conflicts, the platform note in \
             REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md \
             is stale"
        );
        #[cfg(not(windows))]
        assert!(
            conflicts,
            "expected the wildcard bind to conflict with loopback on this \
             platform; if it no longer does, the same report's platform note \
             is stale"
        );
    }

    /// The **reverse order** of the test above, and the one that actually bit:
    /// a wildcard socket bound FIRST, then the per-address bind this supervisor
    /// performs. This is the boot sequence that existed when startup bound
    /// `0.0.0.0:0` for a user who already had `network:lan_discovery` on.
    ///
    /// On Linux/macOS the per-address bind fails, so `reconcile` binds nothing,
    /// `has_lan_listener()` stays false, and `sync_advertising` calls
    /// `discovery.apply(false)` — turning OFF the mDNS the user had enabled.
    /// A silent LAN-disable on every restart, for exactly the opted-in users.
    /// On Windows the bind succeeds and you instead get two sockets on one port.
    ///
    /// Neither outcome is acceptable, which is why [`STARTUP_BIND_ADDR`] is
    /// loopback unconditionally. Like its sibling, this asserts the *measured*
    /// per-platform behaviour so a change in either direction is visible.
    #[test]
    fn wildcard_then_specific_conflict_is_platform_dependent() {
        let Some(lan_ip) = primary_lan_ipv4() else {
            eprintln!("skipping: no non-loopback IPv4 on this host");
            return;
        };

        let wildcard = TcpListener::bind("0.0.0.0:0").expect("bind wildcard");
        let port = wildcard.local_addr().unwrap().port();
        let specific = TcpListener::bind((lan_ip, port));

        let conflicts = specific.is_err();
        eprintln!(
            "{lan_ip}:{port} vs existing 0.0.0.0:{port} — conflicts: {conflicts} \
             (expected: true on Linux/macOS, false on Windows)"
        );

        #[cfg(windows)]
        assert!(
            !conflicts,
            "Windows was measured as ALLOWING the per-address bind under a \
             wildcard; if it now conflicts, STARTUP_BIND_ADDR's rationale needs \
             re-checking (the fix is still correct either way)"
        );
        #[cfg(not(windows))]
        assert!(
            conflicts,
            "expected the per-address bind to be refused under a wildcard on \
             this platform — this conflict is the whole reason startup binds \
             loopback only; got {:?}",
            specific.err()
        );
    }

    /// Guards the fix itself: startup must never go back to a wildcard bind.
    ///
    /// Cheap, but it is the assertion that would have caught the original
    /// regression — `bind_listeners_and_network` now takes its address from
    /// this constant rather than branching on the setting.
    #[test]
    fn startup_bind_is_loopback_only() {
        let addr: std::net::SocketAddr = STARTUP_BIND_ADDR.parse().expect("parses as a SocketAddr");
        assert!(
            addr.ip().is_loopback(),
            "startup must bind loopback so the supervisor solely owns LAN \
             sockets; got {addr}"
        );
        assert_eq!(addr.port(), 0, "startup port must stay ephemeral");
    }

    /// `primary_lan_ipv4` must never hand back something unusable as a bind
    /// target — loopback or unspecified would silently produce a listener
    /// that adds no LAN reachability at all.
    #[test]
    fn primary_lan_ipv4_is_never_loopback_or_unspecified() {
        if let Some(ip) = primary_lan_ipv4() {
            assert!(!ip.is_loopback(), "returned loopback: {ip}");
            assert!(!ip.is_unspecified(), "returned unspecified: {ip}");
        }
    }
}

#[cfg(test)]
mod supervisor_tests {
    use super::*;

    fn router() -> axum::Router {
        axum::Router::new().route("/ping", axum::routing::get(|| async { "pong" }))
    }

    /// Two free ports to stand in for the real web/ws pair. Bound and dropped
    /// so the numbers are almost certainly free — good enough for a test, and
    /// the supervisor tolerates a bind failure anyway.
    fn free_port_pair() -> (u16, u16) {
        let a = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let b = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        (a.local_addr().unwrap().port(), b.local_addr().unwrap().port())
    }

    /// Disabled is the default, and must bind nothing at all — the OS
    /// permission prompt is only acceptable when the user opted in.
    #[tokio::test]
    async fn binds_nothing_until_enabled() {
        let (w, s) = free_port_pair();
        let sup = LanListenerSupervisor::new(w, s);
        sup.set_router(router());
        sup.reconcile();
        assert!(!sup.has_lan_listener(), "must not bind before being enabled");
    }

    /// `apply(true)` before `set_router` must not panic or wedge — startup
    /// ordering isn't guaranteed, and the sweep is expected to recover.
    #[tokio::test]
    async fn enabling_before_the_router_exists_is_safe() {
        let (w, s) = free_port_pair();
        let sup = LanListenerSupervisor::new(w, s);
        sup.apply(true); // no router yet
        assert!(!sup.has_lan_listener(), "cannot serve without a router");
        // …and it recovers once the router arrives, without another apply().
        sup.set_router(router());
        sup.reconcile();
        if lan_bind_addresses().is_empty() {
            eprintln!("skipping bind assertion: host has no non-loopback address");
            return;
        }
        assert!(sup.has_lan_listener(), "sweep should bind once the router exists");
    }

    /// The toggle must work in BOTH directions without a restart — disabling
    /// previously left listeners on 0.0.0.0, still LAN-reachable, which is the
    /// exposure half of the original bug.
    #[tokio::test]
    async fn toggling_on_then_off_binds_then_releases() {
        if lan_bind_addresses().is_empty() {
            eprintln!("skipping: host has no non-loopback address");
            return;
        }
        let (w, s) = free_port_pair();
        let sup = LanListenerSupervisor::new(w, s);
        sup.set_router(router());

        sup.apply(true);
        assert!(sup.has_lan_listener(), "enabling should bind at least one address");

        sup.apply(false);
        assert!(
            !sup.has_lan_listener(),
            "disabling must release every LAN listener, not leave them bound until restart"
        );
    }

    /// `apply` is called unconditionally from the setconfig path (the fs
    /// watcher can re-fire the same value), so repeats must be no-ops rather
    /// than churning listeners.
    #[tokio::test]
    async fn apply_is_idempotent() {
        if lan_bind_addresses().is_empty() {
            eprintln!("skipping: host has no non-loopback address");
            return;
        }
        let (w, s) = free_port_pair();
        let sup = LanListenerSupervisor::new(w, s);
        sup.set_router(router());

        sup.apply(true);
        let first = sup.active.lock().unwrap().len();
        sup.apply(true);
        sup.reconcile();
        let second = sup.active.lock().unwrap().len();
        assert_eq!(first, second, "repeat applies must not add or drop listeners");

        sup.apply(false);
        sup.apply(false);
        assert!(!sup.has_lan_listener());
    }

    /// A LAN listener must actually serve the router — binding a socket that
    /// answers nothing would reproduce "discovered but unreachable" with extra
    /// steps.
    #[tokio::test]
    async fn a_bound_listener_actually_serves_requests() {
        let Some(ip) = primary_lan_ipv4() else {
            eprintln!("skipping: host has no non-loopback IPv4");
            return;
        };
        let (w, s) = free_port_pair();
        let sup = LanListenerSupervisor::new(w, s);
        sup.set_router(router());
        sup.apply(true);
        assert!(sup.has_lan_listener());

        // Give the spawned serve tasks a moment to begin accepting.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let body = reqwest::Client::new()
            .get(format!("http://{ip}:{w}/ping"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .expect("LAN listener should accept a connection on the bound address")
            .text()
            .await
            .unwrap();
        assert_eq!(body, "pong", "the LAN listener must serve the same router");

        sup.apply(false);
    }

    /// Excluding loopback matters: it is already bound by the startup path, so
    /// including it here would guarantee an address-in-use failure every time.
    #[test]
    fn bind_addresses_exclude_loopback_and_v6_link_local() {
        for ip in lan_bind_addresses() {
            assert!(!ip.is_loopback(), "loopback must be excluded: {ip}");
            if let std::net::IpAddr::V6(v6) = ip {
                assert!(
                    !v6.is_unicast_link_local(),
                    "v6 link-local needs a scope id to be dialable: {v6}"
                );
            }
        }
    }
}
