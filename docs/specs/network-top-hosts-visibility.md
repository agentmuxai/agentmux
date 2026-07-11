# Spec: Per-Host Network Visibility (Top External Hosts + Data Transferred)

**Status:** Proposed
**Date:** 2026-07-10

## Problem

The status bar's network readout (`↑1.2M ↓340K`) only shows an aggregate rate summed across
every network interface. It answers "is the machine sending/receiving data right now" but not
"to/from whom." There's no way to see which external hosts AgentMux's shell/agent processes (or
anything else on the machine) are talking to, or how much data each has moved.

This spec covers: (1) whether that data is actually obtainable on Windows/macOS/Linux, and
(2) if so, how to collect, bound, and surface it without adding jank or unbounded memory growth.

## Current State (for reference)

- `agentmux-srv/src/backend/sysinfo.rs:91-133` (`NetState::get_net_data`) sums
  `Networks::iter().total_transmitted()/total_received()` across **all interfaces**, diffs
  against the previous tick, and emits `net:bytessent` / `net:bytesrecv` (MB/s) into the generic
  `sysinfo` WPS event (`EVENT_SYS_INFO`, `sysinfo.rs:220-230`).
- Sampling runs on a `tokio::time::interval` loop, default 1s, configurable 0.2–2.0s via
  `telemetry:interval` (`wconfig/types.rs:195-196`, clamped in `sysinfo.rs:26,154-160`).
- Frontend: `frontend/app/statusbar/SystemStats.tsx:93-115` subscribes to the `sysinfo` event and
  renders the `↑/↓` readout (`SystemStats.tsx:178-187`). No per-host or per-process breakdown
  exists anywhere in the frontend or backend.
- The `sysinfo` crate (v0.34, the only network-facing dependency in the workspace) exposes
  network counters **per interface only** — there is no per-process or per-connection API on
  `sysinfo::Networks` or `sysinfo::Process`.

## Research: is per-host byte data available?

Short answer: **host identity, yes — exact bytes-per-host, not without elevated privileges or a
bundled driver, on any of the three target platforms.** This matches how comparable tools are
actually built (see below), and it drives the two-tier design in this spec.

### What's cheaply available (no elevation, works today)

Every OS exposes a live **socket/connection table**: local/remote address+port, protocol, state,
and (on all three platforms) the owning PID — without admin rights:

- **Windows:** `GetExtendedTcpTable` / `GetExtendedUdpTable` (iphlpapi) — same API `netstat -b`
  uses; no elevation required.
- **Linux:** `/proc/net/tcp[6]` / `/proc/net/udp[6]` (world-readable) or the `NETLINK_INET_DIAG`
  socket.
- **macOS:** `sysctl net.inet.tcp.pcblist` / `net.inet.udp.pcblist` (same mechanism the `netstat`
  CLI uses as a normal user).

The Rust crate **[`netstat2`](https://github.com/ohadravid/netstat2-rs)** wraps all three behind
one API (`get_sockets_info(AddressFamilyFlags, ProtocolFlags) -> Vec<SocketInfo>`, giving
`local_addr`, `remote_addr`, `remote_port`, `associated_pids`, `state`) and needs no elevation on
any platform. This is enough to answer "which remote IPs is this machine currently talking to,
and via which process" — but it's a **point-in-time snapshot**, not a byte counter. There is no
cumulative "bytes transferred on this socket" field in any of these APIs.

### What exact byte-per-connection accounting actually requires

To get real byte counts per connection/host, every platform bottoms out in one of two things,
and both require privilege escalation:

1. **Windows — `GetPerTcpConnectionEStats`/`SetPerTcpConnectionEStats` (iphlpapi) or an ETW trace
   on `Microsoft-Windows-TCPIP`.** `SetPerTcpConnectionEStats` "can only be called by a user
   logged on as a member of the Administrators group" and additionally needs a manifest with
   `requireAdministrator` to survive UAC token filtering ([MS docs](https://learn.microsoft.com/en-us/windows/win32/api/iphlpapi/nf-iphlpapi-setpertcpconnectionestats)).
   Starting an ETW trace session has the same requirement (admin, or pre-provisioned membership
   in the "Performance Log Users" group — itself an admin-configured exception, not a bypass).
   **Windows Task Manager's per-process network column does not use either path** — it reads
   pre-aggregated stats from **NDU** (Network Data Usage), a standing SYSTEM-level kernel driver
   that's always running and exposes results through a private interface. That's not a documented
   public API and isn't a viable integration point for us.
2. **Linux — packet capture (`AF_PACKET`/libpcap) or `nf_conntrack` accounting.** Raw capture
   needs `CAP_NET_RAW` (root or a `setcap` grant on the binary). `nf_conntrack` byte counters
   (`/proc/net/nf_conntrack`) require the conntrack module loaded and accounting enabled
   (`net.netfilter.nf_conntrack_acct=1`), which isn't guaranteed present, and reading the full
   table is typically root-only in practice.
3. **macOS — BPF device access (`/dev/bpf*`)**, historically root-only; even with an entitlement
   it needs a privileged helper.

This is also why every real per-app bandwidth tool in this space is built the way it is:
**GlassWire** and **NetLimiter**-class tools ship a signed kernel driver / WFP callout for exact
attribution; **Little Snitch** intercepts at the socket layer (macOS) or via an eBPF kernel hook
(its newer Linux port) — again privileged. **nethogs**/**iftop**-style tools capture packets and
need `CAP_NET_RAW`. There is no OS-documented, unprivileged path to exact per-host byte counts on
any platform. Shipping a kernel driver or requiring elevation is a large scope/support-burden
increase in trade for one status-bar widget, so this spec does not propose it for v1.

### Conclusion / recommendation

Ship a **v1 that's honestly an estimate**, gated behind an explicit opt-in, using only
unprivileged APIs already reachable via `netstat2`. Treat exact byte-accurate capture as a
possible **v2, opt-in and privileged**, not part of this spec's implementation.

## Proposed Design — v1 (no elevation required)

### Architecture

Add a second, slower collection loop alongside the existing `run_sysinfo_loop`, rather than
folding this into the 0.2–2s hot loop — connection-table enumeration + DNS resolution is heavier
and lower-value at sub-second resolution than CPU/mem:

```
agentmux-srv/src/backend/net_hosts.rs   (new)
```

```rust
pub async fn run_net_hosts_loop(broker: Arc<Broker>, config_watcher: Arc<ConfigWatcher>, conn_name: String) {
    // gated: only runs if config_watcher.get_settings().network_tophosts_enabled
    let mut tracker = HostTracker::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(3)); // fixed, not user-tunable in v1
    loop {
        ticker.tick().await;
        if !config_watcher.get_settings().network_tophosts_enabled { continue; }

        let sockets = tokio::task::block_in_place(|| {
            netstat2::get_sockets_info(AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6, ProtocolFlags::TCP | ProtocolFlags::UDP)
        }).unwrap_or_default();

        tracker.observe(sockets, /* aggregate delta bytes from the existing NetState */ delta_sent, delta_recv);
        if let Some(snapshot) = tracker.snapshot_if_changed() {
            broker.publish(make_event(snapshot, &conn_name));
        }
    }
}
```

### Per-connection → per-host attribution (the estimation)

There's no per-connection byte counter, so bytes are **apportioned**, not measured:

1. Each tick, snapshot active sockets via `netstat2`. Drop loopback/link-local/private RFC1918
   ranges (this is "external hosts" — local dev servers, Docker bridges, etc. are noise).
2. Take the aggregate interface byte delta for the tick (already computed by the existing
   `NetState` in `sysinfo.rs` — reuse it rather than re-summing `Networks`).
3. Split that delta across the distinct remote IPs seen active in the tick, weighted by each
   host's share of *observed active connections* (a host with 3 concurrent connections gets 3x
   the share of a host with 1). This is a heuristic, not a measurement — label it as such in the
   UI ("~2.1 MB, estimated").
4. Accumulate per-host running totals across ticks in the bounded structure below.

This mirrors what's actually achievable without capture: connection-table sampling is the same
technique `nethogs` falls back to when it can't get `CAP_NET_RAW`, and it's the right fidelity
for a glanceable status-bar widget, not a billing system.

### Efficient management (bounding memory & cost — the actual ask)

A long-running session can see thousands of distinct remote IPs (CDN edge rotation, ephemeral
SaaS endpoints); naively keyed by-IP state grows unbounded. Bound it explicitly:

- **Bounded top-K via Space-Saving / Misra-Gries**, not a growing `HashMap<IpAddr, Stats>`. Track
  only `k ≈ 32` counters; when a new host arrives and the table is full, evict the entry with the
  smallest counter (standard Space-Saving replacement rule — O(k) memory regardless of how many
  distinct hosts are ever seen, bounded additive error on the evicted tail). This directly answers
  "how is this efficiently managed" — no unbounded per-IP table, ever.
- **Group by hostname's registrable domain (eTLD+1), not raw IP.** A CDN-backed service (GitHub,
  npm, Anthropic API, etc.) rotates through many IPs for one logical host; keying by IP fragments
  one real "top host" into a dozen table slots and starves the actual top-K of space. Resolve IP
  → hostname, reduce to eTLD+1 (e.g. `objects.githubusercontent.com` → `githubusercontent.com`),
  and key the Space-Saving table on that string.
- **Reverse DNS is the expensive part — cache and rate-limit it, keep it off the poll thread.**
  Use an async resolver (`hickory-resolver` — no async reverse-DNS/PTR crate is in the workspace
  today; `mdns-sd` in `agentmux-srv/Cargo.toml:45` is mDNS/LAN-discovery only and unrelated) with:
  - An LRU cache, bounded (~500 entries), TTL 10–15 minutes, so a host hammering the same IP
    doesn't re-resolve every tick.
  - Resolution dispatched via `tokio::spawn`, never awaited inline in the poll loop — a slow or
    unresponsive resolver must not stall the socket-table snapshot. Unresolved IPs display as the
    raw IP until (if ever) resolution completes.
  - No resolution for private/loopback ranges (filtered out in step 1 above anyway).
- **Separate, slower cadence from CPU/mem (3–5s fixed, not tied to `telemetry:interval`).**
  Socket-table enumeration + potential DNS work is heavier than the sysinfo counter reads and
  doesn't benefit from sub-second resolution the way CPU spikes do.
- **No unbounded history.** Keep only the current in-memory top-K snapshot plus first-seen/
  last-seen timestamps per entry (evicted entries' history is simply gone — consistent with the
  approximate nature of the feature). No disk persistence in v1; if a "history" view is wanted
  later, gate it behind its own opt-in and a size-capped rotated log, matching the pattern the
  existing `telemetry:*` settings already use for optional, bounded telemetry.
- **Publish deltas, not every tick.** Only emit a new WPS event when the top-K ordering or a
  host's rounded MB total actually changes (cheap hash/compare against the last published
  snapshot) — avoids flooding the WS with an unchanged 32-entry list every 3s. Reuse the existing
  `Lane::Background` priority (`eventbus.rs:263`) so this never competes with interactive traffic,
  same as the current `sysinfo`/`blockstats` events.

### Data model

```rust
// agentmux-srv/src/backend/rpc_types/misc.rs (extend)
pub struct HostEntry {
    pub host: String,        // resolved eTLD+1, or raw IP if unresolved
    pub est_bytes_sent: f64,
    pub est_bytes_recv: f64,
    pub connections: u32,    // active connection count, last tick
    pub first_seen: i64,     // epoch ms
    pub last_seen: i64,
}

pub struct TopHostsSnapshot {
    pub ts: i64,
    pub hosts: Vec<HostEntry>, // sorted desc by (sent+recv), len <= k
    pub estimated: bool,       // always true in v1 — surfaced verbatim to the UI
}
```

New WPS event constant `EVENT_NET_HOSTS = "net:tophosts"` (alongside `EVENT_SYS_INFO` /
`EVENT_BLOCK_STATS` in `agentmux-srv/src/backend/wps.rs`), scoped the same way as `sysinfo`
(`scopes: vec![conn_name]`).

### Config

| Key | Type | Default | Notes |
|-----|------|---------|-------|
| `network:tophosts:enabled` | bool | `false` | Opt-in — see Privacy below |
| `network:tophosts:topk` | int | 32 | Space-Saving table size; min 8, max 100 |

Follows the existing `SettingsType` pattern (`wconfig/types.rs:192-199`):
```rust
#[serde(rename = "network:tophosts:enabled", default, skip_serializing_if = "is_false")]
pub network_tophosts_enabled: bool,

#[serde(rename = "network:tophosts:topk", default, skip_serializing_if = "is_zero_i64")]
pub network_tophosts_topk: i64,
```

### Frontend

Extend the existing network readout in `SystemStats.tsx` with the same click-to-popover pattern
already used for CPU (`CpuCoresPopover`, `SystemStats.tsx:55-69,134-141`):

- Make the `stat-net` span (`SystemStats.tsx:178-187`) a `<button>` like `stat-cpu-button`, opening
  a new `TopHostsPopover.tsx` (modeled directly on `CpuCoresPopover`).
- Popover lists top N hosts: hostname, small horizontal share bar, `↑X ↓Y` estimated totals,
  connection count. Header note: "Estimated — approximated from connection activity, not exact
  packet counts" so users don't mistake this for billing-grade accounting.
- If `network:tophosts:enabled` is `false`, clicking shows a one-line explainer with a link/toggle
  to enable it (consistent with an opt-in feature rather than silently doing nothing).
- Subscribe to the new `net:tophosts` WPS event the same way `waveEventSubscribe` is used today
  (`SystemStats.tsx:93-115`); add `WpsEvent.NetTopHosts` to `frontend/app/store/wps-events.ts`.

### Privacy & security

Resolved external hostnames are effectively a browsing/activity log. Default **off**
(`network:tophosts:enabled: false`), require an explicit toggle (Settings UI + `settings.json`),
and document what it does and doesn't do (no payload inspection, connection-table + estimated
byte split only, nothing persisted to disk in v1). This mirrors how `telemetry:enabled` is already
handled as an explicit opt-in in this codebase.

## Out of scope for v1 (future work only)

- **Exact byte-accurate accounting** via a bundled Npcap-based capture (Windows), `CAP_NET_RAW` +
  `setcap` (Linux), or a privileged helper (macOS). Would need its own spec covering installer
  changes, code-signing/notarization impact, and a broker-process split on Windows (a single
  process can't hold a split elevated/non-elevated token, per Microsoft's guidance — a non-
  elevated main process would need a separate elevated helper over an IPC channel).
- **Per-process attribution** (which AgentMux pane/agent talked to which host). `netstat2`'s
  `associated_pids` field makes this possible to layer on top of the same connection-table poll
  later, joined against the existing `pidregistry`/`process_tree` machinery already used for
  per-block CPU/mem in `sysinfo.rs:232-332` — noted here as the natural extension point, not
  built in this spec.
- **Persisted history / longer-range charts** — v1 is live-session-only, in-memory, bounded.

## Files Changed

| File | Change |
|------|--------|
| `agentmux-srv/Cargo.toml` | Add `netstat2`, `hickory-resolver` deps |
| `agentmux-srv/src/backend/net_hosts.rs` | **New** — `HostTracker` (Space-Saving top-K), poll loop |
| `agentmux-srv/src/backend/wps.rs` | Add `EVENT_NET_HOSTS` constant |
| `agentmux-srv/src/backend/rpc_types/misc.rs` | Add `HostEntry` / `TopHostsSnapshot` |
| `agentmux-srv/src/backend/wconfig/types.rs` | Add `network:tophosts:enabled` / `:topk` settings |
| `agentmux-srv/src/main.rs` | Spawn `run_net_hosts_loop` alongside `run_sysinfo_loop` |
| `frontend/app/store/wps-events.ts` | Add `NetTopHosts` event type |
| `frontend/app/statusbar/TopHostsPopover.tsx` | **New** — modeled on `CpuCoresPopover.tsx` |
| `frontend/app/statusbar/SystemStats.tsx` | Make network readout clickable, wire popover |
| `schema/settings.json` | Add schema entries for the two new settings |

## Testing

1. With `network:tophosts:enabled: false` (default), no behavior change — no new loop work, no
   new WS events, clicking the network readout shows the opt-in explainer.
2. Enable the setting; open several connections to distinct external hosts (e.g. `curl` to a few
   different domains) and confirm they appear in the popover within one poll tick (~3s), grouped
   by eTLD+1 even when the same host resolves to multiple observed IPs across ticks.
3. Generate traffic to 50+ distinct hosts (e.g. loop `curl` across a domain list) and confirm the
   in-memory table never exceeds `topk` entries — verify via a debug log of table size, not just
   the UI.
4. Kill network connectivity: `est_bytes_*` stop advancing, existing entries age out of the
   "recent" display but the process doesn't panic on an empty socket table.
5. Confirm DNS resolution failures (unroutable IP, resolver timeout) fall back to displaying the
   raw IP and don't block the next poll tick.
6. Confirm the WS event is only republished when the snapshot actually changes (log/count
   published events over a quiet period).
