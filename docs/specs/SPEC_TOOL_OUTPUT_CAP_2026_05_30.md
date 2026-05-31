# SPEC: Tool-Output Render Cap — interim DOM-bloat mitigation

- **Date:** 2026-05-30
- **Status:** Draft → ready to implement
- **Author:** AgentA
- **Related:**
  - `docs/analysis/ANALYSIS_AGENT_PANE_ARCHITECTURE_2026_05_30.md` — the rethink that surfaced this
  - `docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` — the tool overlay / live-log design (owns `onOpenInPane`)
  - `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` — the virtualizer this is a stepping-stone toward
  - Interim caps already landed (uncommitted): `ToolOverlayLog.tsx` (`MAX_VISIBLE_CHUNKS=500`), `BashOutputViewer.tsx` (`MAX_OUTPUT_LINES=800`)

---

## §0 TL;DR

The agent pane virtualizes the **conversation** but not the **content of a single tool's output**. A long tool call renders **one DOM node per line/chunk with no windowing** → thousands of in-flow nodes → scroll latency. (Observed live: "extremely slow, can hardly scroll" on a *completed* Bash tool — diagnosed as static DOM bloat, not a render/dispatch storm.)

Full inline virtualization is the eventual fix but it's a large, crash-prone, multi-component build (every result viewer + the documented `replaceChild` reconciliation surface) hot-reloaded into the live session. This spec defines the **interim**: a small, consistent **render cap** across every tool-output renderer — show the last N lines, mark what's hidden, and offer the full output on demand through the *existing* "open in pane" affordance. **Bounded DOM, no data loss, low risk** — and a clean stepping-stone to virtualization.

---

## §1 Problem & diagnosis

The conversation list (`AgentDocumentVirtualList`) is virtualized: off-screen rows are unmounted. But a single tool node's **output body** is rendered in full inside its row:

- **Streaming live log** — `ToolOverlayLog.tsx` `ChunkList` renders `<For each={chunks}>` → **one `<pre>` per chunk**, no windowing.
- **Completed Bash result** — `BashOutputViewer.tsx` dumps the **entire stdout into one `<pre>`** (plus stderr), no cap. This is the likely current-slowness path: a *completed* tool renders through `BashOutputViewer` (via `ToolOverlayResult`), not `ChunkList`.
- **Other result viewers** (`DiffViewer`, `HighlightedCode`, `CompactResult`, Read content) are likewise uncapped.

A tool with 10k+ lines therefore puts 10k+ DOM nodes (or one enormous `<pre>`) in the conversation flow. Because a recent tool sits in the always-mounted streaming buffer, it can't be scrolled away from → the whole conversation scroll degrades. Diagnosis confirmed it is **static DOM bloat** (idle `[fe]` log, low dev CPU, only a handful of error-boundary events all session) — i.e. a rendering-volume problem, not a reactivity storm.

---

## §2 Goals / non-goals

**Goals**
- Bound the DOM nodes contributed by any single tool's output, regardless of output size.
- Keep conversation scroll smooth no matter how large a tool's output is.
- **Never silently drop output** — always show a marker of what's hidden, and provide a path to the full content.
- One consistent mechanism across **all** tool-output renderers (no per-viewer ad-hoc caps).
- Tiny, low-risk, reversible; safe to hot-reload into a live session.
- A clean stepping-stone to full virtualization (the cap helper is later swapped for the virtualizer).

**Non-goals**
- Full inline virtualization of tool output (deferred — see §12).
- Terminal-pane (xterm.js) scrollback virtualization (separate concern; xterm owns its own rendering).
- Changing the tool-result data model or how chunks are stored. The full output remains in `ToolNode` state untouched; we only cap what is **rendered**.

---

## §3 Design overview

Three pieces, phased:

- **A. Shared cap helper + one budget constant** — replaces the two inline ad-hoc caps.
- **B. A consistent "N earlier lines hidden" marker** — one component, used everywhere we cap.
- **C. "View full output" bridge** — wire the existing (stubbed) `onOpenInPane` action to a tool-detail pane that renders the **full, uncapped** output. That pane is the right home for virtualization later (Phase 3).

| Phase | Scope | Outcome |
|-------|-------|---------|
| **1** | A + B across all renderers | Immediate, complete relief; no scroll degradation; hidden-count visible |
| **2** | C — wire `onOpenInPane` → tool-detail pane | Full output reachable on demand → cap is non-lossy |
| **3** *(future)* | Virtualize inside the Phase-2 pane | Full output rendered efficiently in a single focused scroll surface |

Phase 1 is the deliverable here (it already exists in hacky form and just needs to be generalized). Phases 2–3 are specified so the design is whole.

---

## §4 The shared cap helper

New file: **`frontend/app/view/agent/components/output-cap.ts`**

```ts
/** Max rendered lines of any single tool's output body. Above this we
 *  render a head/tail window + a hidden-count marker. The full output
 *  stays in ToolNode state and is reachable via "open in pane". */
export const MAX_TOOL_OUTPUT_LINES = 1000;

export interface CappedText {
  text: string;
  hiddenLines: number; // 0 when not capped
}

/** Cap a single output string to `max` lines, keeping the head or tail.
 *  Logs/output → "tail" (recent matters). File/diff content → "head". */
export function capText(
  text: string,
  max: number = MAX_TOOL_OUTPUT_LINES,
  from: "head" | "tail" = "tail",
): CappedText;

export interface CappedChunks<T> {
  chunks: ReadonlyArray<T>;
  hiddenLines: number;
}

/** Cap an append-only chunk list to the last `max` *lines* (not chunks),
 *  counting cumulative newlines from the tail. A single chunk whose own
 *  content exceeds `max` is itself line-capped via capText(.., "tail"). */
export function capChunksByLines<T extends { content: string }>(
  chunks: ReadonlyArray<T>,
  max: number = MAX_TOOL_OUTPUT_LINES,
): CappedChunks<T>;
```

Semantics:
- `hiddenLines` is `totalLines - keptLines` (0 when under budget) — drives the marker and its singular/plural text.
- `capText` splits on `\n`; `from:"tail"` keeps the last `max`, `from:"head"` keeps the first `max`.
- `capChunksByLines` walks from the end accumulating line counts until it would exceed `max`, then includes a partial tail of the boundary chunk (line-capped). Returns same-reference chunk objects where possible so SolidJS `<For>` keyed-by-reference reconciliation stays cheap on append.

Unit-tested in `output-cap.test.ts` (§11).

---

## §5 Per-renderer application

Replace every ad-hoc/inline cap with the shared helper. Cap policy is **tail** for log-like output (recent matters) and **head** for document-like output (read top-down).

| Renderer | File | Body | Helper + policy | Replaces |
|----------|------|------|-----------------|----------|
| `ChunkList` | `ToolOverlayLog.tsx` | streaming chunks (`<pre>`/chunk) | `capChunksByLines` (tail) | the inline `MAX_VISIBLE_CHUNKS=500` chunk-count cap |
| `BashOutputViewer` | `BashOutputViewer.tsx` | stdout + stderr | `capText` tail on **each** (independent budgets) | the inline `MAX_OUTPUT_LINES=800` |
| `DiffViewer` | `DiffViewer.tsx` | unified diff hunks | `capText` **head** (diffs read top-down) | none yet |
| `HighlightedCode` | `HighlightedCode.tsx` | code (already skips highlight > 2000 lines / 200 KB) | `capText` **head** when rendered as a result body | none yet |
| `CompactResult` | `CompactResult.tsx` | summary + `<pre>` JSON detail | `capText` tail on the JSON detail | none yet |
| Read / generic result | `ToolOverlayResult.tsx` branches | file/string content | `capText` **head** | none yet |

Notes:
- The budget is **per visible body**. stdout and stderr each get their own `capText` pass so a huge stdout doesn't hide a short stderr.
- `HighlightedCode` already has a highlight-skip guard at 2000 lines; the render cap (1000) sits below it, so highlighting cost is also bounded for free.
- Keep the `--system`-styled marker visually distinct from real output lines (§6).

---

## §6 The hidden-lines marker

New component: **`OutputHiddenMarker`** (in `output-cap.ts`'s sibling or inline in each viewer — single source of text):

```tsx
// Renders e.g.: "… 4,213 earlier lines hidden — open for full output"
<pre class="agent-tool-log-line agent-tool-log-line--system agent-output-hidden-marker"
     onClick={onOpenFull /* Phase 2 */}>
  … {hiddenLines.toLocaleString()} {hiddenLines === 1 ? "line" : "lines"} hidden
  {from === "tail" ? " (showing the latest" : " (showing the first"} {kept.toLocaleString()})
</pre>
```

Placement:
- **Tail-capped** (logs): marker at the **top** of the body (older content is above the window).
- **Head-capped** (files/diffs): marker at the **bottom**.

Until Phase 2 wires `onOpenFull`, the marker is informational (no click target) — the cap is mildly lossy *in the inline view only*; the full data is still in `ToolNode` state.

---

## §7 "View full output" affordance (Phase 2)

The tool overlay already surfaces an **open-in-pane** action:
- `DocumentRow.tsx:115` `onOpenInPane` — currently stubbed (`console.warn("[tool-overlay] open in pane — not yet implemented")`).
- Surfaced in the overlay action bar via `ToolOverlayActions`.
- `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` §4 earmarks Phase 4 to wire it to `createBlock({ view: "tool-detail" })`.

Phase 2 of *this* spec wires that path to open a **tool-detail pane** that renders the tool's **full, uncapped** output (chunks + result). The marker (§6) becomes a click target into the same pane. That pane is a single, dedicated scroll surface — the correct home for full virtualization (Phase 3), with no nesting inside the conversation virtualizer.

---

## §8 Streaming & scroll interactions

- `ToolOverlayLog`'s auto-stick-to-bottom (`stickToBottom`, the `isConnected`-guarded RAF scroll at `ToolOverlayLog.tsx:91-107`) is unchanged: the capped chunk window is small, so scroll-to-bottom stays cheap and correct.
- On each chunk append while streaming, `capChunksByLines` recomputes the tail window; the marker's `hiddenLines` count grows. Because the helper returns the same chunk object references for the retained tail, `<For>` reconciliation only mounts/unmounts at the window boundary (O(1) per append), not the whole list.
- The `<Switch>` branch structure in `ToolOverlayLog` (which exists specifically to avoid the `replaceChild` re-parent crash, see `ToolOverlayLog.tsx:109-118`) is **not** changed — the cap operates strictly inside `ChunkList`.

---

## §9 Edge cases

- Empty output → `hiddenLines === 0`, no marker.
- Output exactly at budget → no marker (only cap when `total > max`).
- A single chunk whose own content exceeds the budget → that chunk's `content` is line-capped via `capText(.., "tail")`; `hiddenLines` counts its dropped lines too.
- stdout + stderr both large → each capped independently; two markers possible (one per body) — acceptable and clear.
- Marker grammar: singular/plural on `hiddenLines`; thousands-separated via `toLocaleString()`.
- No ANSI parsing today (chunks render as plain text per `ToolOverlayLog` header note) — cap is line-based and ANSI-agnostic; revisit when ANSI lands (it only affects per-line height, not the line count).

---

## §10 Implementation steps (ordered)

1. **`output-cap.ts`** — `MAX_TOOL_OUTPUT_LINES`, `capText`, `capChunksByLines`, `OutputHiddenMarker`. + `output-cap.test.ts`.
2. **`ToolOverlayLog.tsx`** — `ChunkList` → `capChunksByLines` + top marker. Remove the inline `MAX_VISIBLE_CHUNKS` hack.
3. **`BashOutputViewer.tsx`** — stdout & stderr → `capText("tail")` + markers. Remove the inline `MAX_OUTPUT_LINES` hack.
4. **`DiffViewer.tsx`, `HighlightedCode.tsx`, `CompactResult.tsx`, `ToolOverlayResult.tsx`** — apply `capText` per §5.
5. *(Phase 2)* Wire `onOpenInPane` → tool-detail pane rendering full output; make the marker click into it.
6. Tests + smoke (§11).

Each step hot-reloads independently; steps 2–3 are the perf-critical ones (they subsume the current hacks).

---

## §11 Testing & verification

- **Unit** (`output-cap.test.ts`): `capText` head/tail, exact-budget boundary, empty, single-line, `from` correctness; `capChunksByLines` cumulative-line counting, single-oversized-chunk partial tail, reference-stability of retained chunks.
- **Integration**: a `ToolNode` with 10k-line output renders **≤ `MAX_TOOL_OUTPUT_LINES` + O(1)** body DOM nodes and exactly one marker with the correct `hiddenLines`.
- **Smoke** (the original repro): scroll a conversation containing a long completed Bash tool — smooth, no jank. Confirm the marker count + (Phase 2) that "open in pane" shows the full output.

---

## §12 Migration to full virtualization

The cap is deliberately a **stepping-stone**, not a dead end:

- The conversation view **stays capped** (cheap, bounded) — there is no strong UX need to scroll 50k inline lines inside a chat bubble.
- Full output lives in the **Phase-2 tool-detail pane**, a single flat scroll surface. That is where the `AgentDocumentVirtualList` pattern (`@tanstack/solid-virtual`, already a dependency) belongs: a **flat, append-only, immutable** line/chunk list needs only a standard virtualizer — no streaming-buffer/sticky-frontier hybrid, far simpler and lower-risk than virtualizing inside the conversation.
- When Phase 3 lands, the cap helper is removed from the pane path (still used inline for the conversation summary), and the `replaceChild` reconciliation surface is hardened by the move to a dedicated, single-purpose scroll container.

---

## §13 Rollout / PR plan

- **PR 1 (this spec's Phase 1):** `output-cap.ts` + marker + apply to all renderers + tests. The two current uncommitted hacks become the clean helper. Smoke the long-output repro before opening.
- **PR 2 (Phase 2):** wire `onOpenInPane` → tool-detail pane; marker links into it.
- **PR 3 (Phase 3, later):** virtualize the tool-detail pane.
- Versioning: changeset `type: patch` (`fix(agent): cap tool-output rendering to bound conversation DOM`). **Revert the local 0.40.x iteration bump before the PR commit** and add the changeset instead (feature PRs don't carry manual version bumps — see `CLAUDE.md` §Version Management).
