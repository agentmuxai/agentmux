# Ultra-Long Agent Sessions

**Date:** 2026-04-12
**Goal:** Agent pane stays responsive and complete after 8+ hours of streaming,
with full history preserved and memory bounded.

## Problem

The current bundle (PR #334) caps the document at 500 nodes to keep typing smooth.
This works but **drops history** — after 500 nodes, older messages are evicted and
gone. For a multi-hour session, the user loses context: earlier exchanges, tool
results, and Claude's reasoning all disappear.

What we actually want:
- **Full history** retained and scrollable
- **Typing stays smooth** even with 10,000+ nodes in the session
- **Memory bounded** — no OOM after 8 hours
- **Session survives restart** — resume from where you left off

Three separate concerns:
1. **Rendering** — the DOM can't hold all history
2. **Memory** — parsed markdown + highlight.js output grows without limit
3. **Persistence** — session state vanishes on pane close

## Design

### 1. Virtualized document (windowing)

Replace the flat `<For each={document()}>` with a windowed renderer that only
mounts visible nodes + a small buffer above/below.

**Approach:** Use `IntersectionObserver` + `content-visibility: auto` (already
applied). Each node wrapper has a known intrinsic size; off-screen ones are
placeholders. Visible ones mount their real content.

**Why not a library:** `@tanstack/virtual` and friends bring React baggage or
require fixed row heights. Markdown nodes have variable height, and highlight.js
output needs to fully render before its height is known. A hand-rolled approach
using `content-visibility` is simpler and browser-native.

**Implementation:**
- Keep `document()` signal as the full array (source of truth)
- `<For>` over all nodes but each wrapped in `content-visibility: auto`
  (already shipped in PR #334)
- Browser automatically skips layout/paint for off-screen — **this is the
  virtualization**. We don't need a separate library.
- Remove the 500-node cap from `useAgentStream.ts` — it's no longer needed

**Validation:** With `content-visibility: auto`, a 10,000-node document should
have the same scroll perf as a 500-node document, because only ~20 nodes are
actually rendered at any time.

### 2. Bounded node memory

Virtualization solves paint/layout cost but not memory. Each node holds:
- Raw markdown text
- Parsed MDAST tree (via unified)
- Rendered HTML with highlight.js spans
- SolidJS reactive metadata

At 5000 nodes × ~2KB each = 10MB, plus the parsed trees can be 5-10× that. A long
session could easily hit 100MB+.

**Approach:** Lazy parse. Keep `{ id, type, content }` in memory, only run the
unified pipeline when the node becomes visible. Unmount → drop parsed state.

**Implementation:**
- `MarkdownBlock` checks a visibility state, only calls `Markdown` component when
  visible
- Use an IntersectionObserver to track visibility
- When a node scrolls off-screen for >30 seconds, reset the parsed state
- When it scrolls back, re-parse (cheap — happens once per scroll)

**Tradeoff:** First scroll to a past message has a small parse delay (~10ms per
node). Subsequent scrolls are instant. Acceptable.

### 3. Session persistence

The agent's full history should survive pane close / app restart.

**Current state:**
- Persistent process: stores session ID, uses `--resume <id>` on next turn
- Document nodes: in-memory only, lost on pane close
- CLI itself: Claude Code writes `~/.claude/projects/<hash>/<session>.jsonl` with
  the full turn history

**Approach:** On pane mount, if the block has an `agent:sessionid`, load the
last N turns from the CLI's session file and seed the document signal. The
user sees their history immediately.

**Implementation:**
- New RPC: `agent.history` → returns last N events from the session file
- Backend reads `~/.claude/projects/<hash>/<sessionid>.jsonl` and returns parsed
  events
- Frontend on mount: if `agent:sessionid` exists, call `agent.history` and
  populate `documentAtom` with the reconstructed events
- User sees their history, can scroll back, can continue the conversation

**Caveat:** The session file path is Claude Code specific. For Codex/Gemini,
a similar mechanism may not exist — check provider-by-provider.

### 4. Backpressure for streaming

A single Claude response can emit 500+ text delta events. If we're not evicting
and rendering is slow, the event queue grows.

**Approach:** The existing RAF batching handles this — events accumulate in
`pendingNew`, flush once per frame. If RAF is slow, events batch larger. No
explicit backpressure needed.

**Validation:** Stress test — stream a 10,000-word response and check that:
- Typing stays smooth during streaming
- No dropped characters
- Final document renders completely

### 5. Fast scroll

With full history, the user should be able to jump to any message instantly.

**Approach:**
- Add a "jump to start" button (scroll to top)
- Add a "jump to latest" button (scroll to bottom)
- Keyboard: `Home` and `End` in the document container
- Optional: search bar that filters or highlights matching messages

## Implementation Order

### Phase 1 — Remove the caps (Quick win)

1. Remove 500-node cap in `useAgentStream.ts`
2. Remove 50-line cap in `agent-view.tsx`
3. Rely entirely on `content-visibility: auto` for virtualization
4. Validate: open a pane, stream a long response, confirm typing stays smooth
   and all history is preserved

### Phase 2 — Lazy markdown parse

1. Wrap `MarkdownBlock` in an IntersectionObserver
2. Parse on enter, unparse on long exit
3. Add a simple "parsed/unparsed" state signal per node
4. Validate: memory stays bounded after 1000+ nodes

### Phase 3 — Session resume

1. Backend: `agent.history` RPC handler reads session file
2. Parse Claude Code's `.jsonl` format into DocumentNodes
3. Frontend: on mount, if session ID exists, load history
4. Validate: close pane mid-conversation, reopen, see history

### Phase 4 — Navigation

1. Jump-to-top/bottom buttons in agent footer
2. Keyboard shortcuts
3. Optional: search/filter

## Open Questions

**Q1:** Does `content-visibility: auto` actually handle 10,000 nodes smoothly, or
do we need proper virtualization with windowing?
**A:** Need to measure. Chrome's implementation uses `IntersectionObserver`
internally and is typically very efficient. Test with a stress fixture first.

**Q2:** What's the Claude Code session file format?
**A:** Need to check `~/.claude/projects/` on a real session. Likely NDJSON
with the same `stream_event` wrappers we already parse.

**Q3:** How do we handle partially-streamed responses at resume time?
**A:** The session file only contains committed turns. An in-progress stream
is lost on pane close. Acceptable.

**Q4:** Memory target?
**A:** Agent pane should use < 200MB RAM after 4 hours of streaming. Measure
and iterate.

## Success Criteria

- [ ] Open agent pane, send 100 messages, scroll through full history
- [ ] Close pane, reopen — all history restored
- [ ] 8-hour stress test: pane stays responsive, memory < 300MB
- [ ] Typing during heavy streaming: no visible lag
- [ ] Jump to any point in history: instant (< 50ms)

## What We're NOT Doing (yet)

- Full-text search across history
- Export session to markdown/JSON
- Multiple concurrent agent panes sharing a session
- Cross-device session sync

These are orthogonal and should be separate tracks.
