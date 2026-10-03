# SPEC: A names-only fleet feed for LAN peers — snapshot, event stream, UDP siblings

**Date:** 2026-10-03
**Status:** implemented — branch `clamk/lan-fleet-feed`; not yet verified against the phone client on real hosts (section 7).
**Author:** Clamk, at the owner's request
**Affects:** `crates/srv/src/backend/fleet_feed.rs` (new), `crates/srv/src/server/http_fleet.rs` (new), `crates/srv/src/server/routes.rs`, `crates/srv/src/server/http_health.rs`, `crates/srv/src/backend/lan_discovery.rs`, `crates/srv/src/backend/lan_discovery/lan_instances.rs` (new), `crates/srv/src/backend/lan_discovery/udp_peers.rs`, `crates/srv/src/registry/paths.rs`, `crates/srv/src/main.rs`
**Source of the contract:** agentmux-mobile's [`SPEC_LIVE_FLEET_TOPOLOGY_2026_10_03.md`](https://github.com/agentmuxai/agentmux-mobile/blob/main/docs/specs/SPEC_LIVE_FLEET_TOPOLOGY_2026_10_03.md) §4 (adopted 2026-10-03). Field names, headers, timings and limits are defined there; this document records the desktop side and where it differs.
**Aligns with:** `docs/specs/SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md` (the same host → channel → agent tree, and its names-only line for `lan_key` holders)

## 1. Why

A phone holds only the broadcast `lan_key`. Until now it could read one thing with it, `/agentmux/reactive/agent-names`, and only by polling. It had no way to tell "nothing changed" from "this instance restarted", and only one channel per host ever answered its UDP probe, because only one process can hold UDP 47891. The mobile spec's findings F1, F4, F6 and F10 have the detail.

## 2. Routes (both in `lan_forward_routes`, so the `lan_key` is accepted)

**`GET /agentmux/fleet`** returns `{"epoch","rev","hostname","channel","version","agents"}`. `agents` is `reactive_handler.list_agents()` reduced to names, sorted case-insensitively, with names differing only by case collapsed to one. `epoch` is 16 lowercase hex characters, new per srv launch. `rev` starts at 1 and goes up by one each time the name set changes. The response carries `ETag: "<epoch>:<rev>"` and `Cache-Control: no-cache`. A matching `If-None-Match` gets `304` with no body.

**`GET /agentmux/fleet/events`** is Server-Sent Events (`text/event-stream`, `no-cache`). It writes `retry: 3000`, then the snapshot as `event: fleet` / `id: <epoch>:<rev>` / `data: <the snapshot JSON on one line>`. The snapshot is skipped when the request's `Last-Event-ID` already equals the current `<epoch>:<rev>`. After that the stream sends one `fleet` event per change and `: hb` every 15 s. At most 32 streams are open per srv; the 33rd request gets `503`.

Change detection is one task (`FleetFeed::spawn_change_detection`). It reads the name set once a second and publishes `(rev, names)` through a `tokio::sync::watch`. Both routes read from that channel, so they always agree, and a slow stream skips to the latest state instead of queueing. `main.rs` starts the task with the same shutdown token the WAL checkpoint loop watches.

Each stream holds a slot guard that is dropped with the stream. That happens when hyper drops the response body after the client disconnects; the next heartbeat write notices a dead peer, so a slot is freed within about 15 s.

**Names only.** Nothing about an agent beyond its name is added to any `lan_key` route. `handle_reactive_agent_names` is unchanged. `hostname`, `channel` and `version` are the instance metadata the mDNS record already carries.

## 3. UDP reply `siblings` and responder failover

Each LAN-enabled instance writes `<shared>/lan/instances/<channel>.json` (`registry::resolve_shared_lan_instances_dir()`, beside `<shared>/agents/`) as `{channel, port, lan_key, version, pid, updated_at_ms}`. It writes the file when LAN discovery starts and rewrites it every 20 s. The file is removed when discovery stops (`LanDiscovery::shutdown`, which a rebuild also calls) and on clean srv exit (`main.rs`). The write is atomic and the file is created 0600 on Unix, the same care the shared reactive registry takes for its keys. The channel is reduced to a safe file name (`[a-z0-9_-]`, capped at 64 characters); the real name stays inside the file. Only the owning pid removes a record. A stop flag shared under one lock with the rewrite keeps a rewrite that races a stop from bringing the file back.

The 47891 reply (`build_probe_response`) adds `"siblings": [{channel, port, auth_key, version}]`. `auth_key` is that sibling's `lan_key`, under the field name clients already read. The list is built from the records, skipping any that are older than 60 s (or more than 60 s in the future), whose pid is dead, or that are this instance (same channel or same pid). Reads are bounded: at most 64 files are scanned, a file over 4 KiB is skipped, and so is a record with an empty or oversized channel, key or version, or port 0. There is at most one entry per channel and at most 16 entries, and the whole reply stays under 1400 bytes so it fits in one unfragmented datagram. The full `auth_key` is never written or sent. `siblings` is always present, empty when there are none.

When the bind on 47891 fails, the responder now retries every 15 s for as long as LAN discovery is on, and the existing cancel channel ends the wait. It logs once at debug when it starts waiting and once at info when it takes over the port.

## 4. Where this differs from the mobile spec

- **The desktop-to-desktop reply on 29700 (`udp_peers`) carries no `siblings`.** The mobile spec names only the 47891 reply. Desktop peers read replies into a 1024-byte buffer, which a sibling list could overflow, and they find a host's other channels over mDNS anyway. That path uses `build_identity_response`, the payload without siblings.
- **The 1400-byte budget caps the list before the count of 16 does.** A sibling entry is about 110 to 180 bytes, so a reply carries five to ten siblings in practice, depending on channel-name length. A machine runs two or three channels, so this does not bind.
- **`If-None-Match` is matched leniently.** A list of tags, a weak `W/` tag, `*`, and a tag sent without its quotes all match.

## 5. `/agentmux/discovery` (full key)

`host.channel` is added: this instance's own channel (`local_channel_id()`), so a client paired by QR code or by hand can name it.

## 6. Compatibility

Everything here is a new field or a new route. Existing routes, fields and log levels are unchanged, apart from the responder's new retry messages. An older phone ignores `siblings`. A newer phone against an older desktop falls back route by route, as mobile spec §4.5 describes.

## 7. Verification

Unit and router tests cover the snapshot shape, sorting and de-duplication; the ETag and `304`; the `lan_key` being accepted on both routes and a wrong key refused; the first SSE event; a quiet resume on a current `Last-Event-ID`; a change producing an event; the heartbeat; the 503 cap and the slot being freed when the body is dropped; the sibling-record lifecycle, file-name sanitizing and the stale, dead-pid, self, malformed and oversized filtering; the probe reply listing siblings, the desktop reply not listing them, and no full key appearing; the responder taking over a port once its holder lets go, and stopping while it waits; and `host.channel` in discovery.

Still to do live, per mobile spec §8: Narko with two LAN channels and one LAN-off channel, starting and stopping a channel while the phone watches.
