# Retro: agent opens with no conversation history in a fresh build/channel

**Date:** 2026-06-16
**Author:** AgentX
**Severity:** P1 (history *appears* lost — it is intact but unreachable on open)
**Status:** Root-caused (from on-disk data + code; no data lost). Fix pending.

> **Correction kept on purpose:** my first pass blamed the registry's
> `session_id: null`. That's a **separate** gap (provider `--resume` / *continue*),
> **not** the display-history cause. Corrected after tracing the actual #1399/#1403
> read path. The honest trail matters more than a clean first guess.

---

## Summary

Closing **Naki** in a running **0.44.1** instance and reopening it in a freshly-built
**0.46.0 portable** showed an empty conversation. **No data was lost.** Naki's 32 MB
transcript is intact in the global agent zone `agent:f81f9785…:current`. The pane is
empty because the cross-channel read-fallback **can't anchor it**: the agent's global,
"agent-anchored" snapshot stores a **channel-local `sourceBlockId`**, and the fallback
only resolves the agent's zone by looking that block up *in the current channel*. In a
different channel that block doesn't exist → fallback skipped → empty render.

## Root cause (definitive)

Two facts on disk + one line of code:

**1. The global snapshot points at a foreign block.**
`~/.agentmux/shared/agents/transcripts/filestore.db`, zone `agent:f81f9785…:current`:
```json
output.state.json → { "schemaVersion":2, "savedAt":"2026-06-16T00:16:25Z",
                      "highWaterMark":1015, "sourceBlockId":"1cfdef4b-…736b6", … }
output            → 32,099,237 bytes   (the full conversation, present)
```
`sourceBlockId` is **not `""`** — it's `1cfdef4b…`, the block id of the *channel where
Naki last ran live*. The #1403 backfill deliberately seeded `sourceBlockId:""`; but
`writeSnapshotNow` (frontend `agent-view.tsx:384`) sets `sourceBlockId: model.blockId`
on every save, and the snapshot write **mirrors to the global zone**
(`agent_session.rs:178–180`). So the first live run (and the close) **clobbered the
`""` with a channel-local id** (`savedAt` 00:16 = when the instance ran today).

**2. The fallback can only anchor via a LOCAL block.**
`global_output_source` (`server/app_api.rs:1066`):
```rust
if local output for block_id is non-empty { return None; }     // normal path
let block = wstore.get::<Block>(block_id).ok().flatten()?;      // ← needs block LOCALLY
let zone  = agent_zone_for_block_meta(&block.meta)?;            // ← agent zone from its meta
gfs.stat(&zone, "output") …                                    // read global zone
```
It maps `block_id → agent zone` **through the block's local meta**. No local block → `?`
returns `None` → no fallback.

**The failure chain (open Naki in the fresh 0.46.0 channel):**
1. A fresh local block `X` is created (exists locally, meta → agent `f81f9785`).
2. The **snapshot** read is agent-anchored by `definition_id` (`read_session_state`,
   `agent_session.rs:223`) → it loads fine from the global zone.
3. The snapshot says `sourceBlockId = 1cfdef4b`, so restore reads the conversation from
   block **`1cfdef4b`**: `blockfile:read_range(block_id="1cfdef4b", "output")`.
4. `global_output_source("1cfdef4b")` → local output empty → `wstore.get::<Block>("1cfdef4b")`
   = **None** (that block lives only in the 0.44.1 channel) → fallback skipped.
5. Reads the empty local `1cfdef4b` output → **empty pane**.

If `sourceBlockId` were `""` (or block `X`'s id), step 4 would resolve `X → f81f9785 →
agent:f81f9785…:current` and render the 32 MB. That's exactly why the #1403 *seed*
worked and a *live run* breaks it.

## Why each observation fits

- **Worked in 0.44.1:** there `sourceBlockId 1cfdef4b` *is* the local block → resolves.
- **Empty in 0.46.0:** `1cfdef4b` is foreign → `wstore.get` None → no fallback.
- **"Closing it" mattered:** close runs `writeSnapshotNow`, writing the final
  `sourceBlockId=1cfdef4b` snapshot to the global mirror (the 00:16 `savedAt`).
- **Not archived / not lost:** only a `:current` zone exists (no `:archive:`); `output`
  is 32 MB; the archived-suppression branch isn't hit.
- **`session_id: null` is unrelated:** that's the provider `--resume` field, a different
  follow-up.

## Blast radius

**Any agent that has ever run live in a channel** now has a channel-local `sourceBlockId`
in its global snapshot → opening it in **any other channel** (a new portable, a new dev
branch dir, a cleared data dir) renders empty. The per-build data isolation
(`task package` → fresh channel) makes this hit on essentially every fresh build — which
is exactly this incident. The #1403 seed only protects agents that have **not** run since
the backfill.

## What went well / wrong

- **Well:** global-first storage meant zero real loss; root-caused from disk+code in one
  pass after correcting the first hypothesis.
- **Wrong:** the "agent-anchored" snapshot carries a **channel-scoped** field
  (`sourceBlockId`) into a **global** artifact, and the only reader of that field
  (`global_output_source`) requires the block to be local — a latent contradiction that
  shipped because #1399/#1403 verified the **seed** path (`sourceBlockId:""`) but not the
  **post-live-run** cross-channel path. No automated "run agent in channel A, open in
  fresh channel B → history renders" test.

## Fix direction (to implement next)

Preferred — **make the global mirror agent-anchored, not block-anchored:**
- When mirroring the snapshot to the **global** zone, normalize `sourceBlockId` to `""`
  (the local snapshot keeps the real id for same-channel restore). Then any channel reads
  it agent-anchored and `global_output_source` resolves via the *opening* channel's fresh
  local block. (Smallest, targeted change.)

Alternatives / belt-and-suspenders:
- In `global_output_source`, when `wstore.get(block_id)` misses, fall back to the agent
  zone derived from the **opening pane's** block/`definition_id` (pass a defId hint on the
  read_range/line_count RPC) instead of giving up.
- Frontend: on restore, if `snapshot.sourceBlockId` doesn't resolve locally, read from the
  current pane's block id (which does map to the agent).

Plus:
- **One-shot heal** for already-poisoned global snapshots: rewrite `sourceBlockId` → `""`
  for every `agent:*:current` `output.state.json` (idempotent migration).
- **Automated e2e:** run an agent in channel A, open in fresh channel B, assert render.
- (Separate) backfill registry `session_id` so `--resume`/continue also works.

## References

- `frontend/app/view/agent/agent-view.tsx:339–395` (`writeSnapshotNow`, `sourceBlockId = model.blockId`, foreign-block guard `snapshotIsForeignBlock`).
- `agentmux-srv/src/server/app_api.rs:1066` (`global_output_source` — local-block requirement) and `:1216` (`read_range` fallback).
- `agentmux-srv/src/backend/agent_session.rs` (`agent_current_zone`, `SNAPSHOT_FILE`, snapshot global mirror L178–180, snapshot read fallback L223).
- Evidence: `~/.agentmux/shared/agents/transcripts/filestore.db` zone `agent:f81f9785-6315-4d21-9a01-7f81a727bc17:current`.
