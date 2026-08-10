# Spec: pane close/reopen must guarantee conversation continuity, or say so

**Status:** implemented — PR #2323, extended by #2426 (resume outcome as transcript event); verified in code 2026-08-10.
**Author:** Agent1
**Date:** 2026-07-27
**Triggered by:** a user question about whether an agent's Armory-sourced workspace rules (Bundles/Skills/MCP) are "always available," which surfaced that reopening a pane goes through a resume (`--resume <sid>`) whose success is unverified end-to-end. Investigating that led to two real, evidenced gaps below.
**Related:** `docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md`, `docs/plans/PLAN_PANE_REOPEN_SESSION_RESUME_AND_STATS_BAR_2026_07_10.md` (commit `3e720422`, #2059 — closed the *cross-channel* half of the June retro), `docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` (the banner mechanism this spec extends), commit `c39911c2` (#2224 — the silent resume-poison recovery this spec makes visible).

---

## 1. The guarantee this spec establishes

**Closing an agent's pane and reopening it later must resume the same conversation. If that's not possible for any reason, the user must be told — not silently handed a fresh session that looks like a continuation.**

This is currently untrue in at least two concrete, evidenced ways (§3). Both fail *silently*: the user gets a working agent that responds normally, with no indication that its memory of the prior conversation is gone. That's strictly worse than an error, because nothing prompts the user to notice or recover.

## 2. What already works (don't re-solve this)

The June 2026-06-16 retro's headline scenario — reopening a pane in a **different channel/build** than the one the agent originally ran in, where the new block has no `agent:sessionid` meta at all — is fixed, verified by tests:

- `session_backfill.rs::backfill_session_ids` runs at every app startup (`bootstrap.rs`), scans `~/.agentmux/shared/providers/claude/projects/<encoded-cwd>/` (or the identity-bundle equivalent) for the registry's empty `session_id` fields, and picks the **largest** `.jsonl` file's stem — deliberately largest-, not newest-, to avoid a short orphaning session winning over the real conversation.
- `agent_open.rs` seeds a fresh block's `agent:sessionid` from that registry record, guarded by a same-agent concurrency check (`agent_live_elsewhere`) added alongside the same fix so two blocks can't `--resume` the same provider session at once.

Do not touch this mechanism. The gaps below are elsewhere.

## 3. What's actually broken, with evidence

### 3.1 Same-instance "close pane, reopen via My Agents" can hand the picker a stale/empty session id

`AgentPicker.tsx`'s `handleReattach()` — the live, wired reattach path — passes `continueSessionId: row.session_id ?? ""`, where `row` comes from `ListRecentSessionsCommand` (`agent_handlers/session.rs`). That handler **prefers the local `db_agent_instances` row over the registry record** whenever both exist for the same `(definition_id, instance_name)` key.

The local row's `session_id` is initialized to `""` at `instance_create` and — this is the actual bug — **never updated afterward in production**. The RPC that would keep it live (`updateagentinstance` / `UpdateAgentInstanceCommand`) is fully implemented and registered server-side but has **zero frontend callers** (confirmed by grep). Live per-turn session-id capture (`persist_session_id`, `blockcontroller/core.rs`) only writes the block's own `agent:sessionid` meta — which is destroyed the moment the block/pane is deleted — never the instance row or registry.

**Net effect:** any agent that has been launched at least once in the current running app has a local instance row with `session_id: ""`. Closing its pane and reopening via the "My Agents" list hands `handleReattach` that empty string, even though the disk-backfilled registry record might hold the correct session id. Left unverified in this pass whether the resulting spawn falls through to a fresh session or to `agent_open.rs`'s registry-seed path — **first implementation task is a live repro to confirm severity**, but the code path itself is unambiguous: the picker is reading a field nothing keeps current.

### 3.2 A failed `--resume` silently starts a fresh session with no user-visible signal

`persistent.rs`'s `poison_resume` / `try_capture_session_id` (added in `c39911c2`, #2224) handle the CLI reporting `"No conversation found with session ID: ..."` on stderr (e.g. after a config-dir/cwd mismatch breaks the `~/.claude/projects/<encoded-cwd>/` lookup `--resume` depends on). On that error, the code blanks the poisoned session id and lets the next respawn start genuinely fresh.

This is a reasonable *recovery* — better than looping on the same error — but the fix commit's own message frames the outcome as "starts a fresh conversation and gets a real reply instead of erroring again," and the only trace is a `tracing::warn!` in the host log. **No `AgentFailure` is raised, no wave event fires, nothing reaches the frontend.** The user asks a question, gets a normal-looking reply, and has no way to know the agent just lost all memory of the prior conversation.

The subprocess controller path (`blockcontroller/subprocess/session.rs`'s `hydrate_session_id_from_config`) has the same property — best-effort hydration, silently overwritten by whatever the CLI's own init event reports, no signal either way.

### 3.3 Compounding factor: the registry backfill never re-heals an already-set (now-wrong) record

`backfill_session_ids` explicitly skips any registry record that already has a non-empty `session_id` (idempotent by design, so it doesn't fight a legitimately-advancing conversation). But this means once 3.2 poisons and replaces a session, or once 3.1 launches a fresh session under a stale-but-non-empty id, the registry's record is wrong **forever** — nothing re-derives it from disk again. Any future cross-channel resume of that same agent inherits the wrong anchor.

## 4. Design

Two independent fixes, both scoped tightly to avoid re-touching the working cross-channel mechanism in §2.

### 4.1 Fix the picker's session id staleness (closes 3.1)

Preferred approach — make the local row authoritative instead of stale, since that's what `ListRecentSessionsCommand`'s existing "prefer local" logic already assumes is possible:

- Wire `persist_session_id` (`blockcontroller/core.rs`) to also call `UpdateAgentInstanceCommand`'s underlying `Store::instance_update_partial` when it writes the block's `agent:sessionid` meta, so the local instance row tracks the live session id the same turn it's captured — not just at block-meta level.
- This is the smallest change that restores the *intended* design (the June plan's Phase-1-item-1, "populate registry `session_id` on every turn," which #2059 shipped a disk-scan alternative to instead of the live-write — this closes that gap rather than reopening a design debate).
- Fallback/defense-in-depth: `ListRecentSessionsCommand` should fall back to the registry record when the local row's `session_id` is empty, instead of trusting an empty local value as "no session exists." Cheap, and correct regardless of whether 4.1's primary fix lands cleanly.

### 4.2 Surface resume failure/degradation to the user (closes 3.2, mitigates 3.3)

Extend the existing `AgentFailure` taxonomy (`docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md`, `failure-accessory.ts`) rather than inventing a new banner system:

- New failure class, e.g. `session_resume_failed` (naming TBD at implementation time — `resume_poisoned` or `session_continuity_lost` are alternatives). Fired from `persistent.rs`'s `poison_resume` path (and the subprocess-controller equivalent) via `persist_last_failure` (`core.rs`) the same way every other classified failure already reaches the frontend, instead of only `tracing::warn!`.
- Banner copy should be unambiguous that this is not a normal error: something like *"Couldn't resume the previous conversation — started a new one. Your prior conversation is still saved."* (Confirm the prior transcript really is still reachable — e.g. via the CLI's own session picker / `--resume` history — before finalizing this claim in copy; don't assert recoverability without checking.)
- Action affordance: at minimum a dismiss. A "View prior conversation" action (if the old transcript is independently viewable) would be a strong follow-up but is not required for this spec's guarantee — the guarantee is *disclosure*, not automatic recovery.
- Once 4.2 lands, also emit the same signal (or reuse the same event) at the moment 3.1's stale-empty-session-id causes a picker-driven reattach to spawn fresh instead of resuming — both are instances of the same user-facing promise being broken, and should look the same to the user regardless of which internal path caused it.

### 4.3 Registry self-healing (mitigates 3.3, lower priority)

Not required to satisfy the core guarantee (4.1 + 4.2 together already ensure either correct resume or visible disclosure), but worth tracking as a follow-up: once a poisoned/replaced session is flagged via 4.2, consider clearing the registry record's `session_id` back to empty so the next `backfill_session_ids` pass can re-derive the best available anchor from disk, rather than leaving a known-wrong value in place indefinitely.

## 5. Acceptance criteria

1. **Close/reopen in the same running instance, via the "My Agents" picker, resumes the real prior session** — verified by an e2e test: launch an agent, send a message, close the pane, reopen via `handleReattach`, assert the spawned CLI's `--resume` argument matches the session id from the first turn (not empty, not a newly-invented one).
2. **A resume failure (simulated: point `--resume` at a session id that doesn't exist) produces a visible `AgentFailure`/banner**, not just a log line — verified by a test asserting the wave event / `agent:last_failure` meta is set and a `PaneRow` action-map entry exists for the new class.
3. **No regression to the §2 cross-channel mechanism** — its existing tests (`largest_session_beats_a_fresh_short_one`, `backfill_fills_empties_picks_largest_and_is_idempotent`, the `agent_live_elsewhere` concurrency guard tests) continue passing unchanged.
4. **`ListRecentSessionsCommand`'s local-row session id is live**, verified by a test: launch, send a message, query `ListRecentSessionsCommand` without closing the pane, assert the returned `session_id` is non-empty and matches the block's own `agent:sessionid` meta.

## 6. Non-goals

- Does not change `--resume`'s underlying CLI behavior or attempt to make Claude Code re-read `CLAUDE.md`/project-memory on resume — that's a separate, currently-unverified question (see the research trail that triggered this spec) about the CLI's own internals, out of scope here.
- Does not build automatic prior-conversation recovery/merge UI — disclosure is the bar, not recovery, per §4.2.
- Does not address the subprocess-controller path's lack of an equivalent `poison_resume` detector (`hydrate_session_id_from_config` currently has no failure-mode detection at all, only best-effort hydration) — flagged as a real gap but the persistent/Claude controller is the primary path and should land first; the subprocess controller should get the same treatment as a fast follow-up once the failure-classification wiring exists.

## 7. Open questions for implementation time

- Confirm via live repro whether 3.1's stale-empty-`session_id` actually reaches a silent-fresh-spawn today, or whether some other guard incidentally catches it (the research pass flagged the code path as unambiguous but did not drive a live repro to confirm the end-to-end outcome).
- Confirm whether the prior transcript is genuinely still reachable/viewable after 3.2's silent replacement, before promising that in the banner copy.
- Decide the exact failure-class name and whether it needs its own icon or can reuse an existing one from `failure-accessory.ts`'s map.
