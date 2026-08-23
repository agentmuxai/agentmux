# SPEC: `FleetBroadcast` reaches cross-channel/LAN/WAN targets

**Date:** 2026-08-22
**Status:** Implemented
**Author:** Korp
**Repo touched:** `agentmux` (`agentmux-mcp/src/main.rs`)
**Diagnosis:** `docs/reports/REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md` §3.1

## 1. Problem

`FleetList`/`DiscoverAgents` (a thin passthrough of `/agentmux/discovery`)
correctly return entries across all four tiers: `host.addressable`,
`host.cross_channel` (a different channel on this same host), `lan`, and
`wan.subscribed_agents`. `FleetBroadcast`'s own doc comment tells a caller
to "get these from FleetList" for its `targets` array — but its actual
`block_id -> agent_name` resolution (`agentmux-mcp/src/main.rs`) only ever
read `host.addressable`. A `host.cross_channel` target — which genuinely
carries a `block_id` in the same namespace, just under `name` instead of
`agent_id` — silently failed with "no registered agent for this block,"
even though the exact block_id FleetList handed back is real and live.

`lan`/`wan` entries are a different case: they carry **no `block_id` at
all** — `LanInstance.agents: Vec<String>` and
`wan.subscribed_agents: Vec<String>` are both plain agent-name lists, since
those agents aren't local blocks in this instance's own namespace. There's
no block_id to "resolve" for them; the only valid target representation
for those tiers is the agent's name.

## 2. Fix

- `host.cross_channel` entries are folded into the SAME
  `block_id -> agent_name` map `host.addressable` already builds (extracted
  into a new pure `build_block_to_agent_map(discovery: &Value)` helper,
  unit-tested without a live discovery endpoint).
- Any target string that doesn't match an entry in that map now falls
  through to being used **as a literal agent name** directly, instead of
  failing immediately. This is what makes LAN/WAN reachable: a caller who
  read a `lan`/`wan` agent's name off `FleetList`'s output (no block_id
  exists to pass instead) can pass that name straight into `targets`, and
  `/agentmux/reactive/inject`'s own cross-tier cascade
  (cross-channel → LAN → WAN muxbus relay, already used successfully by
  `SendMessage`) resolves it.
- Safety: this can't make a genuinely-wrong/stale block_id succeed
  silently. A string that isn't a real block_id AND isn't a real agent name
  still fails — just via the inject endpoint's own "agent not found," one
  layer later than the old pre-check, with an equivalent error surfaced in
  `failed`.
- Tool description and the empty-`targets` validation error updated to
  document that `targets` accepts block_id (host/cross-channel) or agent
  name (LAN/WAN), matching what FleetList actually hands back for each
  tier.

**Deliberately not touched:** `fleet_broadcast_impl` (agentmux-srv,
`server/app_api/fleet.rs`) — the WS-RPC/Swarm-UI path. Per that module's
own doc comment, it's intentionally host-tier-only: it delivers with
`source_agent: None` (self-declared, a human clicking a UI button isn't
cryptographically claiming to BE any agent) and has no access to any
agent's `AGENTMUX_JEKT_KEY`, so it structurally cannot sign a jekt for
LAN/WAN delivery the way the trust model requires. Only the MCP tool's own
client-side path (which DOES hold the calling agent's key, exactly like
`SendMessage`) can extend cross-tier — that's this fix's actual scope.

## 3. Non-goals

- `FleetBulkStop` is a separate, deliberately-not-bundled follow-up
  (`SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md` or similar) — it's a
  destructive primitive with a different risk profile, and (per the audit)
  extending it past cross-channel into LAN/WAN needs a remote-stop
  primitive that doesn't exist anywhere in the codebase yet, not just a
  resolution fix.
- Does not change `/agentmux/discovery`'s response shape at all — this is
  purely a consumer-side fix in the MCP tool.

## 4. Testing

- `build_block_to_agent_map`: 5 new unit tests — resolves
  `host.addressable`; resolves `host.cross_channel`; merges both without
  dropping either; ignores `lan`/`wan` sections gracefully (no panic on
  their differently-shaped entries, and correctly resolves nothing from
  them); handles an empty/missing discovery response.
- Full `agentmux-mcp` test suite: 13 passed, 0 failed.
