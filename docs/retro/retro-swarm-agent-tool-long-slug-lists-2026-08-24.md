# Retro: Swarm pane rows should be labeled with the tool call's own text, not a Claude-CLI-internal slug

**Date:** 2026-08-24
**Reported by:** repo owner, across three follow-ups on the same thread:
1. *"we keep continue working on the same problem regarding the swarm and the
   long lists. We simply want 1 entry per agent tool call, but we are still
   getting the long lists of slugs."*
2. *"no, I am talking about the swarm pane, not the agent pane"* — confirms
   the dedicated Swarm pane (`defwidget@swarm`), not the Activity Dock.
3. *"we dont need any backfill, we just want it to work moving forward."*
4. *"right .. so we want 1 row per call. the name of the row should be the
   text used at the point the subagent tool is used. but currently we dont
   see that text, instead we see many rows of slugs."*

**This last message is the resolving clarification.** Row *count* is not
the complaint — 1 row per solo Agent-tool call is already what the current
architecture produces (confirmed in §2). The complaint is entirely about row
*labeling*: rows show a Claude-Code-CLI-internal slug instead of the actual
task text the tool call was invoked with, and because many calls share the
same batch slug, a perfectly correct list of N distinct rows reads as "N
copies of the same slug."

**Status:** root-caused with a concrete, code-verified implementation path.
**No code changed in this pass** — investigation only, but §3 below is
detailed enough to implement directly.

---

## 1. Why rows show a slug instead of the tool call's own text — confirmed architecture

**AgentMux never intercepts the Agent/Task tool call.** The tool that spawns
a subagent (`Agent`, formerly `Task`) is Claude Code CLI's own built-in tool
— AgentMux's own MCP server (`agentmux-mcp/src/main.rs`) defines unrelated
tools (`Shell`, `Loop`, cron, UI automation) and has no `Agent`/`Task`/
`Workflow` tool definition at all. This is explicit, documented policy, not
an oversight: `docs/analysis/ANALYSIS_SUBAGENT_SPAWN_TAXONOMY_2026_07_14.md:259-264`
states AgentMux is *"100%-observational by design, never triggering spawns
itself"* — it only ever discovers a spawn by watching the JSONL transcript
files Claude Code CLI writes to disk, after the fact.

Consequently, everything the Swarm pane currently labels a row with is
reconstructed by post-hoc filesystem inference:
- `slug` — a CLI-internal kebab-case codename, read verbatim off the
  subagent's own JSONL, is per-batch not per-call, and has **no relationship
  to what the call was actually asked to do**.
- `display_name` — an async Haiku-generated ~5-word summary, requires an
  extra LLM round-trip after spawn, and (per `SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md`'s
  explicit, deliberate guard) is **never triggered for backfilled/historical
  rows** at all, only live ones — so even when it works, it's a
  reconstruction, not the original text.

Neither of these is "the text used at the point the subagent tool is used."
That text exists — see below — but is currently read, used once for
something else, and thrown away.

## 2. Row count is already correct — this is a labeling problem, not a grouping problem

Confirmed directly in `frontend/app/view/swarm/swarm-model.ts:213-288`
(`buildDispatchBuckets`): every solo `Agent`/`Task`-tool call gets a
permanently unique `dispatch_id` of `solo:<agent_id>`
(`swarm-model.ts:26-33`), and `agentToolRows` is literally
`subagents.filter(s => s.dispatch_id.startsWith("solo:"))` (line 236) — one
row per call, already, with no accidental merging or duplication. (A
dedicated `Workflow`-tool call is a separate, already-correctly-grouped
case — one row per run regardless of member count — not relevant to this
complaint.) **No fix is needed for row count.** The entire fix surface is
"what text does a row display," covered next.

## 3. The tool call's own text IS captured today — read once, then discarded

Two independent places hold this text. Only one is currently reachable from
the Swarm pane's data model, and even that one is discarded rather than
stored.

### 3.1 The full `prompt` — already read, already reaches the right place, currently thrown away

`agentmux-srv/src/backend/subagent_watcher/parse.rs:186-219`
(`read_task_prompt()`) reads the subagent's own `agent-<id>.jsonl` file's
**first line** — a `"type":"user"` record — and extracts `message.content`.
Confirmed directly against a real transcript on this machine:

```
agent-a049d00d35dcf66d4.jsonl, line 1:
{"parentUuid":null,"isSidechain":true,"agentId":"a049d00d35dcf66d4","type":"user",
 "message":{"role":"user","content":"In the agentmux repo at ... I need to find..."}}
```

This `message.content` is **byte-for-byte the `prompt` parameter** the
Agent/Task tool call was invoked with — Claude Code turns the tool's
`prompt` argument directly into the spawned subagent's own first turn.

**This is called today, but only to build a Haiku prompt and then discard
the raw text:** `read_task_prompt()` has exactly two call sites, both in
`agentmux-srv/src/server/app_api/session.rs` — `generate_subagent_name`
(line 245) and `generate_dispatch_name` (line 335) — each feeds the raw
prompt into an LLM call ("give a concise ~5-word name for this"), keeps only
the condensed result (`subagent_watcher.set_display_name`, line 279), and
never stores the original text anywhere. `SubAgent`
(`agentmux-srv/src/backend/subagent_watcher/types.rs:22-59`) has no
`task_prompt`/`prompt` field at all today.

Also notable: the exact same first line is *already parsed* for `slug`/
`model`/`parentUuid` metadata inside `read_jsonl_from_offset`
(`parse.rs:81-173`, lines 117-149) — the same `serde_json::Value` that
`read_task_prompt()` re-opens the file separately to re-parse a second
time. Capturing the prompt text at that point costs no extra file I/O.

### 3.2 The short `description` — Claude's own concise label — exists only in the PARENT's session file, which the backend never reads

The literal `description` field (the short, 3-5-word summary Claude Code's
own Agent-tool schema takes alongside `prompt`) is **not present in the
subagent's own JSONL at all** (confirmed: zero matches for `"description"`
in a real subagent transcript file). It only exists in the **parent
agent's own top-level session JSONL**, in the `tool_use` block and its
paired `tool_result`/`toolUseResult` line:

```
tool_use:    "name":"Agent","input":{"description":"Find LAN discovery status API/diagnostics", "prompt":"..."}
tool_result: "toolUseResult":{"isAsync":true,"agentId":"a049d00d...",
              "description":"Find LAN discovery status API/diagnostics",
              "resolvedModel":"claude-sonnet-5","prompt":"..."}
```

This is a **direct, exact, non-heuristic** correlation: `agentId` sits right
next to `description` and `prompt` in the same JSON object. But
`subagent_watcher.rs` never opens the parent's own session file — only the
subagent's own `agent-<id>.jsonl` / workflow `journal.jsonl` files (confirmed
in `scan.rs:433-495`, `jsonl.rs:35` — these are the only files it reads).

The frontend *does* already parse this, but only for its own, unrelated
purpose (rendering the calling pane's own transcript, live, in-memory,
never persisted):
- `frontend/app/view/agent/stream-parser.ts:129-130` — `extractToolDetail`
  reads `params.description || params.prompt`.
- `frontend/app/view/agent/components/tool-renderers/DispatchCard.tsx` +
  `frontend/app/view/agent/activity/dispatch-correlation.ts` correlate a
  transcript `tool_use` node to a live `AgentDispatch` — but only by
  **ordinal position** (ordinal-matches this pane's Agent/Task/Workflow
  `tool_use` nodes against this pane's dispatches by transcript-order vs.
  spawn-order), explicitly documented as having no exact id link and
  bailing out on parallel same-turn spawns or pruned history
  (`dispatch-correlation.ts:5-9, 12-77`). This pipeline is frontend-only,
  ephemeral, and never reaches the Swarm pane's own data model at all.

## 4. Two implementation paths

### 4.1 Minimal fix — thread the full `prompt` through (cheap, no new watch surface)

Uses the exact mechanism already reading this text today (§3.1), just
stores it instead of discarding it:

1. `subagent_watcher/parse.rs`: add `task_prompt: Option<String>` to
   `JsonlMeta`; extract it in the same `offset == 0` block that already
   parses `slug`/`model` (lines 117-149), reusing the string/content-block
   parsing logic `read_task_prompt()` already has (lines 198-216).
2. `subagent_watcher/types.rs`: add `task_prompt: Option<String>` to
   `SubAgent` (and, sourced from the first member, to `AgentDispatch` for
   the Workflow-row case).
3. `subagent_watcher/jsonl.rs`: copy `meta.task_prompt` into the live
   `SubAgent` state alongside `slug`/`model` in `process_jsonl_change`'s
   existing meta-apply block (lines 141-151).
4. **No RPC/wire change needed** — `subagent.ListActive`/`GetInfo`
   (`agentmux-srv/src/server/service/misc.rs:129-152`) already serializes
   the whole `SubAgent`/`AgentDispatch` struct to JSON; a new field just
   starts appearing.
5. `frontend/app/view/swarm/swarm-model.ts`: add `task_prompt: string | null`
   to `ActiveSubagent`/`AgentDispatch`; update `subagentDisplayLabel()`
   (lines 424-430, currently `display_name > slug > shortId`) and the
   `WorkflowDispatch.name` fallback (lines 224, 271) to prefer `task_prompt`
   — truncated to its first line/N characters, since a `prompt` can be
   multi-paragraph, unlike the short `description` in §4.2.
6. `shallowEqualSubagent` (`swarm-model.ts:432-445`) needs `task_prompt`
   added to its equality check, or an unchanged-but-refetched subagent will
   spuriously read as "changed" — the exact bug class that check exists to
   prevent.

**Gets you:** the real task text, synchronously, at spawn time, per-call,
today's-architecture-compatible. **Tradeoff:** it's the full `prompt`, which
can be long/multi-line/verbose — will need truncation and won't read as
cleanly as a purpose-written short label.

### 4.2 Fuller fix — also capture the short `description` (bigger scope, better label, exact correlation)

Requires new watch surface: `subagent_watcher` would need to additionally
watch/parse the **parent's own top-level session JSONL** for `tool_use`/
`toolUseResult` records naming `Agent`/`Task`/`Workflow`, extracting the
`agentId` + `description` + `prompt` triple directly from the
`toolUseResult` line (§3.2). This gives:
- The literal short `description` Claude itself was given — the actual
  "text used at the point the tool is used," in the most literal reading of
  the request, and a much better row label than a truncated prompt dump.
- An **exact** id-based correlation, which as a side effect would let
  `dispatch-correlation.ts`'s fragile ordinal-matching (frontend, transcript
  dispatch cards) be replaced with something non-heuristic too — a second,
  independent win, not required for this fix but worth noting since it's
  the same new watch surface.

This is real new scope — a file `subagent_watcher.rs` doesn't touch today —
and deserves its own short design pass rather than being folded silently
into 4.1. Recommend implementing **4.1 first** (cheap, immediate, uses only
data already being read) as an improvement available right now, then
deciding whether 4.2's better label + exact correlation is worth the added
watch surface as a fast follow-up.

## 5. Secondary: port whichever fix lands here to the Activity Dock too

`frontend/app/view/agent/activity/subagent-adapter.ts:116`
(`subagentToActivity`) has the exact same `display_name || slug || agent_id`
fallback chain, confirmed still live today, with **no** disambiguation at
all (not even the Swarm pane's old `${slug} · ${shortId}` suffix). Whatever
field/priority change lands in `subagentDisplayLabel()` per §4 should be
ported here too, and to `ActivityRow.tsx:198`'s render (which currently
uses the adapter's raw `title` directly, unlike `ActivityRow.tsx:282`'s
member-roster view, which already calls the correct Swarm-pane function).
Not today's reported bug (confirmed: this report is about the Swarm pane
specifically), but the identical fix will eventually be wanted here too.

## 6. What this is NOT

- **Not a row-count/grouping bug.** Confirmed in §2 — 1 row per solo call is
  already the current, correct behavior. No batch/turn-correlation feature
  is needed to satisfy "1 row per call"; that part already works.
- **Not a backfill/retention issue.** Explicitly ruled out by the repo
  owner; not investigated further here. (An earlier draft of this retro
  focused on retention — `docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md`
  / PR #2677's "Clear completed" button and unchanged `BACKFILL_MAX_FILES`
  cap — that work is real but answers a different question than the one
  actually being asked here.)
- **Not the Activity Dock's own naming gap** (§5) — real, undocumented
  elsewhere, but a follow-up, not today's report.

## 7. Evidence / sources

- `agentmux-mcp/src/main.rs` — AgentMux's own MCP tool list; no `Agent`/
  `Task`/`Workflow` tool defined here, confirming the tool is Claude Code
  CLI's own.
- `docs/analysis/ANALYSIS_SUBAGENT_SPAWN_TAXONOMY_2026_07_14.md:104-107,259-264`
  — "100%-observational by design, never triggering spawns itself"; the
  on-disk JSONL format is explicitly undocumented-as-stable per Anthropic.
- `agentmux-srv/src/backend/subagent_watcher/parse.rs:81-173,186-219` —
  `read_jsonl_from_offset`'s existing meta-extraction (`slug`/`model`) and
  `read_task_prompt()`'s separate, discarded re-parse of the same line.
- `agentmux-srv/src/server/app_api/session.rs:215-281,308-339` —
  `generate_subagent_name`/`generate_dispatch_name`, the only two call
  sites of `read_task_prompt()`, both LLM-condense-then-discard.
- `agentmux-srv/src/backend/subagent_watcher/types.rs:22-59` — `SubAgent`,
  confirmed no `task_prompt`/`prompt` field exists today.
- `agentmux-srv/src/server/service/misc.rs:129-152` — `subagent.ListActive`/
  `GetInfo`, confirmed already serializing the whole struct (no RPC change
  needed for §4.1).
- `frontend/app/view/agent/stream-parser.ts:129-130`,
  `frontend/app/view/agent/activity/dispatch-correlation.ts:5-9,12-77`,
  `frontend/app/view/agent/components/tool-renderers/DispatchCard.tsx` —
  the frontend-only, ordinal-heuristic, never-persisted pipeline that
  already has `description`/`prompt` for its own unrelated purpose.
- `frontend/app/view/swarm/swarm-model.ts:213-288` (`buildDispatchBuckets`),
  `:26-33` (`dispatch_id` doc comment), `:424-430` (`subagentDisplayLabel`),
  `:432-445` (`shallowEqualSubagent`) — the Swarm pane's current row-count
  correctness (§2) and label priority chain (§4.1's edit target).
- `frontend/app/view/agent/activity/subagent-adapter.ts:116`,
  `frontend/app/view/agent/components/ActivityRow.tsx:198,282` — the
  Activity Dock's own, separate, still-unfixed instance of the same
  labeling gap (§5).
- `docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md` —
  the eager-Haiku-naming design this retro's §4 fix would sit alongside
  (not replace) in the label priority chain.
- `docs/specs/SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md`,
  PR #2677 — the retention/accumulation work from an earlier round of this
  same complaint thread, ruled out as the current ask (§6).
