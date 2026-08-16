# SPEC: LAN discovery peer metadata gets clobbered blank by TXT-less mDNS re-resolutions

**Date:** 2026-08-16
**Status:** Root cause confirmed, fix implemented and verified with new unit
tests (all passing — see §5) in `agentmux-srv/src/backend/lan_discovery.rs`
(not yet committed/PR'd)
**Reported by:** repo owner, live session — network panel showed blank hostname
fields, a literal `"v"` where a version should render, and a peer's own
`instance_id` string (`"v0.55.6"`) displayed in place of its hostname.

## 1. Symptom

In the status-bar network popover (`HostPopover.tsx`), LAN peer rows
intermittently show:

- Blank hostname (falls back to `instance_id`, e.g. a peer literally labeled
  `v0.55.6` instead of its real hostname `starpower`).
- A bare `"v"` with nothing after it — `HostPopover.tsx:241` renders
  `v{inst.version}`; an empty `version` string collapses to just `"v"`.
- Real, previously-discovered peers phantom-duplicating or losing their name
  entirely over time, not just on first discovery.

Also observed via `DiscoverAgents`: two `lan` entries for this machine's own
addresses with every field blank (`hostname: ""`, `instance_id: ""`,
`auth_key: ""`) but a real port matching an actual local instance.

## 2. Reproduction / evidence

Queried the live srv's own structured logs for every `LAN peer discovered`
event emitted during normal operation (`muxlog srv grep "LAN peer discovered"
--raw`):

```
peer_id counts:
   3776 ""
     42 "v0.55.6"
     76 "v0.55.8"
     32 "v0.55.9"
```

```
address counts (top 8, all local-machine virtual/link-local adapters):
    419 172.23.176.1
    415 fe80::296f:55a:b354:43c1
    412 192.168.153.1
    403 fe80::142e:c4b3:9f29:8348
    391 192.168.116.1
    384 fe80::3ccb:9537:8f8b:b7b1
    383 192.168.1.230
    371 fe80::4330:1668:b280:f907
```

`address` is **never** blank across ~3900 events; `peer_id` (the TXT record's
`instance_id` property) is blank in **~96%** of them. The addresses receiving
the overwhelming majority of events are this machine's own WSL2/Hyper-V/VPN
virtual adapters and link-local (`fe80::`) interfaces — i.e. this instance
re-resolving *itself* across every local network interface it has, not
distinct remote peers.

## 3. Root cause

`mdns_sd`'s `enable_addr_auto()` (`lan_discovery.rs::start()`) makes the
daemon continuously track address changes per network interface and re-fire
`ServiceEvent::ServiceResolved` far more often than fresh TXT record data is
actually refreshed. The large majority of these re-fires resolve with an
**empty TXT record** — `info.get_property_val_str("instance_id"/"hostname"/
"version"/"auth_key")` all return `None` — while the SRV/A-record-derived
`address`/`port` are always present. This is expected behavior from the mDNS
resolver, not a malformed-packet or malicious scenario; the bug is entirely
in how `handle_event` (previously) consumed these events.

Two compounding bugs in `LanDiscovery::handle_event`
(`agentmux-srv/src/backend/lan_discovery.rs`), both stemming from that one
mDNS behavior:

### 3.1 Self-filtering used the wrong (and frequently blank) key

```rust
let peer_id = info.get_property_val_str("instance_id").unwrap_or_default().to_string();
if peer_id == self.instance_id {
    return;
}
```

When the *first* `ServiceResolved` event for this instance's own service
happens to be one of the ~96% blank-TXT re-fires, `peer_id` is `""`, which
never equals `self.instance_id`. The self-check silently fails, and this
instance's own service gets inserted into its own peer list as a phantom
"peer" — one entry per local virtual/link-local interface address mdns-sd
enumerates.

### 3.2 Known-good TXT fields were overwritten unconditionally on every event

```rust
entry.hostname = hostname;   // always applied, even when hostname == ""
entry.version = version;
entry.auth_key = auth_key;
```

Because the same peer's `fullname` key receives hundreds of re-resolution
events and only a handful carry real TXT data, whichever event happens to
arrive *last* wins — overwhelmingly a blank one. A real, previously-correctly
-resolved peer (e.g. `starpower`, hostname populated) would regress to a
blank hostname/version/auth_key shortly after being discovered correctly,
because subsequent blank-TXT re-fires for the same service kept clobbering
the good data.

`entry.instance_id` was *not* subject to 3.2 (it's only set once, in
`or_insert_with`) — which is actually what makes 3.1's phantom self-entries
permanent: once inserted with `instance_id: ""`, nothing ever updates it, so
the entry can never retroactively be recognized as self.

## 4. Fix (implemented)

Both fixes are root-cause, not symptom patches — they change *what mDNS
event data is trusted for what purpose*, not add defensive UI fallbacks.

1. **Self-filtering by `fullname`, not TXT-derived `instance_id`.**
   `fullname` (`info.get_fullname()`) is deterministic from `service_name`
   (`format!("agentmux-{}", &instance_id)`, set once at registration) and
   comes from the SRV record, which — like `address`/`port` — is always
   present, unlike TXT. Comparing against `self.service_fullname` (already
   stored on `LanDiscovery`) correctly identifies self on every single event,
   including the blank-TXT ones, closing the phantom-self-peer hole at its
   source instead of trying to patch it up after insertion.

2. **Preserve prior TXT-derived fields when a re-fire has none.** `hostname`,
   `version`, `auth_key`, and (for defense in depth) `instance_id` are now
   only written into the entry when the *current* event actually carried a
   non-empty value; a blank re-fire no longer erases previously-known-good
   data. `address`/`port` remain unconditional overwrites since they come
   from the SRV/A record and are always populated correctly, including on
   genuine address changes that should be reflected immediately.

Both changes are in `LanDiscovery::handle_event`,
`agentmux-srv/src/backend/lan_discovery.rs`.

## 5. Verification

**Automated — done, all passing.** Rather than extracting a separate
`merge_lan_instance` free function, `handle_event` is tested directly: it
takes `&self` (private, but reachable from a child `#[cfg(test)]` module in
the same file per normal Rust privacy) and a real `ServiceEvent`/
`ServiceInfo`, both fully constructible without registering or browsing on
the real mDNS daemon (only `ServiceDaemon::new()` is called, to satisfy the
struct field — no `.register()`/`.browse()`, so none of the multicast
flakiness documented on the existing `#[ignore]`d round-trip test applies).
Added `handle_event_tests` (`agentmux-srv/src/backend/lan_discovery.rs`):

- `self_resolution_with_blank_txt_is_never_inserted_as_a_peer` — the exact
  live-log failure mode: a `ServiceResolved` for this instance's own service
  with an empty TXT record must not appear in `get_instances()`.
- `self_resolution_with_full_txt_is_also_never_inserted_as_a_peer` — sanity
  check the straightforward case wasn't regressed.
- `blank_txt_refire_does_not_clobber_previously_known_good_peer_fields` —
  discovers a real peer with full TXT data, then re-fires the same
  `fullname` with an empty TXT record; asserts `hostname`/`version`/
  `auth_key`/`instance_id` all survive unchanged.
- `address_and_port_still_update_unconditionally_on_a_blank_txt_refire` —
  confirms the SRV-record-derived fields keep updating live even when the
  TXT-derived fields in that same event don't.
- `a_new_peer_first_seen_via_blank_txt_has_no_data_to_preserve` — documents
  the expected (acceptable, not a regression) behavior when there was never
  any good data to preserve in the first place.

Ran via (this crate is bin-only, no lib target — `--bin`, not `--lib`):

```
cargo test -p agentmux-srv --bin agentmux-srv backend::lan_discovery::
```

Result: **28 passed; 0 failed; 1 ignored** (the ignored one is the
pre-existing, documented-flaky real-multicast round-trip test, unrelated to
this change and untouched by it). `cargo clippy -p agentmux-srv --bin
agentmux-srv --tests` produced zero warnings in the new `handle_event_tests`
module (two pre-existing style hints elsewhere in the file, in the
untouched ignored test, are unrelated).

**Not yet done:**
- Manual: rebuild, run two instances (or reuse the existing multi-channel
  setup — AgentX/Lark/AgentY/Loap already running this session), confirm the
  network popover shows stable, correct hostnames/versions for every real
  peer and zero phantom self-entries over several minutes of runtime.
- Re-run the same `muxlog srv grep "LAN peer discovered" --raw` peer_id/
  address tally used to diagnose this against a rebuilt instance — after the
  fix, self-address entries (the WSL/Hyper-V/VPN/link-local ones) should no
  longer appear as `LanInstance` rows in `get_instances()`/`laninstances`
  broadcasts at all (they're filtered before reaching the map), and real
  peers' stored `hostname`/`version`/`auth_key` should stay populated even
  while blank re-fires continue arriving.

## 6. Out of scope

- Reducing the sheer volume of redundant re-resolution events
  (`enable_addr_auto()`'s chattiness itself) — cosmetic/log-noise concern,
  not correctness; not addressed here.
- The pre-existing `LAN_AGENT_CACHE_TTL_SECS`/`pubkey_cache` logic in
  `LanDiscoveryController` — unaffected by this bug (keyed by `agent_id` via
  a separate peer-fan-out HTTP call, not by the mDNS-populated `instances`
  map's TXT fields).
