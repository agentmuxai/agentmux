# SPEC: srv signs LAN jekts for an agent that signed nothing

**Date:** 2026-10-25
**Status:** implemented in the PR that adds this file.
**Builds on:** `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` (the signed tuple, the receiver's pubkey lookup, trust-on-first-use pinning), `SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md` (srv signs at the origin for the agent its token names; this spec is the LAN half of it), `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` (the agent token, the v2 UID signature).
**Asked for:** a leader, after jekts from a Codex agent reached a LAN peer as `TRUST=network-claimed` while jekts from Claude agents on the same AgentMux arrived `lan-verified`.

## 1. Short answer

A jekt from an agent whose own process attached no LAN signature is now signed by srv, at the instance the agent sent it from, when the request's own agent token names the sender. The receiver checks exactly what it checked before.

## 2. What was seen

For the Codex agent's jekts, on the receiving LAN peer:

- the header read `TIER=coord DELIVERY=lan TRUST=network-claimed`, with no `SIG` field and the tier unchanged;
- the peer's log had no `did not verify`, `pubkey mismatch`, `rate-limited` or `pin store` line.

That is an unsigned jekt. A failed signature reads `network-claimed` too, but it forces the tier to `sensitive` and says `SIG=invalid`; a mismatched pin does the same and logs a warning.

On the sending instance the same agent was in order: its public key is served to peers (`GET /agentmux/reactive/agent?id=…` carries `lan_public_key`), it matches the key in the agent's workspace `.mcp.json`, and its sends reach srv attributed to its token (`audit_source_uid` set). So the receiver's lookup had a key to check against and nothing to check it with.

## 3. Why srv signs

The same reasons as for WAN (that spec §2): srv mints and keeps every agent's keys and already knows, from the token, who is calling. Signing in the agent's process only works when that process holds the key and the agent id, which depends on how the agent's CLI starts its MCP server; a path that fails to give it either sends unsigned, and no receiver-side check can repair that. A key srv holds in one place cannot drift.

## 4. Design

At the LAN forward (tier 3 of `deliver`), srv replaces the forwarded request with a signed copy when all of these hold:

1. **An identified agent sent it.** `caller_uid` is non-empty: this srv saw an agent token. A forwarded hop carries none, so a hop is never signed here.
2. **It is an origin request.** The request is host-tier. A request already labelled `lan`, `wan` or `channel` came through another srv.
3. **Nothing is signed.** An agent's own `lan_sig` is never replaced, and nothing is added beside a signature srv did not make.
4. **It sends as itself, and the name is its alone.** `actor::wan_signer`: the token's row has `source_agent` as its slug (ignoring case) and no other row's slug folds to the same name. A request naming another agent is not signed.

It then signs:

- **v1**, with the name-keyed LAN key (`agent_lan_key_ensure`), over the same material the MCP signs: message id, source, target, time, message;
- **v2**, with the token's UID-keyed LAN key, when the sender carried no UID of its own; the request then carries `source_uid` = the token's UID, as an agent's own v2 does.

A sender that sent no message id or time (a shell's `muxsh agent send`) gets both from srv, so the signature has something to cover.

Any unmet condition, or a missing key, forwards the request as it came: signing is best effort and never blocks delivery.

## 5. What does not change

- **Every receiver check.** The pubkey lookup, the trust-on-first-use pin, the freshness window, the replay reservation, the v2 pin and the forced `sensitive` tier on a bad signature are untouched. A forged or tampered jekt fails as before (test: a message changed after signing).
- **The trust a signature gives.** `lan-verified` means the sending srv vouches, with the agent's own key, that its token named that agent. The receiving side's pin still binds the name to the first key it saw.
- **Agents that sign for themselves.** Their signature is forwarded as sent.

## 6. Not covered

- **Why the agent's own process signs nothing.** This spec makes the absence harmless; it does not establish which of the agent's id, keys or token its MCP process lacks. The fix is the same either way.
- **A key mismatch against a pin** (a different symptom: `sensitive` tier and a warning on the receiver). That is a re-created agent, and re-pinning is a decision for the receiver's owner, never automatic.
- **Host-tier and cross-channel signatures** from such an agent: local delivery already attributes it by token (`jekt_sender_by_token`).

## 7. Tests

`server/tests/reactive/lan_v2_tests.rs`, module `origin`: what the origin signs verifies at the receiver through `verify_lan_signature`, with the UID claimed; a sender with no id or time still verifies; a tampered message fails; an agent's own signature is kept; hops, other tiers, other agents' names and missing tokens are not signed; a UID the sender carried is not overwritten.
