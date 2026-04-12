# Ultra-Long Session Support Plan

**Date:** 2026-04-12
**Status:** Draft
**Scope:** Agent sessions that run for days with massive conversation histories

---

## Problem Statement

When an agent session runs for days, the conversation history grows unbounded. Today:

- FileStore accumulates output indefinitely with no pagination
- WPS broker caps at 4096 in-memory events, but FileStore has no such limit
- Frontend renders the entire conversation in DOM — no virtualization
- FileStore cache holds decoded blocks in RAM (OOM risk, see `docs/analysis/oom-filestore-cache.md`)
- On reconnect/reload, there's no way to replay old output — only new appends arrive
- No session bookmarking, search, or navigation for long histories

A session running 3+ days could easily produce 50K–200K+ lines of output, causing memory bloat on the backend, DOM thrashing on the frontend, and a terrible UX for finding anything in the history.

---

## Design Goals

1. **Sessions can run for days without degradation** — memory, CPU, and responsiveness stay flat
2. **Full history is preserved and retrievable** — nothing is silently dropped
3. **Users can navigate long histories efficiently** — search, jump-to-time, bookmarks
4. **Reconnect/reload restores context** — not a blank slate
5. **Backward compatible** — existing short sessions work unchanged

---

## Phase 1: Backend — Bounded Memory + Paginated Retrieval

### 1.1 FileStore Output Pagination API

Add an RPC endpoint to query historical output with offset/limit:

```
blockfile:read_range { block_id, filename, offset, limit } -> Vec<Line>
```

- `offset` is line number (0-indexed), `limit` is max lines to return
- Also support `blockfile:line_count { block_id, filename } -> u64`
- This lets the frontend load history on demand instead of buffering everything

### 1.2 FileStore Cache Eviction

The current cache has a 60s TTL but large blocks can still blow up RAM:

- Add a **max cache size** (e.g., 128 MB) with LRU eviction
- For blocks exceeding a threshold (e.g., 10K lines), only cache the tail window (last N lines) in RAM
- Older data stays on disk, served via paginated reads

### 1.3 Output Segmentation

For sessions producing 100K+ lines, a single flat file becomes unwieldy:

- Segment output into **chunks** (e.g., 10K lines per segment file)
- Naming: `output.0000`, `output.0001`, etc.
- Append always targets the latest segment; reads span segments transparently
- Enables efficient seeking without scanning from byte 0

### 1.4 Session Metadata Tracking

Extend the existing HistoryService pattern to agent sessions:

- Track per-session: `start_time`, `last_activity`, `line_count`, `token_estimate`, `duration`
- Persist in WaveStore as a `SessionMeta` object
- Update on each append (debounced, not per-line)

---

## Phase 2: Frontend — Virtual Rendering + History Navigation

### 2.1 Scroll Virtualization

Replace full-DOM rendering with a virtualized list:

- Only render visible conversation nodes + a small overscan buffer
- Candidate libraries: `@tanstack/virtual`, or custom since we're SolidJS-based
- Each `DocumentNode` gets a measured height (estimate first, measure on render)
- Anchor to bottom by default (follow mode), but allow free scrolling

### 2.2 Paginated History Loading

On mount or reconnect:

- Load only the **last N messages** (e.g., 200) via the new `blockfile:read_range` API
- As the user scrolls up, fetch older pages (infinite scroll backward)
- Show a "Loading older messages..." indicator at the top
- Cache loaded pages in a sparse array keyed by line range

### 2.3 Reconnect / Reload Recovery

When the frontend reconnects to an existing session:

- Query `blockfile:line_count` to know total history size
- Load the tail window (last 200 messages)
- Resume live streaming from that point forward
- The user sees continuity, not a blank pane

### 2.4 Session Timeline / Minimap

For multi-day sessions, a simple scrollbar isn't enough:

- Add a **timeline minimap** in the gutter showing:
  - Time markers (hours/days)
  - Activity density (busy vs. idle periods)
  - Bookmarked positions
- Clicking a point on the timeline jumps to that position and loads surrounding context

---

## Phase 3: UX — Search, Bookmarks, and Session Management

### 3.1 In-Session Search

- `Ctrl+F` within the agent pane searches conversation history
- Backend-assisted: `blockfile:search { block_id, query, direction }` returns matching line numbers
- Frontend highlights matches and navigates between them
- Regex support optional (nice-to-have)

### 3.2 Bookmarks / Pins

Let users mark important moments in a long session:

- Right-click a message -> "Bookmark this" (or keyboard shortcut)
- Bookmarks stored as `Vec<Bookmark>` in SessionMeta
- Bookmark panel shows all pins with timestamps and preview text
- Jump to any bookmark instantly

### 3.3 Session Archival + Cleanup

- **Auto-archive:** Sessions inactive for >7 days get compressed (gzip segments)
- **Manual archive:** User can archive a session, freeing RAM but keeping history on disk
- **Export:** Download session as `.jsonl` or formatted `.md`
- **Cleanup policy:** Configurable max total storage for sessions (e.g., 2 GB), oldest archived sessions pruned first

### 3.4 Session Summary / Digest

For returning to a multi-day session after a break:

- Generate a **session digest** — key decisions, errors, milestones
- Could be AI-generated (send last N messages to a fast model for summarization)
- Show as a collapsible banner at the top when resuming a stale session
- "What happened while I was away" for sessions with background agents

---

## Phase 4: Resilience + Edge Cases

### 4.1 Graceful Degradation Under Load

- If a session exceeds 500K lines, show a warning and offer to archive + start fresh
- Rate-limit FileStore writes if output velocity exceeds threshold (e.g., runaway loops)
- Frontend: if render time per frame exceeds 16ms, increase virtualization buffer aggressively

### 4.2 Multi-Day Session Continuity

- Handle system reboots / AgentMux restarts mid-session:
  - Persistent process mode already supports `--resume <session_id>`
  - Ensure FileStore segments survive unclean shutdown (WAL mode helps)
  - On restart, detect orphaned sessions and offer to reconnect

### 4.3 Concurrent Access

- Multiple windows viewing the same session should stay in sync
- WPS already handles pub/sub — ensure paginated reads + live stream don't race

---

## Implementation Priority

| Phase | Effort | Impact | Recommendation |
|-------|--------|--------|----------------|
| 1.1 Pagination API | Medium | High | **Start here** — unblocks Phase 2 |
| 1.2 Cache eviction | Small | High | Do alongside 1.1 — prevents OOM |
| 2.1 Scroll virtualization | Medium | High | **Critical path** — biggest UX win |
| 2.2 Paginated loading | Medium | High | Pairs with 2.1 |
| 2.3 Reconnect recovery | Small | High | Quick win once 1.1 exists |
| 1.3 Output segmentation | Medium | Medium | Needed at scale, can defer initially |
| 3.1 Search | Medium | Medium | High user value, moderate effort |
| 2.4 Timeline minimap | Large | Medium | Polish — do after core works |
| 3.2 Bookmarks | Small | Medium | Nice UX, low effort |
| 3.3 Archival | Medium | Low | Only matters at scale |
| 3.4 Session digest | Medium | Medium | Cool but not blocking |
| 1.4 Session metadata | Small | Medium | Supports 2.4 and 3.x features |

---

## Suggested Sprint Plan

**Sprint 1 (Week 1-2):** Foundation
- 1.1 Pagination API
- 1.2 Cache eviction improvements
- 2.1 Scroll virtualization prototype

**Sprint 2 (Week 3-4):** Core UX
- 2.2 Paginated history loading (connect to API)
- 2.3 Reconnect/reload recovery
- 1.4 Session metadata tracking

**Sprint 3 (Week 5-6):** Navigation + Polish
- 3.1 In-session search
- 3.2 Bookmarks
- 2.4 Timeline minimap (if time)

**Sprint 4 (Week 7-8):** Scale + Resilience
- 1.3 Output segmentation
- 3.3 Archival + cleanup
- 4.x Edge cases and hardening

---

## Open Questions

1. **Segment size:** 10K lines per segment, or size-based (e.g., 1 MB)?
2. **Search backend:** SQLite FTS5 on output, or simple line-scan with ripgrep?
3. **AI digest:** Which model, what trigger (time idle? user return?), cost concerns?
4. **Multi-window:** Should two windows viewing the same session share scroll position or be independent?
5. **Existing sessions:** Migrate existing flat output files to segmented format, or only new sessions?
