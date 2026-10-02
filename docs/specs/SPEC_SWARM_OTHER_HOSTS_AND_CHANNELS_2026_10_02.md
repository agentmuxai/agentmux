# SPEC: The Swarm shows the agents of every other instance this one knows about

**Date:** 2026-10-02
**Status:** proposed — research and design; nothing implemented. Section 9 asks for decisions.
**Author:** AgentX (narko), at the owner's request
**Affects:** `frontend/app/view/swarm/` (`swarm-model.ts`, `swarm-view.tsx`), `crates/srv/src/server/http_health.rs` (`/agentmux/discovery`), `crates/srv/src/backend/lan_discovery.rs`, `crates/srv/src/server/reactive.rs`, `crates/srv/src/registry/`, and, for the cloud tier, `crates/srv/src/muxbus/` plus the separate `agentmux-cloud` repository
**Builds on:** `docs/specs/SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md` (the same-host registry), `docs/specs/SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (install identities), `docs/specs/SPEC_LAN_UDP_PEER_DISCOVERY_2026_10_02.md` (how LAN peers are found), `docs/specs/SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md` §3 (the per-account directory this needs)
**Related:** `docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md` (the line a row shows)

## 1. What the owner asked for

Extend the Swarm to show the other instances this one is aware of, over the LAN and the cloud. At the top, as today, this instance's agents. Below, one section per host with the agents present under it. If a host has one channel, the section is named for the host only; the channel is named only when a host has more than one. This supports LAN and cloud.

## 2. What exists, and what does not

Findings from the code, 2026-10-02. This decides what can ship first.

| Tier | Knows about | Gives per agent | Does not give |
|---|---|---|---|
| **This host, other channels** | `~/.agentmux/shared/agents/reactive/<agent>/<channel>.json`, one file per agent and channel, rewritten every 20 s (`registry.rs`). `GET /agentmux/discovery` already returns `host.cross_channel[]` as `{name, channel, local_url, block_id}` for every other channel. | name, channel, `block_id`, `local_url` | status, provider, summary, tools. No frontend code calls the endpoint. A crashed channel's entries can linger up to 4 h (the startup sweep's age limit). |
| **LAN hosts** | `LanInstance` per peer (`lan_discovery.rs`): `hostname`, `address`, `port`, `version`, and `agents: Vec<String>` refreshed every 30 s from `GET /agentmux/reactive/agent-names`. A peer is one entry **per instance**, not per host: the mDNS label is `agentmux-<hostname>-<port>`, so a host running two channels is two peers. | name only | channel (not advertised), status, provider, summary. `/agentmux/discovery` is not reachable with the LAN key. The names route's own comment says not to extend it beyond names without revisiting an owner decision. |
| **Cloud (same account, other installs)** | Nothing about other installs' agents. `wan.db` holds `wan_known_instances` (installs seen on a verified jekt, with `approved`/`new`/`revoked`) and cached directory records. The relay answers a lease claim with who holds that one agent (`held_by{host, channel, version}`). | nothing | No presence or agent-list API exists on the client or in any spec text. `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_...` §3 only proposes a per-account directory and sequences it as needed "before meaningful WAN-tier enumeration". |

And on the Swarm side: `buildTree()` starts from this instance's tracked block ids, resolves each to a local `Block` object, and drops any id with none. `AgentTreeNode` has no host, channel or instance field. A remote agent has no local block, so it has no status, provider, summary, tool, subagent, shell, cron or todo data, and no live subscriptions. **A remote row cannot be a smaller copy of a local row; it is a different, much thinner kind of row.**

## 3. Design

### 3.1 Layout

```
THIS INSTANCE                         ← today's tree, unchanged, always first
  AgentX   …
  Korp     …

Area54                                ← LAN host with one channel: host name only
  Manoz            (names, plus status when known)
  Opaz

narko · dev-fix-lan                   ← host with several channels: "host · channel"
  Loap
narko · stable
  Agent3

home-desktop (cloud)                  ← install with no LAN path
  Reagent
```

- The first section is this instance and is never grouped or collapsed away.
- **Naming rule:** the channel is shown if and only if the host has more than one channel *in total*, counting this instance's own channel for the local host. So on narko, whose own channel is `stable`, a second channel `dev-fix-lan` makes narko a multi-channel host: its other-channel sections read `narko · dev-fix-lan`, and the top section's header, if shown, reads `narko · stable`. A host with one channel is just `Area54`.
- Sections after the first are ordered: this host's other channels, then LAN hosts, then cloud-only installs; alphabetical within each. A small badge says how it was found (`this machine`, `LAN`, `cloud`), because trust and freshness differ.
- Each section is collapsible and remembers its state (the pattern `toggleAgentCollapsed` already has). A section with no agents is hidden; a footer counts hosts that were seen but are unreachable.
- A stale section is dimmed with its last-seen age; past the tier's limit (§4) it disappears.

### 3.2 The remote row

Phase 1 rows carry the agent name and a one-line origin, and are read-only: no collapse, no sub-rows, no selection checkbox, so they are **excluded from fleet actions** (broadcast, bulk stop). The status chip and the line appear only when the tier supplies them (Phase 4). They never reuse `resolveSwarmLine`'s status phrases for a remote agent, because "No activity yet" would be a claim about an agent we know nothing about. Without data the row shows the name alone.

### 3.3 Data plane

One backend aggregator in srv builds the snapshot and pushes it as a WS event (`swarmremote`), the way `laninstances` already is, so the frontend never talks to other hosts and holds no remote credentials.

```
RemoteHost   { host_id, display_name, tier: "host"|"lan"|"cloud", channels: RemoteChannel[] }
RemoteChannel{ channel, install_id?, version?, seen_at, state: "live"|"stale", agents: RemoteAgent[] }
RemoteAgent  { name, block_id?, status?: "running"|"idle", provider?, line? }
```

`status`, `provider` and `line` are optional and filled only by tiers that can supply them and are permitted to (§6).

**Merging.** The same machine can arrive by two routes (the shared registry and its own mDNS record), and the same install by LAN and by cloud. Preference is the richer source: shared registry over LAN over cloud. Keys:

- same host: a LAN peer whose address is one of this machine's own addresses is another channel on this host, and is dropped in favour of the registry's entry (`is_self_resolution` already compares addresses this way);
- LAN to cloud: needs a stable id on both sides. The WAN `install_id` is not in the mDNS TXT record today. Adding `install_id` and `channel` to the TXT record (and the UDP reply of the LAN discovery spec) is part of Phase 2, and is also what lets a LAN host name its channels at all.

### 3.4 Phases

| Phase | Scope | Needs | Verified on |
|---|---|---|---|
| **1: other channels on this host** | srv: the aggregator, fed by the registry (`cross_channel`), plus the `swarmremote` event. Frontend: the sections, the naming rule, read-only name rows, stale handling. | nothing new on the wire; all data exists | narko with `stable` plus a `task dev` channel |
| **2: LAN hosts** | Feed `LanInstance.agents` into the aggregator. Advertise `channel` and `install_id` in TXT and the UDP reply so multi-channel hosts name their channels and later merge with cloud. | one TXT field each | narko with Area54 or starpower |
| **3: cloud installs** | Needs the per-account presence directory in `agentmux-cloud` (§5). Client: publish this install's agent names, fetch the others, verify each record against the instance key it was signed with, hide `revoked` installs. | a cloud endpoint; a decision on what installs may publish | two installs on one account |
| **4: status and line for remote rows** | `status` first (running or idle, no content). `line` only if the owner decides to (§6). LAN: a new `lan_key` route returning the thin record; cloud: part of the signed record. | owner decisions 1 and 2 of §9 | narko ↔ Area54 |

Phases 1 and 2 do not touch the cloud repository. Phase 3 cannot start from this repository alone.

## 4. Freshness and failure

- **Same host:** a registry file untouched for 60 s (three missed heartbeats) is `stale` and dimmed; past 10 minutes it is hidden, well inside the registry's own 4 h sweep, because a crashed channel should not look present for four hours.
- **LAN:** a peer is listed for the staleness floor of 300 s after it was last heard (`peer_staleness_window_secs`); show it dimmed after 60 s.
- **Cloud:** an install not refreshed within three of its own publish intervals is `stale`; past a day it is hidden. Offline (no account login, no network): the cloud sections are omitted silently, not shown empty.
- A remote tier that errors must never affect the first section or the other tiers. Each tier is read independently and a failure becomes "unreachable", counted in the footer.

## 5. The cloud tier's contract (for the `agentmux-cloud` work)

The client needs, per account: *publish* this install's agent names, and *list* the other installs' records. Sketch:

- `PUT /accounts/me/installs/<install_id>/agents` with `{host_display, channel, version, agents:[{name, status?}], published_at}`, signed with the install's key from the existing key directory so a record proves which install wrote it (the `wan-verified` property, not just a bearer token).
- `GET /accounts/me/installs/agents` returns the records of the account's other installs with each install's `approved`/`new`/`revoked` state; `revoked` installs are omitted.
- Records expire server-side; the client treats a missing record as "gone".

Same-account only, as the WAN signing is. A sender on another account is never listed.

## 6. Privacy and trust

- **What leaves the machine.** Names only crosses to LAN and cloud peers in Phases 1 to 3. LAN peers already get names; the cloud tier newly sends agent names off the machine to the account's relay. Say so in the settings text for the feature.
- **`status`** (running or idle) is activity metadata, not content. **`line`** is content: a paraphrase of what the user asked. A LAN peer holding the `lan_key` is only network-claimed, so a `line` on that route is readable by anything on the LAN that learns the key. Default off; if enabled it should be per-tier and opt-in.
- **Found on the way, not part of this spec's scope:** the `laninstances` WS event serialises each peer's full `LanInstance` to the frontend, `auth_key` (the peer's `lan_key`) included, although the TypeScript type omits it. It goes to local clients only, but the frontend does not need it. It should be dropped from the event before the Swarm starts consuming peer data.
- Remote rows are display only in Phase 1, and carry no credentials to the frontend.

## 7. Not in scope

Jekting a remote agent from its row; opening a remote agent's pane; fleet actions on remote agents (the existing cross-channel `FleetBulkStop` forwarder is not exposed); a remote agent's sub-rows, todos or tools; showing other accounts' agents.

## 8. Tests

- Aggregator: merge preference (registry over LAN over cloud), the same-host address rule, stale and hidden thresholds, one tier failing while the others are served, the naming rule (host with one channel, with two, and the local host whose own channel counts).
- Frontend: section ordering and badges, the channel shown only for multi-channel hosts, no selection checkbox and no fleet participation on remote rows, a remote row with no data shows the name alone, collapse state persisted, first section unchanged with the feature on and off.
- Phase 2: `channel` and `install_id` round-trip through TXT and the UDP reply, and an old peer without them still lists.

## 9. Decisions for the owner

1. **How much about a remote agent?** Name only (Phase 1 to 3); then running/idle (Phase 4, recommended, no content); then the title line (opt-in, content). Where does the line stop?
2. **May the cloud tier send agent names off the machine?** It is the first thing that does. Recommended: yes, same account only, behind a setting that is on once the user is signed in.
3. **Actions on remote rows.** Read-only for now (recommended). Next in value order: compose a jekt to that agent, then focus or open it.
4. **Hosts with no agents.** Hidden, with an unreachable count in the footer (recommended), or always listed?
5. **Host names.** The OS hostname (today's `LanInstance.hostname`), or a user-set alias per host?
6. **A switch.** On by default for same-host channels and LAN, with cloud tied to sign-in (recommended), or one setting for all?
