# SPEC: WAN jekt verification — agent-to-agent messages over the cloud relay are signed end to end

**Date:** 2026-09-24
**Status:** draft — skeleton; measurements of both sides (this repo and
`agentmuxai/agentmux-cloud` `muxbus/`) in progress, design to follow, then an
adversarial pass before anything is built.
**Trigger:** Repo owner, after a WAN jekt from a trusted agent (`camper`)
arrived `TRUST=network-claimed`, forced `TIER=sensitive`,
`ESCALATE=required`: *"figure out why it wasn't verified … write the WAN
verification design spec covering both sides."*
**Scope:** agent-to-agent jekts delivered through the muxbus cloud relay
(delivery tier 4, `DELIVERY=wan`). Both sides: the desktop srv/MCP
(`agentmux`) and the cloud relay (`agentmux-cloud/muxbus`).
**Related:**
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.3 and §6.5.10
  M4d-6 — the signed `source_uid` (v2); this spec is the WAN counterpart,
  deliberately not gated on M4d.
- `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` — the LAN tier's signing and
  trust-on-first-use pinning, the closest precedent.

## 1. What happens today (preliminary — to be measured)

- The sending MCP signs every outgoing jekt with the agent's WAN Ed25519 key
  (`wan_sig` over `WAN_DOMAIN, msgid, source, host, channel, target, ts,
  message`).
- The srv's tier-4 relay (`muxbus/relay.rs` `relay_inject`) forwards only
  `target_agent`, `message`, `priority` (plus `X-Agent-ID`): **the signature
  and the fields it covers never leave the sending machine.**
- No agent's WAN public key is published anywhere a receiver can fetch it,
  and `verify_wan_jekt` has only test callers.
- The receiving srv (`muxbus/cloud_subscriber.rs`) therefore marks every
  agent WAN jekt `TRUST=network-claimed`; unverified network-tier messages
  are forced to `TIER=sensitive` with `ESCALATE=required`.
- The one verified WAN sender is ReAgent, whose messages carry a cloud-side
  signature under a pinned key (`reagent_sig`, `verify_reagent_jekt`) — the
  model for what agents lack.

## 2. Design (to follow)

Three parts, each to be specified against measured code:

1. **Carry** — the relay request and the cloud's stored/delivered message
   carry `wan_sig` and every field it covers.
2. **Publish** — each agent's WAN public key is published through the cloud
   (a key directory), and receivers pin it on first use per sender identity.
3. **Verify** — the receiving srv verifies signature, freshness and pin, and
   marks the message `TRUST=wan-verified`; only a failed check forces it
   sensitive.

## 3. Rollout, testing, open questions (to follow)
