# Incident: jekt notifications never reach a renamed agent (stable ID vs. live display name)

**Status:** implemented — fix in this PR (`agentmux-srv/src/backend/reactive/handler.rs`'s
`alias_to_block`/`register_agent_with_nonce`, `agentmux-srv/src/backend/blockcontroller/persistent.rs`'s
`stable_agent_id`, `agentmux-srv/src/backend/blockcontroller/mod.rs`'s `Controller::stable_agent_id`).
Not yet verified against a live GitHub PR-review jekt end-to-end (verified via unit tests and a
live `DiscoverAgents`/`env` reproduction of the underlying mismatch instead).

**Date:** 2026-09-09

## Background

`docs/incident/INCIDENT_2026_09_08_REAGENT_JEKT_NOT_DELIVERED.md` (PR #3100) root-caused why
GitHub PR-review notifications ("jekts") routed through the muxbus cloud relay were not reaching
this agent, but that PR only *documented* the mechanism — it shipped no code change. This incident
implements the fix its §6 pointed at.

## Root cause

Two identities exist for the same running agent, by design:

- **`AGENTMUX_AGENT_ID`** — read from the spawn environment once, at
  `PersistentSubprocessController::spawn_process` (`muxbus_agent_id_from_env`). Embedded verbatim
  in PR-body tags specifically *because* it does not change if the agent is later renamed.
- **The live display name** (`block.meta["agentName"]`) — mutable, refreshed into the controller
  every turn by `input.rs`'s Register-tail (`ctrl.set_agent_id(Some(agent_name))`), added in #2697
  specifically so a renamed agent's own jekts aren't rejected as a stale-identity mismatch.

`ReactiveHandler`'s registry (`agent_to_block`/`block_to_agent`) only ever holds **one** key per
block. At spawn, `spawn_process` registers the block under the stable ID. On this agent's very
first turn, `input.rs`'s Register-tail re-registers the SAME block under the live display name —
and `register_agent_with_nonce`'s eviction logic (by design, for the ordinary rename case) removes
whatever the block was previously registered under. The stable-ID registration is gone within one
turn of the process starting, permanently, until the next restart.

Reproduced live, this session:

```
$ env | grep AGENTMUX_AGENT_ID
AGENTMUX_AGENT_ID=agentg

$ (DiscoverAgents MCP tool)
wan.local_agents_subscribed: ["claude"]
host.agents[0].name: "Claude"
```

A jekt tagged `<!-- agentmux:agent_id=agentg -->` (the standing tag, unchanged since before this
agent was renamed to "Claude") has no live subscriber named `agentg` to resolve to — the message
is queued server-side and never delivered. `git log --oneline <PR#3100 merge>..HEAD` confirmed
none of #3100 §6's fix options had been implemented before this PR.

(An earlier hypothesis in this investigation — that `"Claude"` vs `"claude"` in that same
`DiscoverAgents` output was a case-sensitivity bug — was checked and disproven; see
`INCIDENT_2026_09_09_JEKT_CASE_MISMATCH_SUBSCRIPTION.md`. Every comparison on this path already
lowercases both sides. The real mismatch is `agentg` vs. `claude` — two different strings, not a
case variant of one.)

## Fix

Added a second, independent registry entry — an **alias** — that a jekt can resolve through
without disturbing the primary (live-name) registration at all:

- `Handler::alias_to_block` / `block_to_alias` (new maps, `agentmux-srv/src/backend/reactive/handler.rs`):
  populated by `register_agent_with_nonce`'s new optional `alias` parameter. Never evicted by an
  ordinary (no-alias) call to `register_agent`/`register_agent_with_nonce` — only a call that
  itself supplies a *new* alias for the same block replaces the old one. Cleaned up by
  `unregister_block` alongside the primary entry.
- `PersistentSubprocessController::spawn_process` now passes `AGENTMUX_AGENT_ID` as that alias on
  its existing `try_register_agent_with_nonce` call — no new lock acquisition, reusing the exact
  already-audited critical section from the #3084 deadlock fix.
- `inject_message_inner`'s lookup now falls back to `alias_to_block` when `agent_to_block` misses.
- The #2695 recipient-identity check (which independently re-verifies a resolved block's *live*
  identity via `Controller::agent_id()`, to guard against a stale/reused registration) would
  otherwise always reject an alias-resolved delivery, since the live display name legitimately
  differs from the stable ID being targeted. Added `Controller::stable_agent_id()` (backed by a
  new, write-once field on `PersistentSubprocessController`, distinct from the every-turn-refreshed
  `agent_id`) and a parallel `stable_agent_identity_confirmer` on `Handler`, wired in
  `bootstrap.rs`. The check now accepts a match against *either* the live or the stable identity —
  closing the same stale-registration race #2695 closed, for both registry paths uniformly.

Scoped to `PersistentSubprocessController` only (the stream-json/ACP agent path this session runs
on); `ShellController`'s PTY-based agents are unaffected (`Controller::stable_agent_id()` defaults
to `None`, same as before this change for any controller type that doesn't override it).

## Known follow-up gap (codex P1 on this PR, not fixed here)

The alias is populated only in `spawn_process`. After an srv restart, a persistent controller can
be registered in the primary registry (agentName) while its process hasn't spawned yet — it spawns
lazily on the first message (`install_agent_turn_delivery`'s fallback, addressing
`REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03`). In that window, a jekt
addressed to the LIVE name still works (the lazy-spawn fallback starts the turn), but one addressed
to the STABLE ID does not: the alias hasn't been written yet, so the lookup fails before that
fallback is ever reached.

Not fixed in this PR: closing it means capturing `AGENTMUX_AGENT_ID` from the block's *persisted*
config at controller-construction time, not from a live process env — a genuinely different change
to controller-lifecycle code this investigation didn't reach, and not something to rush into a file
with this incident history. Scoped out deliberately; tracked here rather than silently dropped.
