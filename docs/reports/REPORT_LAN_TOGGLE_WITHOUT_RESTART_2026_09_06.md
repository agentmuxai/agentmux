# Report: Making LAN discovery work on toggle, without a sidecar restart

**Date:** 2026-09-06
**Status:** Draft — options analysis + recommendation. Not implemented.
**Author:** Agent2
**Goal (operator):** *"we need a way around that. figure out how we can get the
network working without needing a restart. we want it to work similar to muxbus
cloud login, it works right after enabling it."*

---

## 1. What actually happens today

Toggling `network:lan_discovery` is **half-live**. Verified on a running
instance, not inferred:

| Component | Live on toggle? | Evidence |
|---|---|---|
| mDNS daemon (advertise + browse) | ✅ **Yes** | `LanDiscoveryController::apply` (`lan_discovery.rs:571+`), called from `websocket.rs:1591` on `setconfig`. Log shows `LAN discovery started (mDNS)` at the same second the setting was saved. |
| **HTTP listener bind address** | ❌ **No** | `bootstrap.rs:1321-1331` resolves `bind_addr` **once at startup**. |

So after enabling, the instance advertises itself on the LAN and gets
discovered — but its listeners are still on `127.0.0.1`, so a peer's
`GET http://{peer_ip}:{port}/agentmux/reactive/agent?id=…`
(`lan_discovery.rs` `find_agent`) hits nothing. **Discovery succeeds, delivery
cannot.** That is the worst possible failure shape: it looks enabled and
partially works.

Confirmed live on this host — LAN discovery on since 08:16, and:

```
TCP    127.0.0.1:55019   LISTENING   41500
TCP    127.0.0.1:55020   LISTENING   41500
```

The limitation is already documented in-code (`bootstrap.rs:1312-1320`,
both toggle directions), with the reason for deferral: *"rebinding an active
axum server is non-trivial."* That reasoning is sound but, as §3 argues,
**rebinding isn't actually necessary.**

Note the ON→OFF direction is also wrong today, and is arguably the more
serious of the two: mDNS stops, but the listeners **stay on `0.0.0.0`** and
remain LAN-reachable until restart. A user who turns the feature off is still
exposed (routes remain `lan_key`-gated, so this is exposure, not an open door).

---

## 2. Why cloud login *does* work immediately

Worth stating precisely, because it's the operator's reference point and the
difference is structural rather than a matter of effort:

**Cloud is an outbound, client-side connection.** `cloud_subscriber` dials
`wss://muxbus-ws.agentmux.ai`. Enabling it just means starting a task that
opens a socket. Nothing external needs to reach *us*, so there is no listening
socket whose address must change.

**LAN is inbound.** Peers must connect *to us*, which requires a listening
socket on a LAN-visible address. That socket's address is fixed at `bind()`
time. This is a genuine asymmetry, not an oversight — cloud can be made live
by starting a task; LAN cannot, by the same means.

The good news: the repo already has the right *pattern* for the live half —
`LanDiscoveryController`'s slot-based start/stop (`docs/specs/lan-discovery-toggle.md`).
What follows extends that same pattern to the listener.

---

## 3. The key insight: don't rebind — *add* a listener

The in-code deferral assumes the fix is "rebind the active server." It isn't.

`0.0.0.0:PORT` and `127.0.0.1:PORT` conflict (the wildcard subsumes loopback),
which is what makes rebinding look necessary. But **`127.0.0.1:PORT` and
`192.168.1.230:PORT` do not conflict** — a specific non-loopback address plus
the same port is a distinct binding and can be opened while loopback stays up.

And `axum::Router` is `Clone`, already served on two listeners today:

```rust
// main.rs:173-174
let web_server = axum::serve(net.web_listener, router.clone());
let ws_server  = axum::serve(net.ws_listener,  router);
```

So the move is: **on toggle-on, bind additional listeners on the host's LAN
interface addresses at the same ports, and `axum::serve` the same router on
them.** No rebinding, no port change, no dropped loopback connections, no
restart. On toggle-off, shut those extra listeners down via graceful shutdown
and leave loopback untouched.

This also fixes the ON→OFF exposure bug for free — dropping the extra listener
is exactly what "off" should mean.

**Why the port must be preserved:** ports are ephemeral (`:0`, `bootstrap.rs:1321`)
and already published in the mDNS TXT record and `authkey.dev`. Any approach
that changes the port on toggle would invalidate both. Binding the same port on
a different address avoids that entirely.

---

## 4. Options

### Option A — Additional per-interface listeners on toggle ✅ *recommended*

Extend `LanDiscoveryController` (or a sibling `LanListenerController`) with a
slot holding the extra listeners' shutdown handles, mirroring the existing
daemon slot.

- **On:** enumerate non-loopback IPv4/IPv6 addresses, bind `<addr>:<existing_port>`
  for both web and ws, `axum::serve(listener, router.clone())` each with a
  graceful-shutdown future wired to the slot's cancellation token.
- **Off:** cancel the token; loopback listeners are untouched.

*Pros:* no rebind, port preserved, loopback never interrupted, fixes both
toggle directions, reuses the established controller pattern.
*Cons:* needs the `Router` (or a factory) available after startup — a plumbing
change in `main.rs`; needs interface enumeration and a story for interface
changes (§5).

### Option B — Always bind `0.0.0.0`, gate at the app layer

Bind wildcard unconditionally; when LAN is off, reject non-loopback peers in
middleware.

*Pros:* trivial; toggle becomes pure policy, instantly live.
*Cons:* **the socket is always LAN-open.** It changes the OS-permission story
the operator explicitly wanted to avoid — on macOS, binding non-loopback is
what triggers the local-network prompt, so every user would get prompted
regardless of the setting. That directly contradicts the decision to default
the setting off *"so we dont bother users with the OS prompt unless they need
it."* **Rejected on that basis**, despite being the least code.

### Option C — Graceful shutdown + rebind

What the in-code comment contemplated.

*Pros:* single listener set; conceptually clean.
*Cons:* drops in-flight loopback connections (the frontend's own WS!) on every
toggle; with `:0` the rebind risks a *different* port unless explicitly
re-requested, invalidating mDNS TXT and `authkey.dev`. Strictly worse than A.

### Option D — Reverse-proxy/forwarder process

A tiny LAN-facing forwarder started on toggle, proxying to loopback.

*Pros:* zero change to the axum server.
*Cons:* a second process/port to supervise, extra hop, more failure modes, and
it still binds non-loopback (same prompt as B). Not worth it given A.

---

## 5. Details Option A must get right

1. **Interface enumeration and churn.** DHCP renewals, Wi-Fi↔Ethernet
   switches, and VPN adapters change the address set. This host alone shows
   `192.168.1.230` plus WSL/Hyper-V virtuals (`172.23.176.1`, `192.168.116.1`)
   and link-local IPv6. Bind per-address and re-evaluate on change, or accept a
   documented staleness window. **Do not** assume a single LAN IP.
2. **Bind failures must be non-fatal and visible.** A single interface failing
   to bind (already in use, permission) must not take down loopback or the
   toggle. Log it and surface it; the current all-or-nothing `.expect("failed
   to bind web listener")` at startup is not an acceptable model for a runtime
   toggle.
3. **Advertise what is actually reachable.** mDNS should publish an address a
   peer can connect to. Today `apply()` starts the daemon independently of
   whether any LAN listener exists — that decoupling is precisely what produces
   today's discover-but-can't-deliver state. Gate advertising on at least one
   successful LAN bind.
4. **Idempotency.** `apply()` is already idempotent and called unconditionally
   (`websocket.rs:1589-1591`); the listener controller must match, since the fs
   watcher can re-fire the same value.
5. **Both trigger paths.** The setting can change via `setconfig`
   (`websocket.rs:1588`) *and* via an external edit picked up by
   `config_watcher_fs`. The listener toggle must hang off the same place
   `lan.apply()` does — and ideally the *same call*, so the two can't drift.
   (Same "two independently-maintained paths" failure mode as the auth-dir and
   settings-seeding gaps.)
6. **Security posture is unchanged but should be restated.** LAN-forwarding
   routes are gated by the scoped `lan_key`, not the full `auth_key`
   (`bootstrap.rs:1305-1310`). Option A doesn't alter that; it only changes
   *when* the socket exists.

---

## 6. Recommendation

**Option A.** It is the only option that delivers the operator's stated goal
(works immediately on enable, like cloud login) without also delivering the
thing they explicitly rejected (an OS permission prompt for users who never
turn it on).

Suggested sequencing:

1. **Fix ON→OFF first** — dropping listeners on disable is the smaller half and
   removes a live exposure bug. It also proves the slot/shutdown plumbing.
2. **Then OFF→ON** — interface enumeration and per-address bind.
3. **Then gate mDNS advertising on a successful LAN bind** (§5.3), which
   eliminates the discover-but-can't-deliver state permanently.

Until this ships, the honest user-facing statement is: **enabling LAN discovery
requires restarting the instance before other machines can reach it.** That is
currently documented only in a Rust source comment
(`bootstrap.rs:1312-1320`) — no user will ever see it. It should be added to
the `settings-template.jsonc` entry and the Settings UI copy as a stopgap,
independently of this work.

---

## 7. Provenance

**Verified directly:** `bootstrap.rs:1305-1340` (startup-only bind, `:0`
ephemeral port, documented limitation), `main.rs:173-193` (two listeners, one
cloned router, `tokio::select!`), `websocket.rs:1582-1600` (live `apply()` on
setconfig), `lan_discovery.rs:571-600` (`LanDiscoveryController` slot pattern),
`find_agent`'s peer HTTP query, and live `netstat` showing `127.0.0.1`-only
binding on an instance with LAN discovery enabled since 08:16.

**Asserted from general knowledge, not tested here:** that
`127.0.0.1:PORT` and `<lan-ip>:PORT` can be bound simultaneously while
`0.0.0.0:PORT` conflicts with both. This is standard socket behaviour on
Linux/macOS/Windows, and it is the load-bearing assumption of Option A —
**prove it with a throwaway binary on all three platforms before building on
it.** Windows in particular has `SO_EXCLUSIVEADDRUSE` semantics worth
confirming.

**Not investigated:** whether any middleware or route currently assumes a
loopback-only peer address, which per-interface listeners would newly violate.
