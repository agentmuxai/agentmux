# Report: Network system — architecture, DRYness, and a robust/high-performance LAN toggle

**Date:** 2026-09-06
**Status:** Draft — architecture review + design recommendation. Not implemented.
**Author:** Agent2
**Companion:** `REPORT_LAN_TOGGLE_WITHOUT_RESTART_2026_09_06.md` (current-state
analysis of the toggle bug). This report supersedes that one's §6
recommendation, which was written under an implicit
minimise-engineering-cost constraint the operator has since lifted:

> *"we want the most robust, high performance solution .. engineering cost is
> not important"*

---

## Part I — The robust design

### 1. Why the earlier recommendation isn't the right answer anymore

The companion report recommended binding extra per-interface listeners on
toggle. That is correct and cheap, but under a robustness objective it has two
weaknesses:

1. **Address churn.** It enumerates interfaces at toggle time. DHCP renewal,
   Wi-Fi↔Ethernet handoff, VPN up/down, and container adapters all change the
   set afterwards. This host alone has `192.168.1.230` plus WSL/Hyper-V
   virtuals (`172.23.176.1`, `192.168.116.1`) and link-local IPv6 — a snapshot
   is stale almost immediately on a laptop.
2. **It leaves the real latency problem untouched** (§3), which is the larger
   share of "the network feels broken."

### 2. Recommended: an interface-aware listener supervisor

A long-lived supervisor task owning the LAN-facing listener set, reconciling it
against two inputs: **the setting** and **the current interface list**.

```
LanListenerSupervisor
  inputs:  setting toggle (existing LanDiscoveryController::apply path)
           interface-change events (netlink / SCNetworkReachability /
                                    NotifyIpInterfaceChange — see `if-watch`)
  state:   HashMap<IpAddr, ListenerHandle>   // handle = JoinHandle + CancellationToken
  reconcile(desired: Set<IpAddr>):
      bind + axum::serve(router.clone()) for (desired − current)
      cancel graceful shutdown        for (current − desired)
```

Properties this buys, none of which the snapshot approach has:

- **Self-healing.** Plug in Ethernet, join a VPN, renew a lease → the listener
  set follows. No restart, no re-toggle.
- **Correct in both directions.** Disable → cancel every LAN listener, loopback
  untouched. This also closes the current ON→OFF exposure bug, where listeners
  stay on `0.0.0.0` and remain LAN-reachable after the user turns the feature
  off (`bootstrap.rs:1312-1319`).
- **Partial failure is survivable.** One interface failing to bind (in use,
  permission denied, link down mid-bind) degrades that address only. Contrast
  the startup path's `.expect("failed to bind web listener")`, which is fine at
  boot and unacceptable for a runtime toggle.
- **Port stability preserved.** Ports are ephemeral (`:0`, `bootstrap.rs:1321`)
  and already published in the mDNS TXT record and `authkey.dev`; every new
  listener reuses the *existing* port on a different address, so neither
  publication is ever invalidated.
- **Loopback is never interrupted.** The frontend's own WebSocket rides
  loopback; a rebind-based design (companion report's Option C) would drop it
  on every toggle.

**Spike done — assumption VERIFIED, and one stated claim was wrong.**
Measured 2026-09-06 by `backend::lan_listeners::tests`:

- ✅ **`127.0.0.1:PORT` + `<lan-ip>:PORT` bind simultaneously.** Confirmed on
  Windows; asserted in CI on Linux too. This is the claim the design rests on,
  and it holds.
- ❌ **"`0.0.0.0:PORT` conflicts with both" was wrong.** True on Linux/macOS;
  **false on Windows**, where absent `SO_EXCLUSIVEADDRUSE` the wildcard binds
  happily alongside an existing loopback listener. The spike was written
  asserting the conflict and *failed on Windows*, which is how the error was
  caught before any implementation depended on it.

The correction does not weaken the design — it strengthens the case for it.
"Just bind the wildcard on toggle" is now ruled out as a portable shortcut: on
Windows it would leave two sockets on one port with ambiguous accept
behaviour, exactly the hijack hazard `SO_EXCLUSIVEADDRUSE` exists to prevent.
Binding **specific** addresses has no such ambiguity anywhere.

The platform difference is pinned by
`wildcard_vs_loopback_conflict_is_platform_dependent`, which asserts the
observed behaviour per-platform so it fails loudly if either ever changes.

### 3. The bigger performance win: peer lookup is sequential

`LanDiscoveryController::find_agent` (`lan_discovery.rs:745-775`) resolves
"which peer hosts agent X" by querying peers **one at a time**:

```rust
for peer in &peers {
    let result = http.get(...).timeout(LAN_PEER_QUERY_TIMEOUT_SECS).send().await;
    if success { return Some(...) }
}
```

With `LAN_PEER_QUERY_TIMEOUT_SECS = 2` (`lan_discovery.rs:26`), worst case is
**2 × N seconds** before a message is delivered or falls through — 10s on five
peers, and the pathological case (dead/slow peers ordered before the right one)
is the *common* one on a laptop network where stale peers linger.

**Fix: concurrent fan-out, first-success-wins.** `FuturesUnordered` over all
peers, resolve on the first 2xx, drop the rest. Worst case becomes ~one timeout
regardless of peer count.

Worth doing carefully:

- **Don't lose the negative-cache behaviour.** The current code caches negative
  results too (`LAN_AGENT_CACHE_TTL_SECS = 60`) so cloud-only agents don't
  re-fan-out on every inject. Concurrency must preserve that.
- **Bound concurrency** if the peer set can get large, though on a home/office
  LAN it won't.
- **Cancellation on first win** matters — otherwise every peer is queried in
  full anyway and only the latency, not the load, improves.

This is a bigger perceived-performance win than the bind fix, and it is
independent of it — shippable separately.

---

## Part II — Architecture and DRYness

### 4. The delivery tier chain is copy-pasted three times

`handle_reactive_inject` (`reactive.rs`, 2410 lines) walks: local → same-host
same-channel → same-host cross-channel → LAN peer → (return error; cloud is the
caller's job). Tiers 2a, 2b and 3 each contain a near-identical block at
`reactive.rs:747`, `:876`, `:982`:

```
build forward_url  →  POST /agentmux/reactive/inject with X-AuthKey
                   →  check body.success
                   →  echo_jekt_to_sender(...11 args...)
                   →  on failure: log + evict + fall through
```

Three hand-maintained copies of the same control flow, differing only in how
the target URL and key are resolved. `echo_jekt_to_sender` is called from
**four** places with the same twelve-positional-argument shape
(`reactive.rs:49-62`).

**Why this matters beyond tidiness:** this is exactly the shape that produced
two separate bugs already documented this week — the provider auth-dir seeding
gap and the settings-seeding gap both came from two independently-maintained
copies of one operation drifting. A tier that forgets to call
`echo_jekt_to_sender`, or passes `delivery_tier` wrong, fails silently and
differently from its siblings.

**Refactor:** one `forward_to(target: ForwardTarget) -> Option<Response>`
helper, where `ForwardTarget { url, auth_key, tier_label, on_failure }`. The
tier chain becomes a list of resolvers feeding one forwarder. `echo_jekt_to_sender`'s
twelve positionals become a struct (several are `Option<bool>` in a row —
`sig_verified`, `reagent_verified`, `lan_verified` — which is a swap waiting to
happen; the compiler cannot help you there).

### 5. Tier 4 exists in the comment and nowhere in the code

`reactive.rs:1044`:

```rust
// 4. Return original error (muxbus-client will fall back to cloud relay).
```

But **the MCP `SendMessage` tool is not that client.** It POSTs to
`/agentmux/reactive/inject` and bails on `success != true`
(`agentmux-mcp/src/main.rs:2322-2330`). No cloud fallback anywhere on that
path — while the tool's own description promises *"tries local → LAN → cloud in
order."*

So the documented four-tier model is really three tiers plus a comment, and the
one caller most agents actually use silently stops at three. This is an
architecture gap, not just a missing feature: **the tier chain has no single
owner.** Parts live in `reactive.rs`, one part is delegated to an unnamed
"muxbus-client," and the MCP tool re-implements the entry point.

**Recommendation:** make the tier chain a single, complete, owned abstraction —
if cloud relay is tier 4, `handle_reactive_inject` should perform it, and every
caller inherits it. Failing that, the description must stop promising it.

### 6. Discovery reports local truth as if it were global

`DiscoverAgents` returns `wan.subscribed_agents`, which on inspection lists
**this host's own** agents that are cloud-subscribed — not remote agents
reachable via cloud. Verified: the list is exactly the local addressable set.

An agent reading that output reasonably concludes "the WAN tier can reach these
five agents," when it actually means "these five of mine are subscribed." There
is no way to ask "who is reachable via cloud?" This directly caused a wrong
diagnosis earlier today.

**Recommendation:** either rename the field to what it is
(`wan.local_agents_subscribed`) or make it answer the question its name
implies.

### 7. Two config-watching mechanisms

`wconfig::watcher` (`ConfigWatcher`) is a state holder with no filesystem
watching, despite the name; the actual watching is `config_watcher_fs.rs` on
top of the shared `fs_watch::pool`. Two modules, similar names, only one
watches. Minor, but it cost me a wrong turn while tracing the settings reload
path today, and it will cost the next person the same.

### 8. Credential surface

Five distinct secrets appear across the network path: `auth_key` (full API),
`lan_key` (scoped to LAN-forward routes), `host_reg_secret`, `ipc_token`, plus
per-message `jekt_sig` / `lan_sig` / `reagent_sig`. Each has a real,
well-documented reason to exist — this is **not** a call to consolidate them;
scope separation is exactly right for a security boundary.

The observation is narrower: there is no single document mapping which
credential gates which route at which tier. `Config::lan_key`'s doc comment is
the closest thing and only covers its own case. For a subsystem where the
security argument is the reason several tiers exist at all, that map should be
written down once.

---

## 9. Recommended sequencing

Ordered by value-per-risk, each independently shippable:

1. **Concurrent peer fan-out** (§3). Biggest perceived-latency win, smallest
   blast radius, no new architecture.
2. **Cross-platform bind spike** (§2). Cheap, and gates everything else in
   Part I.
3. **`LanListenerSupervisor`** (§2), ON→OFF first (closes the live exposure
   bug and proves the shutdown plumbing), then OFF→ON with interface watching.
4. **Gate mDNS advertising on a successful LAN bind** — eliminates the
   discover-but-cannot-deliver state permanently rather than narrowing it.
5. **Unify the tier-forward blocks** (§4) — do this *before* adding tier 4,
   so cloud relay is added in one place instead of becoming a fourth copy.
6. **Resolve tier 4** (§5): implement it in the chain, or correct the tool
   description. Not both.
7. **§6 naming fix**, **§7 rename**, **§8 credential map** — small, do
   opportunistically.

Note the ordering dependency: **§5 (tier 4) should follow §4 (unify)**. Adding
cloud relay to three copy-pasted blocks is how this subsystem got here.

---

## 10. Provenance

**Verified directly by reading code this session:** `bootstrap.rs:1305-1340`
(startup-only bind, ephemeral port, documented limitation);
`main.rs:173-193` (two listeners, cloned router); `websocket.rs:1582-1600`
(live `apply()` on setconfig); `lan_discovery.rs:571-600`
(`LanDiscoveryController` slot pattern), `:26` (2s timeout), `:745-775`
(sequential fan-out); `reactive.rs:49-62` (12-arg echo), `:747/:876/:982`
(three forward blocks), `:1044` (tier-4 comment);
`agentmux-mcp/src/main.rs:2266-2330` (no cloud fallback). Live `netstat`
showing `127.0.0.1`-only binding with LAN discovery enabled.

**Measured, 2026-09-06** (`backend::lan_listeners::tests`): simultaneous
`127.0.0.1:PORT` + `<lan-ip>:PORT` binding — **holds** (the load-bearing
assumption, verified on Windows, asserted in CI on Linux). And the wildcard
conflict is **platform-dependent**, not universal: true on Linux/macOS, false
on Windows. The first draft of this report asserted the universal version as
fact; the spike disproved it. Recorded rather than silently corrected, because
"standard socket behaviour" reasoning is exactly what produced the wrong claim.

**Still asserted, not tested:** that `if-watch`-style interface-change
notification is available and adequate on all three targets (§2).

**Not investigated:** whether any middleware or handler assumes a loopback peer
address (per-interface listeners would newly violate that); actual peer counts
in real deployments (§3's worst case assumes several); and the cloud relay
client that `reactive.rs:1044` refers to — I confirmed the MCP tool isn't it,
but did not establish whether some *other* caller implements tier 4 correctly.
