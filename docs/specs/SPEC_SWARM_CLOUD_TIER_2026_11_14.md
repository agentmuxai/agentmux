# Spec: The cloud tier: agents on the account's other machines that are not on the LAN, listed by srv for the Swarm and Atlas

**Status:** active. Built in #47600 (PR #7324): the cloud tier, the `cloud` field and the Presence `who` state, with the host-and-channel fallback for de-duplication. Left: the LAN advertisement's WAN instance id, for the exact match (section 3.3).

Task #47599 (build: #47600, after this). Owner: AgentA (srv core). Requested by Camper (Atlas, #47554 part d); routed by
AgentY. Written 2026-11-14 from origin/main `a87476e72`. Built in the PR that carries this file, except the LAN advertisement's WAN id
(section 3.3), which is a follow-up. The relay's side of it, and the one thing
asked of it (nothing), is in the private cloud repository's copy of this spec.

## 1. What is asked

The Swarm and Atlas already draw a **cloud** tier: the frontend types it (`RemoteHost.tier` is `host` | `lan` | `cloud`,
`swarm-remote.ts`), badges it, orders it last, and treats a cloud section as another machine that a broadcast and a stop
cannot reach (`swarm-fleet-targets.ts`). srv never sends one: `swarm_remote.rs` makes `host` and `lan` hosts only. The
build is srv reading the account's other installs back and adding them to the `swarm.other-instances` answer as hosts of
tier `cloud`, so a machine of the account that is not on this LAN (a laptop on another network, a build box) appears.

Three behaviours are fixed by the task: a machine already listed over the LAN is not listed twice; a stale record is flagged
and then dropped as LAN peers are; signed out of the cloud means no cloud tier, not an error. A fourth is a decision here:
whether a broadcast can reach a cloud agent.

## 2. What exists today (re-read before building)

- **srv publishes this install's presence, and reads nothing back.** `muxbus/install_presence.rs` is the signed record
  (an install's `instance_id`, public key, `hostname`, `channel`, `os`, `version`, `channels_running`, its agents, `published_at_ms`,
  and a `gone` goodbye); each agent carries `name`, `kind` (host or container), and from version 2 its `state`
  (`working`, `waiting`, `idle`, `stopped`, `error`), from version 4 its summary `line`. `muxbus/wan_presence.rs` publishes
  it with the MuxBus sign-in's token and relay base URL when signed in (not on a dev build, a headless srv or an isolated
  home), on every change and at least every minute, and says goodbye on quit and sign-out.
- **The record is checkable.** It is signed with the install's WAN instance key and `instance_id` is derived from that key
  (`jekt_sign::wan_instance_id`); `InstallPresence` has the check and the bounds (`MAX_PRESENCE_AGENTS`, lengths), written for
  the relay and reusable by a reader.
- **The relay can list the account's records.** The signed-in phone reads them (agentmux-mobile's fleet-host spec §6). srv
  has no reader.
- **`Presence who`** lists agents reached over the relay by name only, with the status fixed at "online"
  (`server/work_facts_handlers.rs`, `remote_agents`); it reads a different source (the agent directory), not presence.
- **The LAN peer's id is not the install's.** `LanInstance.instance_id` is the discovery id (a release or build id in
  tests and in `udp_peers.rs`), not the WAN instance id; the LAN advertisement carries `hostname`, `channel`, `os`,
  `version`, `address`, and the agents' maps, but not the WAN id. So today a LAN peer and its cloud record can only be
  matched by host name and channel.
- **How the answer is built.** `server/app_api/agent_io.rs` registers `swarm.other-instances`: it reads the registry, asks
  other channels over loopback, and calls `swarm_remote::snapshot(entries, statuses, lan_peers, own_addrs, hostname, own_channel,
  own_url, now_ms)`, which returns this machine's other channels as one `host` host followed by `lan_hosts(..)`.
  `RemoteAgent` is `name, block_id, status?, line?, provider?, seat?, account?`; `RemoteChannel` is
  `channel, seen_at_ms, stale, os, agents, route` (the route is never serialized).

## 3. Proposal

### 3.1 Reading the account's installs

- A new module `muxbus/cloud_installs.rs` reads the account's records with the same token and relay base URL the
  publisher uses (`cloud_subscriber::load_valid_token`), through one function that returns `Vec<CloudInstall>` and the relay's
  clock at the time of the answer (its `Date` header, as `wan_presence` already uses it, so ages do not depend on this
  machine's clock).
- **Every record is verified before it is used**: the signature against the record's own public key, `instance_id` against
  that key, the field bounds, a known version. A record that fails is dropped and counted in a log line (never its content).
  Records are display data: nothing is actionable from one.
- **When.** On demand, with a cache: a `swarm.other-instances` call reads the cache; when the cache is older than 30 s and
  the account is signed in, one request refreshes it (one at a time, never blocking the answer past 1.5 s, like the loopback
  channel reads; the answer then has the old cache). Nothing polls while nobody asks. A sign-in, a network change and a
  wake from sleep (the signals `wan_presence` already watches) drop the cache so the next ask is fresh.
- **Signed out, no route (an older relay), a network error, a refusal:** no cloud hosts and no error in the answer. On a
  network error or a 5xx the last good list is kept for up to 10 minutes, ageing (so it turns stale and then drops as time
  goes), then dropped.

### 3.2 Records into hosts

- One record is one install, so one host with one channel: `RemoteHost { host_id: "cloud:<hostname lower>#<hash of instance id>",
  display_name: hostname, tier: "cloud", channels: [RemoteChannel { channel, seen_at_ms, stale, os, agents }] }`. The naming
  rule the frontend applies (host alone, or host · channel when a host has more than one) works unchanged. There is no
  `route`: a cloud channel is never addressable from here, which is how the broadcast and the stop already skip it.
- `seen_at_ms` is when the relay last heard from the install, moved onto this machine's clock using the relay's `Date`;
  `stale` after 3 minutes (three missed beats; the LAN's rule is 60 s against a 20 s beat) and hidden after 10 minutes,
  the same shape as `STALE_AFTER_MS` and `HIDE_AFTER_MS`. A `gone` record is dropped at once.
- Agents: `name`, `status` (the record's state, mapped onto `AgentState`; absent for version 1), `line` (version 4), and the
  runtime kind from the record's `kind` (the container-status spec, #47595, gives `RemoteAgent` that field). `provider`,
  `seat` and `account` are not in a record and stay absent, so a cloud row is thinner than a LAN row, as the Atlas spec
  already notes. `block_id` is empty.
- The answer carries no address, no key, no instance id and no signature: the frontend never needs them.

### 3.3 No machine twice

A cloud record is left out when it is:

1. **This install's own** (`instance_id` equals this srv's WAN id);
2. **Another channel of this machine** (the registry already lists it, in more detail): its host name equals this machine's
   (case-insensitively) and its channel is one the registry lists;
3. **A LAN peer**: matched exactly by the WAN instance id when the peer's advertisement carries it, else by host name and
   channel (case-insensitively).

**Built so far: the fallback only** (host name and channel, case-insensitively; a peer that did not advertise its channel
hides nothing). For the exact match, the LAN advertisement gets one optional field, the install's WAN instance id (a public hash of a
public key, not a secret). A peer on an older build does not send it, so it falls back to host name and channel. The
fallback can hide a cloud machine that shares both with a LAN peer (two cloned VMs with one name and one channel), which
is a smaller error than listing one machine in two sections, and it ends as peers update. *(Decision 3.)*

### 3.4 Broadcast and stop

**Not in this build.** A cloud agent has no route here, and the frontend keeps treating the tier as unreachable by
Broadcast and Bulk stop. Reaching a cloud agent is a signed message over the relay, the route `Message send` already uses
for a remote agent by name; it needs a decision about which cloud rows are addressable (an agent that has published its
key), what the Swarm's selection does with them, and an approval for a remote stop. That is its own spec when someone asks.

### 3.5 What the Swarm can say about the cloud

An optional `cloud` field on `swarm.other-instances`: `"off"` (signed out, or publishing and reading off), `"ok"`,
`"unavailable"` (the last read failed and the list is the kept one or empty). The frontend ignores it today; Atlas can show
"cloud off" instead of an empty tier. *(Decision 4.)*

### 3.6 Who may read it

`swarm.other-instances` is a websocket command, closed to container agents. The same cache lets `Presence who` replace its
fixed "online" for agents reached over the relay with the record's state, where the agent appears in a record, which is a
one-line improvement for every agent, container agents included. *(Decision 5.)*

## 4. Decisions to confirm

1. **Read on demand with a 30-second cache**, against a background poll. *Recommend:* on demand (nothing runs while the
   Swarm is closed; the Swarm refreshes about as often as the cache expires).
2. **Staleness for cloud rows:** stale after 3 minutes, hidden after 10. *Recommend:* as written.
3. **Dedup by the WAN id on the LAN advertisement, falling back to host name and channel.** *Recommend:* as written, in the
   same build as the reader.
4. **The `cloud` field** on the answer. *Recommend:* yes (small, optional, and Atlas gets a truthful empty tier).
5. **`Presence who` uses the same cache** for the agents it reaches over the relay. *Recommend:* yes, in the same PR, so
   the two views never disagree.
6. **Broadcast and stop on cloud agents: out of scope here.** *Recommend:* as written.
7. **Relay behaviour:** this build asks the relay for nothing it does not already do (the private copy says what it does
   and what a heavier use would cost). *Recommend:* as written; AgentY is asked before anything changes.

## 5. Plan (the build, #47600)

One PR (the reader, the mapping and the dedup are small and only useful together), then the Atlas look with a real cloud
machine (Camper):

- `muxbus/cloud_installs.rs`: the reader, the verification, the cache and its rules; a `Source` trait (the real relay
  client, and a fake for tests).
- `swarm_remote.rs`: `cloud_hosts(installs, own_wan_id, lan, registry_channels, now_ms)` and `snapshot` calling it; the
  `cloud` field.
- The LAN advertisement's optional WAN id and its reader; `Presence who`'s use of the cache.
- Tests: a valid record becomes one cloud host; a forged signature, a wrong `instance_id` and an out-of-bounds field are
  dropped; the install's own record, another channel of this machine and a LAN peer (exact id and the fallback) are not
  listed twice; a version 1 record has no state; stale after 3 minutes, hidden after 10, `gone` at once; signed out gives
  no hosts and `cloud: "off"`; a network error keeps the last list for 10 minutes and then drops it; the answer carries no
  id, key or address; two requests never overlap; the frontend's `remoteSections` draws the tier unchanged.
- A changeset, light checks, one push; Linux and macOS for the cloud read (a real signed-in account is the first live
  check, on a build with publishing forced on, by someone with two machines).

## 6. Risks and notes

- **Thin rows.** A cloud agent has a state and a line and no provider, seat or account; Atlas draws it with what there is.
- **Trust.** The records are signed by installs of the account, so the relay cannot invent an agent, but it can withhold or
  replay one: the tier is display-only and says when it last heard. Nothing acts on it.
- **Privacy.** Only the account's own records; summary lines are what the person already chose to share when publishing
  (`swarm:sharesummaries`); no address or credential leaves srv in the answer.
- **Clock.** Ages come from the relay's own clock, so a wrong local clock does not make a live machine stale.
- **Public repository:** this copy names no table, key, account detail or unfixed weakness of the relay.

## 7. Where each fact lives (for the build)

- `crates/srv/src/muxbus/install_presence.rs` (the record, its signature, bounds), `muxbus/wan_presence.rs` (publisher,
  token, `Date` handling, the network and wake signals), `muxbus/cloud_subscriber.rs` (`load_valid_token`),
  `backend/storage/wan_identity.rs` (this install's WAN id).
- `crates/srv/src/backend/swarm_remote.rs` (`RemoteHost`, `snapshot`, `lan_hosts`), `server/app_api/agent_io.rs`
  (`swarm.other-instances`), `backend/lan/discovery.rs` and `lan/discovery/agent_names.rs` (the advertisement and its
  reader), `server/work_facts_handlers.rs` (`remote_agents`).
- Frontend: `frontend/app/view/swarm/swarm-remote.ts` (`RemoteHost`, `remoteSections`), `swarm-fleet-targets.ts`.
- Related specs: `SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md`, `SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md`,
  `SPEC_LAN_FLEET_FEED_2026_10_03.md`, `SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md`, `SPEC_ATLAS_AGENT_MAP_2026_11_13.md`,
  and the container-status spec of the same date.

Agent: AgentA@Area54
