# Spec: Agent-facing Global Memory API (MCP tools to add/list/read a Global Memory entry)

**Status:** implemented — Phases 0-2 (audit trail, REST routes, MCP tools). — #3237
Phase 3 (the availability/gating open question) is UNRESOLVED — every agent
gets these tools unconditionally today (option 1 from §2.4), matching
native memory's own precedent, but that was not an explicit decision, just
what shipping Phases 0-2 with no additional gating defaults to. Also not
built in this pass, deliberately: the read side of the audit trail
(`bundle_version_list`/`bundle_version_get` exist and are tested, but no
`GlobalMemoryHistory`/`Diff`/`Revert` MCP tool or Armory UI surfaces them
yet — see `bundle_versions.rs`'s own module doc comment).
**Date:** 2026-09-15
**Verified against:** code as of `507071f8` (backend chain confirmed live by
a dedicated research pass — see §1 for the exact files/methods, not
inferred from spec prose).
**Related:** `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` (the isolation
this spec must not weaken), `SPEC_GLOBAL_MEMORY_UNIFY_SYSTEM_AND_ORDINARY_
2026_09_15.md` (the frontend-side unification this spec's naming follows),
`SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_2026_09_15.md`.

## 1. Current state — confirmed, not assumed

**No agent-facing API for Global Memory exists today.** The only
memory-shaped MCP tools an agent has (`MemoryWrite`/`MemoryRead`/
`MemoryList`/`MemoryHistory`/`MemoryDiff`/`MemoryRevert`) are scoped to
**native memory** (`db_agent_native_memory`) — explicitly "your own,"
per-agent, never global.

**How those existing tools actually work** (agentmux-mcp/src/main.rs →
REST → `agentmux-srv/src/server/app_api/mod.rs`), because the new tools
this spec proposes should follow the identical pattern:

- `MemoryWrite`'s handler (`agentmux-mcp/src/main.rs:2607-2643`) resolves
  `agent_slug()` (`main.rs:280-289`) from `AGENTMUX_AGENT_ID`, a **process-
  level env var baked into that agent's own MCP server process at spawn
  time** — not a caller-supplied parameter. It POSTs to
  `/api/v1/agent/memory/write` (`server/mod.rs:566`), which dispatches to
  `memory_write_impl` (`app_api/mod.rs:969`).
- The trust model, stated explicitly in both files: an agent's own PTY has
  no auth key to reach this REST route directly, so the slug can't be
  forged from inside the agent's own shell — only the trusted `agentmux-
  mcp` binary can reach it, and it always stamps its own agent's real
  slug. `memory_write_impl` does **not** additionally cross-check identity
  against an authenticated RPC connection (no `check_s1` — that exists and
  is used elsewhere, e.g. `mcp.upsert` in `app_api/mcp.rs`, but not here).

**The Global Memory write path today** (`agent_handlers/bundle.rs:58-100`,
`COMMAND_UPSERT_MEMORY` → `Store::bundle_upsert`,
`backend/storage/bundles.rs:216-268`):

- Reachable only via the authenticated WebSocket RPC channel the frontend
  uses (`TabRpcClient`) — a completely different transport from the REST
  routes the native-memory MCP tools call. **Not reachable from
  `agentmux-mcp` today at all** — this is the actual gap, not a missing
  MCP tool wrapper around an already-agent-reachable backend call.
- `bundle_upsert`'s handler ignores `ctx` (identity) entirely — confirmed
  still true, matching `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` §2
  exactly, not stale. It persists whatever `is_global` the caller sends,
  with exactly one guard: it refuses outright if the target row has
  `is_system=1` (lines 218-223). **This guard is what already makes system-
  tier entries structurally unreachable through this path — any new
  MCP-facing route built on the same `Store::bundle_upsert` inherits that
  protection for free, without reimplementing it.**
- `upsertsystemmemory`/`deletesystemmemory` (`bundle.rs:171-241`) are a
  genuinely separate, hardcoded-`is_system=1` path, and are — per that
  file's own comment — "never wired to any MCP tool." **This spec does not
  propose changing that.** Nothing here should ever let an agent create or
  touch a system-tier entry.

**Real, confirmed gap: no audit trail.** `db_bundles`
(`migrations.rs:1601-1618`) has only `created_at`/`updated_at` — no
versions table, unlike `db_agent_native_memory_versions`
(`migrations.rs:870`, the table that backs `MemoryHistory`/`MemoryDiff`/
`MemoryRevert`). If an agent could write Global Memory today, there would
be **no way for a human to see what changed, when, by which agent, or to
revert it** — not even the coarse-grained history native memory already
gets for free from an equivalent, already-shipped, already-working system.

**Precedent for a shared/workspace-wide MCP mutation:** `WorkEnqueue`/
`WorkClaim` (`agentmux-mcp/src/main.rs:2228` → `work_queue.rs:85-131`) —
any agent's enqueue becomes claimable state visible to every other agent,
gated by nothing beyond identity-of-sender. The closest existing example,
but categorically lower-stakes than Global Memory: a work-queue item is
transient and meant to be claimed/consumed once; a Global Memory entry is
**persistent and re-read by every future agent at every future launch**
until someone notices and removes it.

## 2. Design

### 2.1 Phase 0 — audit trail (recommended prerequisite, not hard-blocking)

Add `db_bundles_versions`, mirroring `db_agent_native_memory_versions`'s
already-proven shape exactly (same migration pattern, same
`bundle_history`/`bundle_diff`/`bundle_revert` Store-method shape as the
native-memory equivalents this repeats). Every write through the new
agent-facing path (§2.2) — and, for consistency, every write through the
existing human-facing Armory UI path too — appends a version row instead
of only updating in place.

**Why this belongs in Phase 0, not "nice to have later":** Global Memory's
blast radius is every agent, indefinitely, not just the writing agent
itself. Native memory already treats "every write is retained as a
version — nothing is ever silently lost" as a stated design guarantee
(`MemoryWrite`'s own tool description). Opening a second, higher-stakes
write path without the same guarantee would be a regression relative to
the bar this codebase has already set for itself.

### 2.2 Phase 1 — backend: new REST route(s), reusing `Store::bundle_upsert`

New routes under `/api/v1/agent/globalmemory/{write,list,read,remove}`,
mirroring `/api/v1/agent/memory/write`'s existing pattern exactly (same
`server/mod.rs` registration style, same "REST reachable from agentmux-mcp,
not the WS RPC channel agents can't reach" trust boundary as §1
described).

- **`write`**: deserializes `{ id?: string, name: string, content: string,
  provenance?: {...} }` (mirroring `MemoryWrite`'s own optional
  `provenance` shape). **Hardcodes `is_global: true` and `is_system:
  false`/omitted server-side — never reads `is_system` from the request
  body at all**, the same "S4a: strip caller-supplied escalation fields"
  pattern `bundle.rs:258` already uses elsewhere in this same file, so
  there is no code path here that could ever construct an `is_system=1`
  row even if a future refactor added a field carelessly. Calls
  `Store::bundle_upsert` (existing) then appends a `db_bundles_versions`
  row (§2.1) recording the calling agent's slug (from `AGENTMUX_AGENT_ID`,
  same resolution as `MemoryWrite`) as provenance.
- **`list`**: returns ordinary (non-system) Global Memory entries only —
  `{id, name, updated_at}`, no content (mirrors `MemoryList`'s shape).
  Never includes system-tier rows, structurally: the query itself filters
  `is_system=0`, not just the response shape.
- **`read`**: `{id}` → full `{name, content}`.
- **`remove`**: `{id}` → flips `is_global=false` (matches the existing
  Armory UI's own "Remove" semantics — demotes rather than hard-deletes;
  the underlying bundle row survives, same as the human-facing path
  today).

None of these routes accept or infer `is_system` from the caller under any
circumstance. The system tier remains reachable only through
`upsertsystemmemory`/`deletesystemmemory`, unchanged, still never wired to
any MCP tool.

### 2.3 Phase 2 — agentmux-mcp: new tools

`GlobalMemoryWrite`/`GlobalMemoryList`/`GlobalMemoryRead`/
`GlobalMemoryRemove`, registered in `agentmux-mcp/src/main.rs` alongside
the existing `Memory*` tools, same implementation shape (resolve
`agent_slug()`, POST to the new REST route, return the response). Tool
descriptions should explicitly state the scope distinction plainly enough
that a model can't confuse this with the existing native-memory tools:
"Adds an entry to **Global Memory** — inherited by **every** agent in this
workspace at launch, not just you," vs. `MemoryWrite`'s "your own native
memory."

### 2.4 Phase 3 — OPEN QUESTION, not decided here: availability/gating

Native memory's tools are unconditionally available to every agent today,
with no extra confirmation step — the trust argument
`SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` §2 already made for this
whole app ("agents already merge PRs, push code, and run arbitrary shell
commands under existing operator-granted autonomy") argues for treating
Global Memory writes the same way, once Phase 0's audit trail exists to
make a bad write noticeable and reversible. But Global Memory differs from
every existing agent-facing mutation in one respect: it isn't reviewed
(unlike a PR) and, unlike `WorkEnqueue`'s transient claimable state, it
persists and silently reshapes every other agent's instructions until a
human happens to look at the Armory pane. Options, **not chosen here**:

1. **Same trust model as native memory** — available to every agent,
   unconditionally, once Phase 0 ships. Simplest, most consistent with how
   this app already treats agent autonomy elsewhere.
2. **Available, but the write surfaces as a `jekt`-style notable event** —
   e.g. a `TIER=sensitive`-equivalent signal or a pushed notification to
   the human operator on every Global Memory write, so a bad one is caught
   promptly rather than only on eventual manual review. Doesn't block the
   write, just guarantees visibility close to real-time.
3. **Gated per-agent** — an explicit opt-in (a setting, or a scope on the
   agent's own identity) before an agent's `agentmux-mcp` process even
   exposes these tools, so Global Memory write access is something an
   operator grants deliberately per agent rather than a blanket default.

Needs a decision before implementation; this spec deliberately stops short
of picking one, the same way `SPEC_GLOBAL_MEMORY_UNIFY_SYSTEM_AND_ORDINARY_
2026_09_15.md` left its own visual-distinction question open rather than
guessed.

## 3. Non-goals

- **Not proposing any change to system-tier isolation.** §2.2 is explicit
  about this; re-stated here because it's the single most important
  invariant this spec must not weaken.
- **Not proposing this for Personal Memory.** Personal Memory
  (`db_agent_native_memory`) already has a full agent-facing API
  (`Memory*` tools) and its own audit trail — this spec is scoped to the
  Global tier's gap specifically.
- **Not designing the REST/RPC response schemas in full JSON-Schema
  detail** — left for implementation time, following the existing
  `Memory*` tool schemas as the template (§2.3).

## 4. Test plan (for implementation time)

- Rust: a `bundle_upsert`-via-the-new-route test confirming an `is_system`
  field in the request body (if one were ever added) is silently ignored,
  never persisted — the regression test for the exact invariant §2.2
  depends on.
- Rust: `db_bundles_versions` round-trip (write → history → diff → revert)
  mirroring the existing native-memory version tests.
- Integration: an MCP tool call end-to-end, confirming a `GlobalMemoryWrite`
  from one agent is visible via `bundle.upsert`'s existing read path (i.e.
  shows up in the Armory Global Memory list the same as a human-authored
  entry would) and is captured in `db_bundles_versions` with the correct
  agent provenance.
