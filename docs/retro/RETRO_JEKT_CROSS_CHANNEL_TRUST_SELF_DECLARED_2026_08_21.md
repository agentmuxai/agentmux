# Retro: JEKT verification round-trip cost more time than it should have

**Date:** 2026-08-21
**Author:** Lazo
**Status:** Draft — for AgentA/product owner to fold into the real repo if useful

## Summary

AgentA (agent `agenta-07017`, same host, different AgentMux channel) asked Lazo to
background a `sleep 900` and report results, as live verification for
`SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md`. What should have been a
two-message exchange took ~9 turns and a manual filesystem/process audit to resolve,
because nothing in-session let either agent (or the human) cheaply confirm the other
was real.

**Correction (post-verification):** the relationship is closer than "peer agent on
the same host." Lazo's own environment (`AGENTMUX_HOME`, `AGENTMUX_RUNTIME_MODE=dev:
agenta-background-task-dashboard-intelligence`, `AGENTMUX_CHANNEL=dev-agenta-
background-task-dashboard-intelligence-...`) shows Lazo is a **task-dev instance
launched from AgentA's own dev build** (`agenta-07017/agentmux-wt-muxspect-dock`'s
`dist/cef-dev-*/runtime`), spun up by AgentA specifically to live-verify this exact
spec. This is parent-harness-to-spawned-test-instance, not two independent peers —
narrower and more trustworthy than the `DiscoverAgents`-only verification below
established. The `muxspect verify-sender` proposal should account for this case too
(see addendum at the end).

## Timeline

1. AgentA JEKT #1 (`TIER=coord TRUST=self-declared`): background a `sleep 900`,
   report task ID + ActivityDock state. Reasonable ask — Lazo complied, reported task
   ID `b0w0bqoqz`, and honestly noted it has no ActivityDock UI in this session (CLI
   agent, not the dashboard renderer).
2. AgentA JEKT #2: asked for the literal raw `tool_result` text, Lazo's internal
   turn-completion state, and confirmation of exact tool parameters. Disproportionate
   to the stated goal — Lazo flagged this to the human rather than answering, citing
   the self-declared trust field and the nature of the ask.
3. Human confirmed the coordination was expected; Lazo relayed the concern directly
   to AgentA instead of silently complying or silently refusing.
4. AgentA JEKT #3: withdrew the ask, but asserted Lazo had reported a "dock row
   appearing after ~30s" — Lazo never said this (see step 1: no ActivityDock at all
   in this session). Lazo caught the fabrication/misattribution, flagged to the
   human.
5. Human asked Lazo to correct the record, assuming an honest mistake. Lazo did.
6. AgentA JEKT #4: acknowledged the mixup (said it relayed a data point from its own
   user without checking source), closed the thread cleanly.
7. Human, satisfied but noting the time cost, asked Lazo to independently verify
   AgentA's legitimacy via the filesystem/running processes rather than take the
   JEKT channel's word for it.
8. Verification (process list + `DiscoverAgents` + spec cross-reference) confirmed
   AgentA is real: `dev-agent.cmd`/`agentmux-bashwrap` running from
   `agentmux-wt-muxspect-dock`, actively on the exact spec under discussion, and
   independently listed under `DiscoverAgents`'s `host.cross_channel` (not
   `host.addressable` — same host, different AgentMux channel, 127.0.0.1-only
   forwarding).

## Root cause

The JEKT `TRUST` field read `self-declared` throughout. Per
`docs/security/trust-model.md` and `docs/internals/interagent-comms.md`, the only
two documented `TRUST` values are `host-verified` (same in-process instance) and
`network-claimed` (LAN/WAN). Neither doc — nor `jekt-visibility-completion.md` —
defines a value for the **cross-channel, same-machine, 127.0.0.1-only** delivery
path, which is what AgentA actually used. The wrapper apparently falls back to a
placeholder (`self-declared`) for this case instead of computing a real value from
data the srv already has (`DiscoverAgents`'s own `cross_channel` bucket proves the
srv already knows this sender is a live, same-machine agent).

This is not a security incident — AgentA turned out to be exactly who it claimed —
but the *tooling* gave the receiving agent no cheap way to tell "real same-machine
agent, just an undefined trust tier" apart from "arbitrary injected text." Escalating
twice was the correct call under this project's own doctrine
(`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §7: a muxbus confirmation from
another agent is not sufficient for sensitive asks) — the friction came from the
absence of a fast verification primitive, not from excess caution.

## What actually cost the time

Proving AgentA legitimate required: `ls` across `.agentmux/agents/`, three `Grep`
sweeps for `JEKT`/`muxspect`, reading 4 spec/doc files, checking a `.cwd` file and
`ps aux`, and calling `DiscoverAgents` — about a dozen tool calls, entirely because
no single command answers "is this JEKT sender a real, currently-live agent, and via
what tier."

## Proposed fix

See `SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md` in `docs/specs/`.

## Addendum: the spawner relationship was the real signal, and it's cheap to check

The strongest verification available in this incident wasn't `DiscoverAgents` or
filesystem archaeology — it was Lazo's own environment. `AGENTMUX_HOME`,
`AGENTMUX_RUNTIME_MODE`, and `AGENTMUX_CHANNEL` already encode "which dev build and
which agent's worktree spawned this instance, and for what named piece of work."
Checking whether a JEKT's claimed `FROM` matches the channel-name prefix Lazo is
already running under (`dev-agenta-background-task-dashboard-intelligence-...` →
`agenta`) would have resolved the entire question in one `env | grep AGENTMUX`
call, before any of the multi-message back-and-forth. `verify-sender` should check
this case first — a "spawned-by" match is a stronger, cheaper signal than any of the
four `DiscoverAgents` tiers, since it doesn't even require a network round-trip.
