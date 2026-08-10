# SPEC: eager per-dispatch naming + two-bucket swarm row model

**Date:** 2026-07-19
**Status:** implemented — Phase A PR #2231 (eager naming), Phase B PR #2232 (two-bucket row model); verified in code 2026-08-10.
**Ask (verbatim):** "we want 1 haiku call be agent tool call, and 1 per
workflow tool call, which should be 54 .. actually, lets split Agents and
Workflows, they should be top-level ... AgentA -> Agent Tool -> All
subagents concatenated by time and AgentA -> Workflow -> All subagents
concatenated ... also the Agent Tool/Workflow slug gets a haiku name at the
beginning" — refined down to: at this point we only want a spec written to
file, no implementation.

---

## 1. Problem

Live investigation this session (see the two reports below) found that the
Swarm pane's row labels come from Claude Code's per-**session** `slug`, not a
per-subagent identifier — every subagent spawned within one CLI session
inherits the identical literal slug. A long session with many Agent-tool and
Workflow-tool calls therefore renders as a wall of visually-duplicate rows,
even though the backend correctly tracks each dispatch as a distinct entity
(`dispatch_id`).

Two prior docs (this session, kept local/uncommitted) capture the
investigation and the real numbers behind this spec:

- `docs/specs/REPORT_SWARM_PANE_ROW_MODEL_2026_07_18.md` — the row-kind model
  as of PR #2208 (one row per `AgentDispatch`, Workflow membership always
  wins, `NameGroup` collapses same-slug/same-name solo dispatches).
- `docs/specs/REPORT_SWARM_SUBAGENT_INVENTORY_2026_07_19.md` — ground-truth
  counts for this session's own history: **52 Agent-tool calls, 2
  Workflow-tool calls, 265 subagents total** (213 of them workflow members),
  spanning 14 days, all but one sharing the slug
  `quizzical-tumbling-valiant`.

PR #2226 (merged earlier this session) patched the visible symptom by
widening `NameGroup`'s grouping key to fall back to `slug` when a subagent's
Haiku-generated `display_name` hasn't resolved yet — naming today is lazy,
resolved on-demand only when a client expands a row
(`subagent.GenerateName`, `agentmux-srv/src/server/app_api/session.rs:215-281`).
That fix is sound as far as it goes, but it's a patch on a lazy-naming model.

## 2. What's actually wanted (superseding #2226's approach)

Instead of collapsing same-slug rows into a data-driven `NameGroup`, give
every dispatch its own real, distinct name **eagerly, at dispatch-detection
time** — one Haiku call per Agent-tool call, one per Workflow-tool call (54
for this session, not 265 — never one call per workflow *member*). Organize
the tree into two **fixed** top-level buckets per agent block:

```
▾ 🤖 AgentA
  ▾ Agent Tool (52)
      ├─ ▸ "Verify per-turn overhead doc claims"                  07-04  ○
      ├─ ▸ "Audit swarm dedup regression"                          07-07  ○
      ├─ ...49 more, each its own Haiku title, own row...
      └─ ▸ "Armory consolidation identities and reorder"           07-18  ○
  ▾ Workflow (2)
      ├─ ▸ "Pool-eviction under memory pressure" — 107/107 done    07-13  ✓
      └─ ▸ "Consolidate open issues"              — 106/106 done   07-14  ✓
```

`Agent Tool` and `Workflow` are always exactly these two headers, not
data-driven groups — they don't appear/disappear based on shared names, and
nothing collapses multiple dispatches into one row anymore. Every leaf,
solo or workflow, expands to **one concatenated chronological text feed**
(reusing/generalizing the mechanism `WorkflowDispatchRow` already has today)
instead of the structured per-event tool_use/tool_result tree solo rows
currently show.

This directly **removes `NameGroup`** (`swarm-model.ts:124-146`,
`groupKeyFor`, and the two slug-fallback tests added minutes earlier in PR
#2226) — call this out plainly as a superseding design decision, not an
oversight. With every dispatch eagerly and individually named, and no more
"collapse same-name things into one row" concept, `NameGroup` has no reason
to exist. `frontend/app/view/agent/activity/subagent-adapter.ts` directly
imports `NameGroup`/`isNameGroup`/`groupCacheKey` today for its own
independent activity-dock grouping (`groupSubagentsForDock()`) — needs a
compatibility pass when those types disappear, scope to be confirmed at
implementation time.

## 3. Backend: eager naming infrastructure

Traced against `agentmux-srv/src/backend/subagent_watcher.rs` and
`agentmux-srv/src/server/app_api/session.rs`:

- **The existing naming call** (`generate_subagent_name`, `session.rs:215-281`)
  already does the right thing per-subagent: checks the `SubagentWatcher`
  cache first (no re-spend), admits through the Ambient Model Call gateway
  (`crate::ambient::gateway()`, purpose `AMBIENT_PURPOSE_SUBAGENT_NAME`),
  acquires `pull_call_semaphore()` (`session.rs:110-125`, cap 2 concurrent —
  shared across every ambient caller in the backend), reads the subagent's
  own first JSONL line via `read_task_prompt()`
  (`subagent_watcher.rs:1695-1728`), and calls
  `invoke_ambient_haiku_call` (`session.rs:526-625`, hardcoded
  `--model claude-haiku-4-5-20251001`, the only model this whole ambient-call
  subsystem ever uses). This machinery is reused as-is — the change is
  *when* it fires, not *how* it works.
- **The hook point**: `process_jsonl_change`'s `is_new` computation
  (`subagent_watcher.rs:1018`) is where a subagent's first appearance is
  detected today, immediately followed by the existing solo/workflow branch
  (`workflow_id.is_some()`, lines 1121/1131) that already decides between
  buffering into `pending_activity` vs. broadcasting `subagent:spawned`
  directly. This is the natural place to trigger eager naming too.
- **New tracking needed**: a `naming_triggered: Mutex<HashSet<String>>` keyed
  by `dispatch_id`, checked-and-inserted atomically at that same point,
  *before* spawning any background task — this is what caps naming to
  exactly once per dispatch (the one member for a solo call; the first of N
  for a workflow). No existing state fits this role: `dispatches: Mutex<HashMap<...>>`
  (`subagent_watcher.rs:291`) is populated from both the live debounce loop
  AND `process_journal_change` (workflow `journal.jsonl` processing,
  `subagent_watcher.rs:1429-1495`), which can land in the same debounce batch
  in either order — reusing "does `dispatches` already have this key" as an
  "already named" proxy is racy and would silently skip naming a workflow
  whose journal event happened to process first.
- **Naming source for a workflow**: no single task prompt exists for a whole
  workflow (members can have different prompts) — per the resolved design
  decision, base the one workflow-level Haiku call on the **first member's**
  task prompt, reusing `read_task_prompt()` unchanged. Simple, no new
  prompt-sourcing logic; the name may not perfectly represent every member's
  task, accepted as a reasonable v1 trade-off.
- **New surface area for workflow naming**: `AgentDispatch`
  (`subagent_watcher.rs:125-145`) has no name field at all today — only
  `SubAgent.display_name` exists (`subagent_watcher.rs:70-73`, written by
  `set_display_name`, `subagent_watcher.rs:678-733`). Add
  `AgentDispatch.dispatch_name: Option<String>`, a
  `set_dispatch_display_name`-equivalent, and a `dispatch:named`-equivalent
  broadcast (mirroring `subagent:named`'s shape), wired into
  `broadcast_dispatch_updated` (`subagent_watcher.rs:1551-1573`) and the
  coalesced flush's `latest_info` (`subagent_watcher.rs:1359-1361`) so every
  client watching the session picks up the resolved workflow name, not just
  whichever one triggered it — this is genuinely new backend surface, not a
  relocation of the existing per-member RPC (today's `WorkflowDispatchRow`
  derives its label purely client-side from a member's `slug`,
  `swarm-model.ts:207-211`, and never calls `GenerateName` for a workflow at
  all).
- **Non-negotiable guard — never during backfill.** `scan_subagents_dir`
  (`subagent_watcher.rs:903-962`, capped at `BACKFILL_MAX_FILES=200`) replays
  historical `agent-*.jsonl` files as `is_new` on every srv restart / pane
  reopen — this session alone would trigger ~54 unwanted Haiku calls on every
  restart if eager naming isn't explicitly scoped to genuinely-live,
  filesystem-watched spawns only. `process_jsonl_change` has no
  caller-context signal today distinguishing a backfill replay from the live
  debounce loop (`subagent_watcher.rs:492-535`) — one needs to be added
  (e.g. a `live: bool` parameter threaded from each call site). This directly
  guards against repeating the incident class documented in
  `docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md`.
- **Async pattern to follow**: `agentmux-srv/src/backend/reactive/activity_watcher.rs:99-141`
  is the closest existing precedent for "background/non-request-driven
  ambient call, capped and de-duplicated" — an `in_flight: Arc<Mutex<HashSet<String>>>`
  guard, its own `Semaphore`, `tokio::spawn`, cleanup on every exit path.
  Mirror this shape for the eager-naming task rather than inventing a new
  pattern.
- **`SubagentWatcher` currently has no `Store` handle.** `generate_subagent_name`
  needs `wstore: &Store` to resolve the parent Block's `cmd`/`cmd:env` (a
  subagent borrows its parent's CLI path + auth env — it has no Block of its
  own). Plumbing `wstore: Arc<Store>` into `SubagentWatcher::new`/`spawn`
  touches 3 construction call sites: `agentmux-srv/src/main.rs:923`,
  `agentmux-srv/src/server/agent_handlers/mod.rs:334` and `:645`,
  `agentmux-srv/src/server/tests.rs:54`. Mechanical, not architecturally
  hard — `wstore` is already constructed earlier in `main.rs`, right before
  `subagent_watcher` (line 923).
- **Groundwork this also requires for the frontend's unified feed (§4):**
  solo-dispatch activity/spawn/completion currently bypass
  `pending_activity` entirely via three `else` branches gated on
  `workflow_id.is_some()` (`subagent_watcher.rs:~1131-1161` spawn,
  `~1176-1199` activity, `~1212-1239` completion) — confirmed live,
  `dispatch:activity` is never broadcast for a `solo:<agent_id>` dispatch_id
  today (only the old `subagent:activity` event fires). Route solo dispatches
  through the same `pending_activity`/`flush_pending_dispatch_activity`
  machinery (`subagent_watcher.rs:1267-1329`) instead, so
  `createDispatchDetail("solo:<agent_id>")` actually receives events.

## 4. Frontend: two-bucket tree + unified expand

Traced against `frontend/app/view/swarm/swarm-model.ts` and `swarm-view.tsx`:

- **`buildDispatchChildren()`/`AgentTreeNode.subagents`**
  (`swarm-model.ts:162-270`) today produces one flat, recency-sorted
  `SwarmChild[]` mixing three row kinds. Replace with two fixed arrays
  (`agentToolRows: ActiveSubagent[]`, `workflowRows: WorkflowDispatch[]`),
  dropping `NameGroup`/`groupKeyFor` entirely. Keep `WorkflowDispatch`'s half
  of `stabilizeGroupIdentity`/`groupCacheKey`/`pruneGroupIdentityCache` — the
  object-identity-stabilization problem those solve (SolidJS `<For>`
  remounting wrapper objects on every unrelated tree recompute,
  `swarm-model.ts:371-414`) doesn't go away just because `NameGroup` does.
- **`AgentRow`'s single flat `<For>`** (`swarm-view.tsx:300-307`) becomes two
  new bucket-header components (`AgentToolBucket`, `WorkflowBucket`), each
  with its own live count and its own color. No existing "kind"-based (as
  opposed to status-based) color axis exists anywhere in
  `swarm-view.scss` today — the only current color differentiation is
  active/retired/completed/abandoned status (e.g. the status-badge pattern
  at `swarm-view.scss:218-235`, `color-mix(in srgb, var(--warning-color, ...) 15%, transparent)`).
  Follow that same CSS-custom-property + `color-mix()` convention for the two
  new bucket colors rather than hardcoding hex values — this is a new
  precedent, not a reuse of an existing one. Buckets hide when empty,
  matching the existing `hasChildren`-gated collapse-chevron precedent
  (`swarm-view.tsx:236,268-279`) — no "always visible even when empty"
  behavior; there's no analog for that anywhere in this codebase and no
  stated reason to invent one.
- **Unified concatenated-feed expand.** Generalize
  `DispatchActivityFeed`/`createDispatchDetail`
  (`swarm-view.tsx:322-444`, `swarm-model.ts:534-562`) — today wired only for
  Workflow-kind dispatches — to cover solo rows too (depends on §3's backend
  routing fix). Two gaps to close while doing this:
  1. `createDispatchDetail` is explicitly **live-only**, no historical
     backfill on expand (`swarm-model.ts:526-533` doc comment: "a large
     dispatch can have thousands of prior events... eagerly fetching +
     merging on every expand would reintroduce the exact volume problem this
     redesign exists to fix"). The mechanism it replaces for solo rows,
     `createSubagentDetail` (`swarm-model.ts:459-522`), DOES backfill via
     `subagent.GetHistory`/`subagent.GetInfo`. Don't regress that — add
     backfill to the generalized feed (bounded, matching the existing
     `MAX_DISPATCH_FEED_ENTRIES = 500` cap, `swarm-model.ts:540`).
  2. Once the unified feed covers both cases, retire
     `SubagentDetailPane`/`SubagentDetailEvent`/`createSubagentDetail`
     (`swarm-view.tsx:553-642`, `swarm-model.ts:459-522`) — dead code after
     the migration.
- **Naming display**: every row (solo and workflow) shows its
  eagerly-resolved title immediately — no more `handleToggle`-triggered
  `GenerateName` call on first expand (`swarm-view.tsx:509-519`), since
  naming already happened at dispatch time per §3. `subagentDisplayLabel`'s
  `slug`-fallback logic (`swarm-model.ts:281-287`, added for #2226) becomes
  a true last-resort fallback only (e.g. a dispatch whose naming call is
  still in flight or failed), not the common case it is today.
- **`subagent-adapter.ts` compatibility**: it imports
  `NameGroup`/`isNameGroup`/`groupCacheKey` directly
  (`frontend/app/view/agent/activity/subagent-adapter.ts:34-39`) for its own
  independent `groupSubagentsForDock()` — confirm at implementation time
  whether it needs updating or is already self-contained enough not to.
- **Test rewrite**: `swarm-model.test.ts`'s `buildDispatchChildren` describe
  block (`swarm-model.test.ts:61-362`) needs rewriting against the two-bucket
  shape, including dropping the `NameGroup` describe block
  (`swarm-model.test.ts:201-320`ish) — which includes the two slug-fallback
  tests (`collapses 2+ solo dispatches sharing only a slug...`,
  `prefers display_name over slug as the grouping key...`) added minutes
  earlier in PR #2226. Their removal is an expected, direct consequence of
  §2, not a regression.

## 5. Phasing

Implement as two phases, each verified live in the already-running `task dev`
instance (`dev:main`, `~/.agentmux/dev/main/`) before the next starts:

**Phase A (backend)** — §3 in full: `wstore` plumbing, `naming_triggered`
set, the backfill-vs-live guard, the eager-naming background task, workflow
`dispatch_name` + broadcast, solo activity routed through
`pending_activity`. Verify: dispatch a real solo call and a real workflow
batch from a live agent pane, confirm via `muxlog srv grep` exactly one
naming-resolved log line fires per dispatch immediately (not on click),
confirm **zero** naming calls fire on an srv restart against this session's
existing 265-subagent backfill (the critical regression to rule out), confirm
`dispatch:activity` now flows for solo dispatch_ids too.

**Phase B (frontend)** — §4 in full: two-bucket tree, bucket-header
components + colors, generalized concatenated feed with backfill,
`NameGroup` removal, `subagent-adapter.ts` compatibility pass, test rewrite.
Verify: in `task dev`, confirm two colored buckets under AgentA showing
live counts (52/2-style for a session like this one), each leaf showing its
eagerly-resolved title with no click needed, expand showing the concatenated
feed with historical entries present (not just live-from-expand-onward).

## 6. Explicitly deferred / open questions

- **No global ambient-call budget/rate-limit exists in the codebase today.**
  `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md:339-342`
  already flags this as an unimplemented gap for the whole ambient-call
  framework, not something specific to this feature. Eager firing turns a
  click-paced trigger into a filesystem-event-paced one, which makes the gap
  more relevant than it was for the lazy design — a session that spawns many
  dispatches now pays for every single one unconditionally, throttled only
  to 2 concurrent Haiku calls via the existing `pull_call_semaphore`, with no
  cap on total spend. **This question was explicitly left open by the user
  rather than resolved** — Phase A ships reusing only the existing semaphore,
  no new per-session/global budget. Revisit if real-world cost from eager
  firing turns out to be a problem.
- **Always-visible-even-empty buckets** — not building this; buckets hide
  when empty per §4, no existing precedent for the alternative.

## 7. Files (representative, not exhaustive)

**Backend:**
- `agentmux-srv/src/backend/subagent_watcher.rs`
- `agentmux-srv/src/server/app_api/session.rs`
- `agentmux-srv/src/main.rs`
- `agentmux-srv/src/server/agent_handlers/mod.rs`
- `agentmux-srv/src/server/tests.rs`

**Frontend:**
- `frontend/app/view/swarm/swarm-model.ts`
- `frontend/app/view/swarm/swarm-view.tsx`
- `frontend/app/view/swarm/swarm-view.scss`
- `frontend/app/view/swarm/swarm-model.test.ts`
- `frontend/app/view/agent/activity/subagent-adapter.ts` (compatibility check)

## 8. Verification (for the eventual implementation, not this spec pass)

Backend: `cargo check -p agentmux-srv`, `cargo test -p agentmux-srv` for the
touched modules, then the live Phase A steps in §5 (Rust changes need
`task build:backend` + restart — `task dev`'s frontend hot-reload doesn't
cover this). Frontend: `npx vitest run frontend/app/view/swarm/`,
`npx tsc --noEmit -p tsconfig.json`, then the live Phase B steps in §5
(hot-reloads directly in the running `task dev` window). No CI coverage
exists for the live dispatch/naming behavior itself (WS events, Haiku call
timing) — manual verification via `muxlog srv` is the only signal for
whether eager naming and the two-bucket tree actually work end-to-end.
