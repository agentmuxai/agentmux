# SPEC: Session-scoped pane scrollback + a full "Agent History" view

**Date:** 2026-08-09
**Status:** Proposed
**Severity:** Medium — UX/correctness follow-up, no data loss involved
**Extends:** `SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md` (Part A shipped the
honest *"New session started"* divider this spec builds on)
**Related:** `SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` (account-wide,
cross-agent browse — orthogonal; this doc is the *per-agent, in-pane* surface),
`docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`
(the global transcript zone this doc reads from)

---

## 0. Ask

> "We want to retain all the history, but in the agent pane we want the
> conversation history to reflect the actual knowledge of the agent as it is
> working. So after a 'New Session Started' we would not have any conversation
> EXCEPT a link to another view — an 'Agent History' view of the agent pane
> that makes it easy to peruse the entire history, including separators for
> days."

Two halves:

1. **The working scrollback is scoped to the agent's live session.** Everything
   above the most recent *fresh* session boundary disappears from the working
   pane — the pane shows only what the model actually has in context, so a
   human glancing at it never mistakes lost history for live knowledge.
2. **Nothing is deleted.** The full multi-session history stays stored exactly
   as today and becomes *more* accessible than today, via a dedicated
   read-only **Agent History** view reachable from a link where the old
   content used to be — with day separators and session-boundary dividers for
   easy perusal.

---

## 1. Current state (verified against `main` @ `9720ce4c2`, 2026-08-09)

### 1.1 The boundary marker exists and is persisted — but display doesn't use it

`SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md` Part A landed:

- Backend: `persistent_resume.rs` emits `ResumeEffect::EmitSessionOutcome`
  whenever a `--resume` attempt's fate becomes known; `persistent.rs` persists
  it as a `{"type":"system","subtype":"agentmux_session_outcome","outcome":
  "resumed"|"fresh",...,"timestamp":...}` NDJSON line appended to **both** the
  per-channel blockfile and the global mirror zone — it is a durable line in
  the same stream as the conversation itself.
- Frontend: both the live path (`useAgentStream.ts`) and the replay path
  (`parseHistoryLines.ts:150-165`) intercept that frame into a
  `SessionOutcomeNode`, rendered as a divider row —
  *"Session continued"* / *"New session started — prior conversation is not
  available to this agent"* (`virtualization/DocumentRow.tsx:303-315`).

So the pane is now *honest* about the boundary — but the pre-boundary
conversation still fills the scrollback above the divider. The human still
reads dead context by default; the divider is a caption, not a scope.

### 1.2 Restore replays everything in `:current`, boundary-blind

- Store shape: `agent:<definitionId>:current/` holds `output` (raw NDJSON) +
  `output.state.json` (UI snapshot) in the **global** (cross-channel)
  FileStore, mirrored on every append
  (`agent_session/{zone_naming.rs,session_io.rs}`,
  `blockcontroller/shell.rs::resolve_global_output_zone`).
- On mount, `useHistoryPagination.ts` reads the snapshot + the trailing
  `RESTORE_WINDOW_LINES = 5_000` lines of `output`, replays via
  `parseHistoryLines`, and pages further back `PAGE_SIZE = 200` lines at a
  time on scroll — across any number of `fresh` boundaries. Multiple logical
  sessions accumulate in one undifferentiated stream.
- Nothing clears `:current` in practice: `/clear` is frontend-only
  (`reducer.ts` `UserClear` — and correctly so: it's a visual reset, not a
  model-context event); the per-agent archive RPC
  (`agent:session:archive` → `archive.rs::archive_session`, which archives to
  `agent:<defId>:archive:<ts>` and clears `:current`) has **no non-test
  frontend caller**.

### 1.3 There is no history-browse UI at all

- `agent:session:list_archives` RPC + the typed
  `AgentSessionListArchivesCommand` binding (`rpc-api/session.ts:29`) exist
  but nothing renders them.
- The backend `HistoryService` (`backend/history/`, RPCs `history.List/Get/…`
  in `server/service/misc.rs`) indexes provider-CLI transcripts on disk and
  likewise has zero frontend callers.
- The agent pane has **no transcript-swap mechanism** — accessories
  (`PaneRow`: failure row, session digest, ActivityDock) stack *around* the
  always-mounted `AgentDocumentView`; nothing replaces it.

### 1.4 Timestamps are sparse in replayed history

Most provider events carry no wire timestamp, and replay deliberately doesn't
invent them (`parseHistoryLines.ts:57` — thinking/tool_call events get
`timestamp: 0` or none). The reliably-timestamped records are the synthetic
frames AgentMux itself writes (`agentmux_session_outcome`, `compact_boundary`)
and result-type frames that happen to include one. Day separators need a
timestamp source that actually exists (§4.4).

---

## 2. Goals / non-goals

**Goals**

1. After a `fresh` session outcome, the working scrollback contains only
   post-boundary content, headed by a single link row into Agent History.
   This holds on live occurrence (boundary event arrives while the pane is
   open) *and* on restore (pane re-opened later, any instance/channel).
2. A read-only **Agent History** view per agent pane: the entire retained
   history (all sessions in `:current`, and — phase 3 — archived zones),
   navigable with **day separators** and the existing session-outcome
   dividers.
3. Zero change to what is stored. Truncation is display-scope only; the
   global transcript zone keeps accumulating exactly as today.
4. Old agents whose streams contain no `fresh` boundary render exactly as
   today (the clamp simply never engages). No migration.

**Non-goals**

- Introspecting the provider CLI's real context window (impossible from here;
  the Aug 5 spec's audit F2 stands). "Actual knowledge" means: the content
  since the last *known* fresh boundary — the closest honest approximation
  AgentMux can compute.
- Treating `/clear` or compaction as scope boundaries. `/clear` is a visual
  reset (model keeps context); compaction keeps summarized context and
  already has its own divider. Neither truncates.
- Deleting/GC'ing history, cross-agent browse, search, labels — that's the
  unified-history-store spec's P2 (account-wide) and later phases here.
- The headless drone/`run_agent` path — same carve-out as the Aug 5 spec:
  only the persistent-controller pane path emits session outcomes today.

---

## 3. Part 1 — session-scoped working scrollback

### 3.1 Scope rule

The working pane's scope anchor is **the most recent
`agentmux_session_outcome` line with `outcome: "fresh"`** in the agent's
stream. Content strictly older than the anchor is out of scope for the
working view. `outcome: "resumed"` lines are *not* anchors (the model really
does have the prior turns — showing them is correct).

### 3.2 Restore path (`useHistoryPagination.ts`)

- **Initial window:** after `parseHistoryLines` produces the restore nodes,
  scan the parsed batch for the last `session_outcome` node with
  `outcome === "fresh"`. If found, drop every node before it and mark the
  pagination model **terminal** (no further `loadOlder`), replacing the
  "load older" affordance with the **history link row** (§3.4).
- **Paged window (`loadOlder`):** each 200-line page is parsed before
  prepending; if the parsed page contains a `fresh` session-outcome node,
  prepend only the nodes at-or-after it, mark terminal, show the link row.
- This is **frontend-only** — no new RPC, no backend change. The boundary is
  already a line in the very stream being paged; the clamp is a parse-time
  filter. (An alternative — backend records a `session:boundary_line`
  high-water mark in block meta so restore can seek directly — is a
  performance refinement, deliberately deferred: the tail-anchored window
  means restore already reads only the trailing 5k lines, and in the common
  case the boundary is inside or near that window.)
- The snapshot fast-path (`output.state.json`, schema v2) restores a
  ready-made node list; apply the same drop-before-last-fresh-boundary filter
  to the snapshot's nodes. The snapshot writer needs no change.

### 3.3 Live path (`useAgentStream.ts` + reducer)

When a live `agentmux_session_outcome` frame with `outcome: "fresh"` arrives
(today it becomes a divider node via `parseSessionOutcomeFrame`), also
dispatch a new reducer action:

```ts
{ type: "SessionScopeTrim", boundaryNodeId: string }
```

`reducer.ts` handles it like a scoped `UserClear`: remove all document nodes
older than the boundary node, keep the boundary divider itself as the first
row, and set a `hasEarlierHistory` flag in pane state that renders the
history link row above it. (Same file/action-shape conventions as the
existing `UserClear` at `reducer.ts` and the `StreamTruncate` reconnect path —
both are precedent for bulk node removal without store mutation elsewhere.)

Timing note: `fresh` outcomes are emitted at spawn/retry resolution — i.e.
between turns, never mid-assistant-message — so the trim never races an
in-flight streaming node. (The `FireRetry` and `SessionCaptured` emission
points in `persistent_resume.rs` both precede any post-respawn content.)

### 3.4 The history link row

A single row pinned at the top of the truncated scrollback:

> ⌛ **Earlier conversations** — this agent started a new session on
> <date>; prior history is preserved. **Open Agent History →**

- Implemented as a `PaneRow` (`components/PaneRow.tsx`) — the established
  accessory primitive (failure row / digest / dock all use it) — rendered in
  the slot where `useHistoryPagination`'s "load older" indicator lives when
  the model is terminal-with-history. It is **not** a `DocumentNode` (it must
  not persist, not virtualize, not appear in Agent History itself).
- Clicking it opens the Agent History view (§4) scrolled to the boundary the
  working pane was clamped at, so "what came right before this?" is one
  click + zero hunting.
- When the working session itself contains `resumed` dividers or compaction
  dividers, those render inline as today — the link row only marks the
  *fresh* scope edge.

---

## 4. Part 2 — the Agent History view

### 4.1 What it is

A **read-only, full-stream transcript reader for one agent**, presented as an
alternate body of the agent pane (the pane's chrome — tabs, header — stays;
the transcript body and composer swap out). It renders the *entire*
`agent:<definitionId>:current/output` stream — all sessions, boundary-blind —
using the same node pipeline as the live pane, plus:

- **Day separators** — synthetic divider rows between calendar days (§4.4).
- **Session dividers** — the existing `session_outcome` rows, unfiltered, so
  every "new session" edge is visible in place.
- **Compaction dividers** — as today.
- Backward pagination all the way to line 0 (the existing
  `loadOlder`/200-line machinery with the §3.2 clamp *disabled*).

No composer, no send, no tool interactivity beyond expand/collapse and copy.
A prominent "← Back to conversation" affordance returns to the working view.

### 4.2 View-swap mechanism (new, minimal)

There is no existing transcript-swap primitive (§1.3), so add the smallest
one that works, scoped to the agent view:

- `agent-view.tsx` gains a `bodyMode: "live" | "history"` signal (default
  `"live"`).
- `"history"` renders `<AgentHistoryView>` in place of the
  `AgentDocumentView` + composer subtree. The live subtree unmounts;
  `useAgentStream`'s subscription stays owned by the live subtree, so history
  mode does not double-subscribe. New live output while browsing history is
  not streamed into the history view — on return to `"live"` the normal
  mount/restore path (already built for reconnect) catches the pane up.
- This is deliberately *not* a new pane `viewType`/widget: history is a mode
  of the agent pane (like the cog → settings panel precedent), not a place.
  It needs the pane's `definitionId` context and should never appear in pane
  pickers or persisted layouts. (If a standalone cross-agent history *pane*
  materializes later, that's the unified-history-store spec's P2 — different
  surface, shares §4.3's data layer.)
- Entry points: the §3.4 link row; a "View full history" action on any
  `session_outcome` divider row; and an item in the pane's control/cog menu
  (`AgentControlBar.tsx`) so history is reachable even when the pane has no
  boundary (e.g. to scroll a very long single session with day separators).

### 4.3 Data layer — reuse, generalized

`useHistoryPagination` is already 90% of the reader; extract/parameterize
rather than fork:

```ts
useHistoryPagination(opts: {
  definitionId: string,
  scope: "session" | "all",   // §3 clamp on/off
  store: AgentDocumentStore,   // history view gets its own store instance
})
```

The history view instantiates its own document store + virtual list
(`AgentDocumentVirtualList` is presentation-only and reusable as-is) in
read-only configuration. Same `agent:session:read` RPC, same
`parseHistoryLines`, same renderer registry — no new backend endpoints for
phase 2.

### 4.4 Day separators with sparse timestamps

Rule: emit a `day_divider` synthetic row whenever the **best-known timestamp**
crosses a local-calendar-day boundary between consecutive nodes, where
best-known = the node's own timestamp if present, else the last preceding
known timestamp (carried forward). Untimestamped runs therefore group under
the last known day rather than inventing times — honest and stable.

- Reliable in-stream sources today: `agentmux_session_outcome` (RFC3339,
  backend-stamped), `compact_boundary`, and result frames that carry
  `timestamp`. In practice every session start is stamped, so day resolution
  is at worst "the day the session containing this message started" — already
  sufficient for perusal.
- **Optional backend refinement (phase 3):** stamp receive-time on mirror.
  The global-zone append path (`shell.rs::handle_append_block_file` /
  `resolve_global_output_zone`) appends batches whose arrival time it knows;
  writing a sidecar `output.tsidx` (NDJSON of `{line, unix_ms}` per batch) —
  additive, separate file, no change to `output`'s format or existing
  parsers — gives the history view dense timestamps for exact day edges and
  future "jump to date." Explicitly not required for phase 2.
- `day_divider` rows are render-time synthetics (like the link row, not
  persisted, not part of the node stream), id'd `day-<YYYY-MM-DD>` so
  pagination prepends merge instead of duplicating — same stable-id rationale
  as `sessionOutcomeNodeId`.

### 4.5 Archived sessions (phase 3)

`agent:<defId>:archive:<ts>` zones are currently write-only-in-theory (§1.2).
Once the Agent History view exists, wire them in:

- The history view's top (when line 0 of `:current` is reached) shows an
  "Archived sessions" section fed by the already-existing
  `AgentSessionListArchivesCommand`; selecting one loads that zone's `output`
  through the same reader.
- This also finally gives the archive RPCs a consumer, and makes an eventual
  "archive on fresh boundary" rotation policy (size control for huge
  `:current` zones) safe to introduce without any UX regression — history
  view users won't notice where `:current` ends and archives begin. Rotation
  itself is out of scope here.

---

## 5. Scope and blast radius

- **Frontend (phases 1–2, the bulk):**
  `hooks/useHistoryPagination.ts` (clamp + `scope` param),
  `useAgentStream.ts` (+`SessionScopeTrim` dispatch),
  `store/agent-document/reducer.ts` (+one action, `UserClear`-shaped),
  `agent-view.tsx` (`bodyMode` + link-row + menu entry),
  new `view/agent/history/AgentHistoryView.tsx` (+ day-divider injection,
  ~small — composition of existing pieces),
  `virtualization/{DocumentRow.tsx,renderers.ts,expansion-source.ts}`
  (+`day_divider` row, `context_compacted`-shaped),
  `components/AgentControlBar.tsx` (menu entry).
- **Backend:** none for phases 1–2. Phase 3: `output.tsidx` sidecar append +
  archive-section reads (existing RPCs).
- **Storage/format:** no changes to `output`, `output.state.json`, zone
  naming, or any RPC contract in phases 1–2.
- **Risk concentration:** the §3.2 clamp sits in the restore path that has
  had recent race/regression history — it must be a pure post-parse filter
  with no changes to read offsets/high-water-mark bookkeeping, so a clamp bug
  can at worst show too much/too little scrollback, never corrupt pagination
  state.

## 6. Phasing

| Phase | Scope | Outcome |
|-------|-------|---------|
| **P1** | §3: restore + live clamp at last `fresh` boundary; history **link row** (links to nothing yet → opens a "coming in P2" no-op is not acceptable — P1 ships only if P2 ships in the same release train; otherwise P1's link row is feature-flagged off and P1 is just the clamp + a static "prior history preserved" note on the divider) | Working pane = agent's actual knowledge |
| **P2** | §4.1–4.4: Agent History view, `bodyMode` swap, day separators (sparse-timestamp rule), entry points | Full history perusable, per agent, in-pane |
| **P3** | §4.5 archives section; `output.tsidx` dense timestamps; jump-to-date/boundary navigation | Complete retention story + groundwork for rotation |

## 7. Testing

- **Reducer/parse units:** clamp-at-boundary (boundary inside initial window;
  boundary found mid-`loadOlder` page; multiple fresh boundaries → newest
  wins; `resumed`-only stream → no clamp; no-boundary legacy stream → no
  clamp), `SessionScopeTrim` (trim keeps boundary node, is idempotent,
  no-ops when boundary is already first), day-divider injection (carry-
  forward rule, id stability across pagination prepends, no divider between
  same-day nodes).
- **Fixture replay:** extend the existing `parseHistoryLines.test.ts` /
  session-outcome fixtures with a two-session NDJSON fixture (session A →
  `fresh` outcome → session B) asserting: working restore shows only B +
  divider; history mode shows A + divider + B with a day separator when A/B
  timestamps straddle midnight.
- **Manual/live:** kill a resume (the `persistent_resume.rs` unreachable
  path) with the pane open → observe live trim + link row; reopen the pane
  cross-channel → observe clamped restore; open Agent History → full stream
  with separators; back → live view intact and streaming.

## 8. Open questions

1. **Should the `fresh` divider offer one-click "show anyway"** (temporarily
   un-clamp in place, without entering history mode)? Cheap to add
   (`scope: "all"` re-restore), but it re-creates the ambiguity Part A
   removed — lean **no**; the history view is one click away and stays
   honestly labeled.
2. **Session count / preview on the link row** ("3 earlier sessions,
   last active Aug 7")? Requires a cheap backend boundary count (scan or
   meta counter). Nice-to-have; defer to P3 alongside `tsidx`.
3. **Does history mode need to survive pane re-mount** (persisted
   `bodyMode`)? Lean no — always reopen in `"live"`; history is a transient
   reading posture.
