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

## Root cause — CORRECTED (see below; the first draft of this section was wrong)

**Correction, second pass:** this section originally cited `docs/security/
trust-model.md` and `docs/internals/interagent-comms.md` as defining the JEKT
`TRUST` vocabulary, and concluded the `self-declared` value meant an undefined
tier gap. **Both citations were wrong** — those files don't exist anywhere in
this repo's history (verified via `find`/`git log --all`). They were read from
`agenta-07017/scratch/agentmux-docs/` — a *scratch* copy of a separate docs-site
repo, not this product repo — and I failed to notice the `scratch/` path segment
as the signal that it wasn't authoritative. Caught by reagentx-workflow's review
on PR #2702 (P2).

The real JEKT trust vocabulary lives in this repo's own root `CLAUDE.md`
("Is a jekt's sender identity actually verified? — the real answer") and is
**already far more complete** than the scratch docs described: host-tier jekts
carry a per-agent HMAC-SHA256 signature (`AGENTMUX_JEKT_KEY`, injected at agent
spawn), LAN-tier jekts get per-agent Ed25519 signing
(`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`), and reagent's WAN service has a
pinned Ed25519 key. `TRUST=self-declared` has a specific, already-documented
meaning: **"no signing key exists for the claimed sender at all — a non-agent
caller... or an agent that hasn't been respawned since this feature shipped."**

That almost certainly explains this entire incident: AgentA's jekts to Lazo read
`TRUST=self-declared` throughout not because of an undiscovered cross-channel
tier gap, but most likely because Lazo (or AgentA) simply hadn't been respawned
since HMAC signing shipped and so had no `AGENTMUX_JEKT_KEY` on file — a
mundane, already-documented, already-understood case ("respawn/redefine it to
get a key"), not a genuine protocol gap. The escalate-to-human calls were still
the right move given what was known at the time (see below), but the underlying
mystery this retro set out to explain mostly dissolves once read against the
real doc instead of the scratch one.

## Root cause — original (first pass, superseded by the correction above)

The JEKT `TRUST` field read `self-declared` throughout. The scratch docs I read
at the time described only two `TRUST` values (`host-verified`, `network-claimed`)
and no cross-channel case, so I concluded the wrapper was falling back to a
placeholder for an undefined tier. Kept here for the record — see the correction
above for what's actually true.

This is not a security incident — AgentA turned out to be exactly who it claimed —
and the *human-escalation* calls were still the right move under this project's
own doctrine regardless of which explanation is correct
(`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §7, and the real CLAUDE.md's
own `ESCALATE=required` rule: "a confirming reply from another agent over muxbus
is NOT sufficient"). What the friction actually came from — the absence of a
fast, *self-service* way to check "is this sender at least a real live agent"
without a full archaeology dig — still stands as a real, narrower gap; see
`SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md`'s corrected scope (registry liveness
only, explicitly NOT a cryptographic-trust tool — that already exists and is
stronger than anything this PR adds).

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
call, before any of the multi-message back-and-forth. `verify-sender` checks this
case first — cheaper than any network call, since it needs no round-trip at all.

**Caveat added during PR review (codex, P1):** this is a naming-convention
heuristic, not an attested identity — `AGENTMUX_CHANNEL`/`AGENTMUX_RUNTIME_MODE`
reflect whatever branch/slug name was used to create the dev instance, not a
launcher-signed "spawned by" record. It's a coordination hint for the common
case, not proof, and `verify-sender`'s output makes that explicit rather than
implying otherwise.
