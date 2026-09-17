# Retro: LAN peers vanished from discovery every cycle — a 5-minute staleness cutoff was tuned against one fast peer, not the protocol

**Date:** 2026-09-17
**Status:** implemented — root-caused and fixed here; fix shipped in this same PR.
**Severity:** Low functionally for a single-LAN-peer setup (discovery still
works, just flickers), Medium for multi-peer LANs — most peers on a real
network intermittently disappear from `GET /agentmux/discovery`, `SendMessage`
routing, and the status-bar/Warden LAN peer lists, on a ~40-46 minute cycle.
**Observed by:** repo owner, comparing LAN peer visibility across three hosts
on the same network (`Area54`, `narko`, `starpower`) plus one long-lived peer
(`gamerlove`).
**Related:** PR #3245 (`831a57149`, "fix(lan-discovery): stop wiping peer
identity on mDNS ServiceRemoved churn"), `docs/retro/retro-lan-diamond-vanished-after-self-peer-fix-2026-09-06.md`
(a different bug, same file, same shape of lesson — see below).

---

## TL;DR

`agentmux-srv/src/backend/lan_discovery.rs`'s `get_instances()` hid any LAN
peer whose `last_seen` was more than a **fixed 300 seconds (5 minutes)** old,
regardless of that peer's own actual mDNS re-announcement cadence. The
constant was introduced in PR #3245 (merged 2026-09-15) and was tuned against
live logs from exactly one peer (`gamerlove`), which happens to re-announce
every 1-2 minutes. Two other real, healthy peers on the same LAN (`narko`,
`starpower`) re-announce roughly every 40-46 minutes instead — comfortably
inside the mDNS-standard 75-minute TTL for PTR/TXT records (RFC 6762 §10;
`mdns-sd` 0.12 hardcodes this as `DNS_OTHER_TTL = 4500`), but 9x past the
5-minute cutoff. Every observer on the LAN saw those two peers appear for a
couple of minutes right after each re-announcement, then vanish for the rest
of the ~40-minute gap, on a loop — while `gamerlove`, whose own cadence
happens to fit inside the window, stayed continuously visible the whole time.

Fixed by keying staleness on **each peer's own advertised TTL**
(`ServiceInfo::get_other_ttl()`, captured at each `ServiceResolved`) instead of
one constant sized for the fastest peer anyone had happened to look at.

---

## What was actually verified

Measured directly against this instance's (`Area54`, v0.56.2) srv log, not
inferred:

1. **`narko` and `starpower` are real, healthy peers, not a network or
   config problem.** `narko` resolves and pings fine at the network layer
   (`192.168.1.230`, wired) and was independently confirmed by its own
   operator to see the exact same asymmetric pattern from its side (only
   seeing `gamerlove` continuously, not `Area54` or `starpower`) — ruling out
   anything specific to one host's NIC, firewall, or Wi-Fi power management
   (`starpower` is Wi-Fi, `narko` is wired; the pattern is identical on both).
2. **The re-announcement gaps are real and synchronized, not random flakiness.**
   Grepping this instance's srv log for `narko`/`starpower` `"LAN peer
   discovered"` lines across a multi-hour window showed both appearing in
   tight 2-3-sighting bursts at almost the same timestamp every time —
   `03:44:00`/`03:44:07`, `04:30:45`/`04:30:45` (identical to the second),
   `05:18:59`/`05:18:59`, `06:12:26`/`06:12:26`, `06:52:15`/`06:52:15` — each
   burst ~42-46 minutes after the last, then total silence in between.
   `gamerlove` shows no such gaps anywhere in the same log.
3. **`get_instances()`'s staleness filter, not the mDNS layer, is what made
   them disappear between bursts.** `agentmux-srv/src/backend/lan_discovery.rs`
   (pre-fix): `LAN_PEER_STALE_TIMEOUT_SECS: u64 = 300`, applied identically to
   every peer in both `get_instances()` and the periodic GC loop that prunes
   `self.instances`. A peer whose true re-announce gap is ~46 minutes
   necessarily reads as "stale" for all but the first few minutes after each
   announcement.
4. **The 5-minute figure was never meant to be universal.** PR #3245's own
   commit message explains it was picked because live logs "showed [`gamerlove`
   specifically] firing [`ServiceRemoved`] on ordinary TTL churn seconds
   before it re-announced" with "1-2 minute re-fire gaps" — the fix solved a
   real, different bug (identity getting wiped by a blank-TXT re-resolution
   after a premature delete), but the specific number chosen for the new
   staleness window was implicitly calibrated to the one peer in the log at
   the time, not to the mDNS protocol's actual guarantees.
5. **The protocol already specifies the real number, and it's not 300s.**
   RFC 6762 §10 recommends a 75-minute (4500s) TTL for "other" (PTR/TXT)
   records specifically because these describe long-lived services that don't
   need frequent refresh traffic; `mdns-sd` 0.12 (this repo's dependency,
   confirmed via its `service_info.rs` source) hardcodes exactly that value as
   `DNS_OTHER_TTL` and exposes no public API in this version to override it.
   A querier is expected to treat a record as good for its full advertised
   TTL, refreshing proactively before expiry (RFC 6762 §5.2's 80/85/90/95%
   re-query schedule) — not to invent a shorter, unrelated expiry of its own.

---

## Root cause

Two correct-in-isolation decisions, whose interaction was never considered —
the same shape as the 2026-09-06 LAN-diamond retro this file sits next to:

- **PR #3245** needed *some* answer to "how stale is too stale," because it
  could no longer rely on `ServiceRemoved` to signal a real departure (that
  event fires on ordinary churn too — see that PR's own rationale). It picked
  a single fixed constant, verified against the one peer visible in the logs
  at the time.
- **Real LAN peers legitimately re-announce on wildly different cadences** —
  anywhere from ~1 minute (`gamerlove`) to ~46 minutes (`narko`, `starpower`),
  both well within what RFC 6762 considers a perfectly healthy, unremarkable
  PTR/TXT TTL cycle. A constant tuned against the fast end of that range is
  wrong for everything slower.

Because the only peer in the debugging session that produced #3245 happened to
be a fast re-announcer, the mismatch was invisible in that PR's own testing —
the same "a fix's blind spot is whatever wasn't in the log at the time" shape
as the earlier retro.

---

## Why this is worse than "one peer flickers"

Every consumer of `LanDiscovery::get_instances()` inherited the same
5-minute blind spot: `GET /agentmux/discovery` (and therefore `DiscoverAgents`,
the tool an agent uses to pick a valid `SendMessage` target), the status-bar
LAN indicator, and the Warden LAN peer table. On a LAN with more than one
peer whose re-announce cadence differs, the *set of peers you can currently
see* rotates almost arbitrarily depending on when you happen to look — a peer
that is completely healthy reads as unreachable for the majority of every
40-plus-minute cycle. `SendMessage` addressed at such a peer would silently
fall through to the cloud-relay queue instead of routing over LAN, for no
reason related to the peer's actual availability.

---

## The fix

`LanInstance` gains a new field, `other_ttl_secs: u32`, captured from
`ServiceInfo::get_other_ttl()` on every `ServiceResolved` (unconditionally,
like `address`/`port` — it's a DNS record TTL, not TXT payload content, so
it's always present even on the blank-TXT re-resolutions this file already
works around elsewhere). Staleness is now `peer_staleness_window_secs`, which
uses **that peer's own advertised TTL**, clamped between the old 300s value
(a floor against a peer advertising an implausibly small TTL) and a new 9000s
ceiling:

```rust
fn peer_staleness_window_secs(other_ttl_secs: u32) -> u64 {
    (other_ttl_secs as u64).clamp(LAN_PEER_STALE_TIMEOUT_FLOOR_SECS, LAN_PEER_STALE_TIMEOUT_CEIL_SECS)
}
```

**The ceiling was added after review** (ReAgent P1 on PR #3301): the first
version of this fix only floored `other_ttl_secs`, with no upper bound.
That field comes directly from a peer's own mDNS advertisement, which this
file already treats as adversarial input elsewhere (`LAN_AGENT_NAMES_MAX_*`,
a few lines above). Without a ceiling, a spoofed peer advertising `other_ttl`
near `u32::MAX` (~136 years) would have been honored by both
`get_instances()` and the periodic GC, permanently pinning it as "alive" and
defeating the entire self-pruning design this fix exists to restore. 9000s
(2x the RFC/mdns-sd default) comfortably covers every legitimate cadence
observed on this LAN with margin, while bounding untrusted input the same
way the file's existing byte/count/length caps do.

Both call sites (`get_instances()` and the periodic GC loop) now key off this
per-peer window instead of the single constant. Since `mdns-sd` 0.12 exposes
no public way to set a custom TTL, every peer observed today will resolve to
the same 4500s default in practice — but the fix reads the real advertised
value rather than re-guessing a second fixed number, so it stays correct if
a future crate version, or a peer running different software, advertises a
different TTL.

An alternative — simply raising the fixed constant to, say, 4500s or higher —
was **rejected**: it would silently reintroduce the exact bug #3245 fixed
(a fast-departing peer staying "present" for up to 75 minutes after it's
actually gone), just with a worse number picked for a different single case.
Tying the window to each peer's own TTL is the only option that is correct
for both `gamerlove`-shaped fast peers and `narko`/`starpower`-shaped slow
ones at the same time.

---

## Lessons

1. **A timeout tuned against "the logs I have open right now" is tuned
   against one sample, not the population.** #3245's number was correct for
   the peer that happened to be visible during that debugging session and
   silently wrong for every peer with a different cadence — which is most of
   them, per the protocol's own recommended default.
2. **When the protocol already specifies an authoritative number (a TTL),
   prefer reading it over guessing a replacement.** The "right" fixed number
   was available the whole time, on every resolved record, via a public getter
   that was already a two-line addition away.
3. **Symmetric, synchronized failure across independent observers is a strong
   signal against "flaky hardware" and for "shared, deterministic mechanism."**
   `narko` and `starpower` re-appearing at the same wall-clock second, on both
   wired and Wi-Fi links, ruled out network-medium theories in about one
   comparison and pointed straight at a receiver-side computation instead.
