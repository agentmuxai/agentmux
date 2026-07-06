# ANALYSIS: Tool preview "jerk" on running → completed transition

**Date:** 2026-07-05
**Status:** Root causes confirmed by direct file inspection (see citations). Not previously
documented — `SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` describes a different, already-fixed
bug (native scroll-chaining dead zone / double-scrollbar); its own addendum explicitly says "Not a
jerk, a dead zone" — that "jerk" refers to browser scroll-bounce feel, not this layout reflow.

---

## Problem (as reported)

In the Agent pane, when a running tool call finishes, the tool-call preview visibly jerks — an
abrupt layout shift rather than a smooth transition — because the DOM shape rendered for the
`running` state differs from the DOM shape rendered for the terminal (`success`/`failed`/etc.)
state, and nothing bridges the two.

---

## Root cause: one state update flips two independent DOM shapes at once, and neither is transitioned

The `running`→terminal transition is driven by a single object replacement,
`mergeReplacement()` (`frontend/app/store/agent-document/reducer.ts:659-679`). When a
`tool_result` arrives, the parser's replacement node is merged with the existing node's live-log
buffer, and — critically — `log.open` is force-cleared to `false` in the same merge whenever the
new status is terminal:

```ts
const terminal =
    replacement.status === "success" ||
    replacement.status === "failed" ||
    replacement.status === "denied";
const mergedLog: ToolStreamingLog = {
    chunks: existingLog.chunks,
    open: terminal ? false : existingLog.open,   // line 676
};
```

So on the *same* render tick, `status` leaves `"running"` **and** `log.open` flips `true → false`.
Two independent pieces of UI key off exactly these two signals, and both change shape at once:

### A. Summary row — content swap, not a style change (`ToolBlock.tsx:256-322`)

- `resultPill()` (lines 177-229) explicitly returns `null` while `status === "running"` (line
  179: `if (s === "running" || s === "pending_approval" || !props.node.result) return null;`).
  So the result-pill `<span>` (lines 262-266) is **absent** while running and **appears** the
  instant status leaves running.
- The live-tail/elapsed-ticker block is gated on `props.node.log?.open === true` (line 274). It
  renders either the last stdout/stderr line (`.agent-tool-live-tail`, lines 286-294) or an
  `<ToolElapsedTicker>` (lines 296-301) while running, and is **removed entirely** the instant
  `log.open` flips false — the exact same tick the result pill appears.
- The status icon (`statusIcon()`, line 236, backed by the `STATUS_ICON` map) also swaps glyph
  in that tick.

Net effect: one flex row (`.agent-tool-summary`) loses one child (`.agent-tool-live-tail`) and
gains another (`.agent-tool-result-pill`) simultaneously, with no transition on either —
confirmed by `frontend/app/view/agent/styles/_document-nodes.scss`: the live-tail rule (around
line 219-241) and the result-pill rule (around line 424-442) carry no `transition` property, and
grepping the whole file for `transition` turns up only an unrelated hover rule (line 142) and the
panel collapse/expand transition (lines 297-307, see below — doesn't apply here).

### B. Panel body — two structurally different component trees swapped, not a style/height change (`ToolOverlayLog.tsx:90, 198-220`)

```tsx
const isStreaming = () => props.node.log?.open === true;   // line 90
...
<Switch>
    <Match when={isStreaming() && hasChunks()}>
        <ChunkList chunks={chunks()} />                      // raw streamed log lines
    </Match>
    <Match when={!isStreaming() && hasResult()}>
        <ToolOverlayResult node={props.node} />               // DiffViewer / BashOutputViewer / etc.
    </Match>
    ...
</Switch>
```

`isStreaming()` flips false in the same tick `result` is populated, so Solid's `<Switch>`
unmounts `ChunkList` and mounts `ToolOverlayResult` — a **different component**, per-tool
(`DiffViewer`, `BashOutputViewer`, `HighlightedCode`, etc.). The doc comment right above this
block (lines 187-196) confirms this was deliberately built as an exclusive `<Switch>` (rather
than a `<Show>` cascade) specifically to fix a *different* bug — a reconciler crash from
`ToolOverlayResult` appearing in two sibling `<Show>` branches during this exact transition. That
fix made the transition crash-safe; it did not make it visually smooth. `ChunkList` (raw log
lines) and `ToolOverlayResult` (e.g. a syntax-highlighted diff) can have very different natural
heights, and the container, `.agent-tool-overlay-log` (`_tool-overlay-portal.scss:34-68`), has
**no transition at all** on any property — confirmed by grep: zero `transition` hits in that
file. The outer `.agent-tool-panel` *does* have a 120ms transition
(`_document-nodes.scss:297-307`, on `max-height/padding/margin/opacity/content-visibility`), but
that only covers the hidden↔flow collapse/expand toggle — the panel stays in `flow` the entire
time here, so that transition never engages; it's the content *inside* the still-open panel that
jumps.

### C. Secondary contributor for non-success terminal states (`ToolBlockOverlay.tsx:49-63`)

```tsx
<div class="agent-tool-overlay-header" style={{
    display: props.node.status === "running" || props.node.status === "success" ? "none" : "",
}}>
```

The overlay header is suppressed for both `running` and `success`, so a plain successful Bash
call doesn't add/remove this row. But any tool terminating as `failed`/`denied`/`canceled`/
`pending_approval` shows this header for the first time on that same tick — another whole row
appearing with no transition (and `display` toggled via inline `style`, which CSS transitions
cannot animate as written regardless).

---

## Why this hasn't been caught by the scroll-chaining work

`SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` / PR #1963 fixed a *different* problem in the
same file (`ToolOverlayLog.tsx`) — double scrollbars and a wheel-scroll dead zone at the box's
scroll boundary. Its own addendum explicitly disambiguates: "Not a jerk, a dead zone." That
"jerk" language describes native browser scroll-chaining bounce, unrelated to the DOM-reflow jerk
here. No spec or analysis doc anywhere under `docs/specs/` or `docs/analysis/` mentions this
running→completed rendering jerk; the closest related docs
(`PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`,
`ANALYSIS_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`,
`SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md`, `SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`) are all
about *when* the panel opens/closes or *what* it streams, not the internal-content-swap-while-open
case.

---

## Why "universalize the form" is the right framing

The user's instinct — make the running-state and completed-state DOM shapes closer so there's
less to jerk over — is correct, but a pure CSS transition cannot fully solve part B: `ChunkList`
and `ToolOverlayResult` are genuinely different components with different natural content, so
there's no single element whose properties can be eased between them. Two complementary
directions:

### A viable fix shape (not yet implemented — for discussion before any code changes)

1. **Summary row (A):** stop keying the live-tail/elapsed-ticker's *presence* directly off
   `log.open`, and stop keying the result-pill's presence directly off `status !== "running"`.
   Instead, keep both elements mounted with a shared fixed-height wrapper and cross-fade via
   `opacity`/`transform` (absolute-position both, transition opacity), so the row's box stays a
   constant height across the swap and only the content cross-fades. This is a bounded, purely
   CSS/markup change in `ToolBlock.tsx:256-322` and `_document-nodes.scss`.
2. **Panel body (B):** this is the harder one, since `ChunkList` and `ToolOverlayResult` are
   different trees with different intrinsic sizes. Two candidate approaches:
   - **Height-only transition**: measure the outgoing tree's height before unmount (already have
     `scrollRef` in `ToolOverlayLog.tsx`), set an explicit `height` on `.agent-tool-overlay-log`
     equal to that measurement, swap the `<Switch>` branch, then transition `height` to `auto`
     (or the new measured height) over ~120-150ms matching the existing panel-collapse easing at
     `_document-nodes.scss:297-307`. This is the standard "FLIP"-style approach for swapping
     differently-sized subtrees and needs no new abstraction — `ToolOverlayLog.tsx` already does
     manual RAF-based DOM measurement for the auto-scroll-to-bottom logic (lines 164-185), so the
     pattern is already present in this file.
   - **Narrow the gap instead of bridging it**: when a tool streamed chunks and is now terminal
     with a structured result, consider keeping `ChunkList`'s last N lines visible *underneath* or
     *behind* `ToolOverlayResult` very briefly with a cross-fade, rather than an instant swap —
     more visual work, likely overkill for what's fundamentally a small-tool-call preview.
   - The height-measure-then-transition approach is recommended: it directly targets the box that
     currently has zero transition (`.agent-tool-overlay-log`), reuses an existing measurement
     pattern in the same file, and doesn't require touching the `<Switch>`'s crash-safety
     properties (must stay exclusive per the existing comment at lines 187-196).
3. **Header row (C):** same cross-fade/reserved-space treatment as (A), lower priority since it
   only affects non-success terminations.

### Scope/complexity note

Implementing (2)'s height-measure-then-transition needs care around the existing
`content-visibility: hidden` interaction (`ToolOverlayLog.tsx:138-162` already documents a
subtlety here — measuring a hidden subtree forces synchronous layout and emits console warnings)
and around the auto-scroll-to-bottom effect (lines 164-185) — a height transition mid-flight
would fight that RAF-based scrollTop assignment if not sequenced correctly (transition should
likely be skipped/instant when the panel isn't visible, matching the existing `panelHidden` guard
at line 180).

---

## Files referenced (all read directly, not inferred)

| File | Role |
|---|---|
| `frontend/app/store/agent-document/reducer.ts:659-679` | `mergeReplacement` — the single state update that couples `status` and `log.open`, root cause of both A and B firing on the same tick |
| `frontend/app/view/agent/components/ToolBlock.tsx:177-322` | Summary row — `resultPill()` (177-229), live-tail/elapsed-ticker (267-303), status icon (236), the flex row that gains/loses children (256-322) |
| `frontend/app/view/agent/components/ToolOverlayLog.tsx:90, 187-220` | `isStreaming()` gate + the exclusive `<Switch>` that swaps `ChunkList` ↔ `ToolOverlayResult`; comment at 187-196 explains its crash-safety history |
| `frontend/app/view/agent/components/ToolBlockOverlay.tsx:49-63` | Overlay header `display` toggle — secondary contributor for non-success terminations |
| `frontend/app/view/agent/styles/_document-nodes.scss:142, 219-241, 297-307, 424-442` | `.agent-tool-block` hover transition (142, unrelated); live-tail rule (219-241, no transition); panel hidden↔flow transition (297-307, doesn't cover this case); result-pill rule (424-442, no transition) |
| `frontend/app/view/agent/styles/_tool-overlay-portal.scss:34-68` | `.agent-tool-overlay-log` — the panel-body container; zero `transition` declarations anywhere in this file |

## Related docs (checked, confirmed unrelated or tangential)

| Path | Relevance |
|---|---|
| `docs/specs/SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` (PR #1963) | Different bug (scroll dead zone / double scrollbar) in the same file; explicitly disclaims being this "jerk" |
| `docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`, `docs/analysis/ANALYSIS_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md` | About *when* the panel auto-collapses on scroll, not content swaps while open |
| `docs/specs/SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md` | About *when* the panel auto-expands |
| `docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` | About what the live log streams, predates the terminal-state pill/switch design |

No prior doc addresses this issue — it appears genuinely unreported until now.
