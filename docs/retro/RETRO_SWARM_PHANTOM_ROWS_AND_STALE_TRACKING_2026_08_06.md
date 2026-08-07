# RETRO — Swarm Phantom Rows, "No Activity Yet" Ghosts, and Missing Live Visibility

**Date:** 2026-08-06
**Severity:** Regression, cross-instance (confirmed on a second machine, different OS)
**Status:** Root cause hypothesized from source, NOT yet live-confirmed — needs a repro pass before any fix lands

---

## Symptoms observed this session

1. **A live-spawned Task-tool subagent (Manoz's own test spawn, ~12s runtime, completed successfully) was not observed in the Swarm view during its run.** Not yet confirmed whether it truly never rendered, or rendered too briefly / in a spot not checked — see Open Questions.
2. **Rows whose only visible content is "No activity yet"** appear in the Swarm tree as if they were real entries, when the entry should not be rendered at all.
3. **A row displaying the literal placeholder name "Agent"** (not a real agent's name) was seen — and critically, this was independently observed **on a second AgentMux instance, on a different OS**, ruling out "leftover state from one specific Windows dev session" as the sole explanation.
4. **No auto-expiry:** completed rows linger indefinitely until manually dismissed (separately tracked — see `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`). Not itself a bug, but it's why symptom 2/3's phantom rows don't just quietly age out — they sit in the tree forever, which is how they got noticed.

---

## Investigation trail (this session)

### Ruled out: "Agent" fallback logic itself is not new

`swarm-model.ts`'s `agentName` fallback:
```ts
const agentName =
    (block?.meta?.["agentName"] as string | undefined)?.trim() ||
    "Agent";
```
`git blame` dates this to 2026-06-22 (commit `5312bd8964`), untouched since. This is not a recently-introduced bug — it's an old fallback that's now visibly firing for input it wasn't firing for before (or wasn't firing for as noticeably, absent any auto-expiry to clear it).

### Ruled out: today's auth-work PRs don't create new tracked blocks

Checked diffs for PR #2431 (isolated-auth-by-channel defaults) and PR #2436/#2425 (tier-1 PTY capture + `AgentAuthPanel` bottom-dock move) — neither touches block registration, `AgentTrackedBlocksCommand`, or any subagent-tracking code. `AgentAuthPanel` is a pure UI sibling reusing the existing agent pane's own status; it registers no new block. These are not the cause.

### Confirmed: `agentName` IS set at normal agent-spawn time

`agentmux-srv/src/server/app_api/agent_open.rs:303`: `meta.insert("agentName".to_string(), json!(&agent.name));` — a normally-opened agent block always gets a real name. The fallback can only fire for a block that either (a) was never opened through this path, or (b) is being referenced before this registration completed.

### Leading hypothesis: orphaned `parent_block_id` references from stale subagent/dispatch records

`swarm-model.ts:buildTree()`:
```ts
const parentIds = subagents.map((s) => s.parent_block_id).filter(Boolean);
const allBlockIds = [...new Set([...blockIds, ...parentIds])];
```
Every currently-active subagent's `parent_block_id` is unconditionally added to the set of rows to render — **even if no real, currently-registered block exists under that ID.** When that happens, `WOS.getWaveObjectAtom("block:${blockId}")` resolves to nothing, `block?.meta` is `undefined`, and the row falls back to the literal `"Agent"` — rendered as its own top-level tree node, not merged into any real parent, with whatever (possibly empty) subagent/dispatch rows happen to share that same stale `parent_block_id`.

If the associated dispatch has zero real events (e.g. the parent's own registration never completed, so no activity was ever actually attributed to it, or the record is a leftover from a run that never truly started), expanding it shows nothing but **"No activity yet"** — symptom 2 and symptom 3 are very likely **the same underlying bug wearing two different faces**, not two unrelated ones.

The one documented cleanup path for this class of thing — `prune_block` / `prune_block_and_notify` (`agentmux-srv/src/backend/subagent_watcher/mod.rs:588,674`) — is triggered by `BlockDeleted`/`TabDeleted`/`WorkspaceDeleted`. A block that **never finished opening in the first place** (crashed mid-registration, or a race where subagent-tracking started before the parent's own `agent_open` handler completed) was never "deleted" — there's nothing to prune it, because from the backend's point of view it was never fully created. `reconcile_stale_subagents` (`scan.rs:100`) downgrades an individual subagent from `active`→`abandoned` when its parent's turn is confirmed idle, but that's a subagent-status transition, not a "this parent_block_id was never real" cleanup — it doesn't address the phantom-parent case at all.

**This would explain the cross-instance reproduction**: it's not machine-specific leftover state, it's a gap in a shared code path that any instance can hit whenever a block's registration doesn't complete cleanly (which is more plausible on some runs than others — crash, race, network agent connection drop — hence why it wasn't constantly visible before).

### Not yet investigated: why the live test subagent wasn't seen

Manoz's own test spawn (Task tool, general-purpose agent, ~12s, completed with a real result) was reportedly not seen in Swarm. Two live hypotheses, not yet distinguished:
- **(a)** It rendered correctly but too briefly / was missed by the observer, OR
- **(b)** It never rendered at all, for a reason distinct from the phantom-row bug above — e.g. `AgentTrackedBlocksCommand`/`subagent:spawned` event timing, or the specific block-scoping this Swarm pane instance uses.

This needs a **live, watched repro** (spawn a subagent while actively looking at the Swarm pane, ideally with `muxspect`/`muxlog` open alongside) before concluding it's the same bug — it would be a mistake to fold this into the phantom-row root cause without confirming it, per this session's own debugging discipline (root cause before patch).

---

## What's NOT the cause (ruled out with evidence, not assumption)

- Not the `"Agent"` fallback string itself (pre-existing, unchanged).
- Not today's merged auth PRs (#2425, #2431, #2436) — verified via diff, no block-registration code touched.
- Not missing `agentName`-setting logic at spawn time — it's set correctly when the normal path completes.

## What's still open

- [ ] Live-reproduce the phantom `"Agent"` row with `muxspect`/`muxlog` attached to catch the exact `parent_block_id` and confirm no matching `block:<id>` WOS object exists for it.
- [ ] Live-reproduce a "No activity yet" top-level row and confirm it shares a `parent_block_id` with a phantom `"Agent"` row (would confirm the unified-root-cause theory above).
- [ ] Live-reproduce the missing-test-subagent symptom with the Swarm pane actively watched, to determine if it's the same bug class or a separate one.
- [ ] Determine why a block's registration would fail to complete on two independent instances/OSes closely enough in time to both notice it this week — is there a recent shared-code change (even one not touching block-registration directly) that increased the *rate* of incomplete registrations, e.g. a timing change elsewhere that shifted a pre-existing race window?

## Proposed next steps (not started)

1. **Frontend defensive filter**: in `buildTree()`, skip rendering a block row entirely (not just fall back to `"Agent"`) when `block` itself is `undefined` — i.e. no WOS object exists for that ID at all — rather than rendering a placeholder for a block that structurally isn't there. This directly fixes symptom 2/3's *visibility* even before the backend-side root cause is fixed, though it wouldn't explain why the phantom entries exist in the first place, only stop showing them.
2. **Backend**: add a cleanup path for a `parent_block_id` that has never resolved to a real block after some bounded grace period (mirroring `reconcile_stale_subagents`'s existing pattern, but for "phantom parent" rather than "stale active subagent status").
3. Once root cause is confirmed (not just hypothesized), land the fix, THEN implement `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md` — landing the countdown first would just make phantom rows disappear-after-60s instead of disappearing-never, which is not the actual fix.

## Related

- `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md` — the separately-tracked auto-expiry gap.
- `SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md` — original two-bucket row design this bug lives inside.
- `SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md` — the existing stale/abandoned reconciliation this gap sits adjacent to but isn't covered by.
