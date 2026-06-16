# Retro: cross-channel agent conversation continuity "fixed, then broke again" — all agents

**Date:** 2026-06-16
**Author:** AgentX
**Severity:** P1 (every agent opens to a fresh/short conversation cross-channel; original intact but orphaned)
**Status:** Root-caused. Display half fixed (#1472); continuity half + recovery pending.

> Companion to `retro-legacy-agent-history-cross-channel-2026-06-16.md` (#1471),
> which covered only the *display* half. This one covers the **full regression**:
> why it looked fixed and broke again, and the **session-continuity** half.

---

## Summary

After the #1403 backfill (2026-06-14) an agent opened in a fresh build/channel
**showed its conversation** — verified "renders on open." Now (2026-06-16) **every
agent opens blank**, and once you interact it **latches onto a brand-new short
session**; the original conversation (e.g. Naki: 6.28 MB provider session +
32 MB global output) is **intact but orphaned**.

It is **not a code revert**. It's that the #1399/#1403 fix was only ever verified
in its **freshly-seeded** state, and **normal use undoes it**. Two independent
mechanisms both fail the same way and mask each other:

## What the user saw

1. Opened agent (Naki) in the new 0.46.0 build → **blank** (display half).
2. Interacted → a **new short session** started.
3. Reopened → it **resumed the short session**, not the original — "the same for
   all agents… this was fixed before, now it's broken again."

## Why "fixed before, broken again"

`#1403` seeded each agent's global snapshot with `sourceBlockId: ""` and verified
agents *render on open* — but only in that pristine, never-run-since state. The
moment an agent **runs live**, both halves break:

### Mechanism 1 — display (snapshot poisoning) — *since #1399*
`write_session_state` (added in #1399, `204fcf07`) mirrors the snapshot to the
global zone **verbatim**, including `sourceBlockId = model.blockId` — the *writing
channel's* local block. The only reader, `global_output_source` (`app_api.rs:1066`),
resolves the agent zone by looking that block up **locally**. In another channel the
block doesn't exist → fallback skipped → **blank**. Fixed + healed in **#1472**
(normalize the global mirror's `sourceBlockId` to `""`).

### Mechanism 2 — continuity (no cross-channel resume) — *never implemented*
Same-channel reopen resumes via `agent:sessionid` **block meta**
(`hydrate_session_id_from_config` → `--resume <sid>`). Cross-channel the block is
fresh and has no such meta; the **global** carrier is the registry record's
`session_id`, **which production code never writes** — every `.session_id = Some(..)`
in the tree is in tests (`registry/schema.rs:249/267`, `registry/migrate.rs:1430`).
So a fresh-build open has **no sid to resume** → the CLI starts a **new** provider
session. After one turn the new session's id is captured/persisted and **latches
in**, so subsequent opens resume the *short* session. The original provider session
is orphaned.

The two compound: the blank display (M1) is what makes the user interact with a
"fresh" agent, which (M2) spawns and latches the new session that shadows the
original.

## Evidence (on disk)

- Registry: `session_id: null` for **all 9** agents; `created_by_version: "(legacy)"`.
- No production writer of `session_id` (only tests).
- Naki originals intact: provider `91f26930…jsonl` **6.28 MB**; global `output` **32 MB**.
- New short session created on the blank open: provider `e96ed91b…jsonl` (15 KB).
- Global snapshot collapsed to the short view: `highWaterMark: 15`, `sourceBlockId: ""`.
- The global `output` even **grew** (32,099,237 → 32,111,864 B) — the short turns
  appended onto the long log.

## Data safety

**No conversation lost.** Every original lives in (a) the global `agent:<defId>:current`
`output` and (b) the provider session jsonl. The damage is *pointer/anchor*, not bytes.

## Blast radius

**All agents**, on **every** fresh build/channel (per-build isolation → new channel
each `task package`). The #1403 seed only protects an agent until its next live run.

## What went well / wrong

- **Well:** global-first storage — zero real loss; recoverable from disk.
- **Wrong:**
  - A **seed/backfill fix verified only in its pristine state**. The very next normal
    action (run the agent) undoes it. Verification must exercise the **post-use** path.
  - **Two subsystems (display + session) silently depend on each other** and fail the
    same way, so fixing one (display) still looks broken.
  - A **channel-scoped field (`sourceBlockId`) in a global artifact**, and a
    **global continuity field (`session_id`) that's declared but never populated**.
  - **No e2e** for "run agent live in channel A → open in fresh channel B →
    same conversation continues."

## Action items

1. **Display:** land #1472 (normalize-on-mirror + startup heal). *(done, in review)*
2. **Continuity:** actually **populate the registry `session_id`** — on each turn,
   write the agent's authoritative (CLI-emitted) session id into the **global**
   registry record; read it into the spawn config on cross-channel open so the FIRST
   turn `--resume`s the original session.
3. **Backfill** existing records' `session_id` from the newest *original* provider
   session (idempotent migration), and **don't** pick the post-bug short session.
4. **Guard:** a cross-channel open with an existing global transcript/session must
   **resume before** it is allowed to start a new session that shadows the original.
5. **Recover already-affected agents** (Naki et al.): re-point display + session to the
   original (`91f26930`), keep the short `e96ed91b` on disk. *(pending user OK)*
6. **e2e test:** run-live-in-A → open-in-fresh-B → same conversation renders AND
   `--resume` targets the same session id. This is the coverage #1399/#1403 lacked.

## Prevention

Any "seed/backfill/migration" fix must be verified against the **state after normal
use**, not just the freshly-seeded state — and cross-subsystem invariants (display
anchor ⇄ session id) need a single end-to-end test, not two unit checks.

## References

- #1399 (`204fcf07`) globalize transcript; #1403 backfill; #1471 retro + #1472 fix (display half).
- `agentmux-srv/src/backend/agent_session.rs` (`write_session_state` global mirror).
- `agentmux-srv/src/server/app_api.rs:1066` (`global_output_source`).
- `agentmux-srv/src/backend/blockcontroller/subprocess.rs:290–334` (`hydrate_session_id_from_config`, `--resume`).
- `agentmux-srv/src/registry/schema.rs` (`session_id` field, written only in tests).
- Evidence: `~/.agentmux/shared/agents/registry/34ee9b58-…json`; `…/transcripts/filestore.db` zone `agent:f81f9785…:current`; `…/providers/claude/projects/C--Users-asafe--agentmux-agents-naki-0612a/`.
