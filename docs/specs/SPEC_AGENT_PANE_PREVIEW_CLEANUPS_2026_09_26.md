# Spec: Agent-pane preview cleanups (message shell, scroll hand-off, cap estimates, file preview)

**Date:** 2026-09-26
**Status:** active — §1 implemented in #3932; §2 in #3933, §3 in #3934, §4 in #3936
**Scope:** `frontend/app/view/agent/` — message blocks, capped preview boxes,
row-height estimates, file previews
**Verified against:** `main` @ `6b7b74ef1`
**Source:** `docs/reports/REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26.md`
items B2, B3, B4, C6, C8 (a second instance) and D1. The larger items (A1, B1)
shipped as `SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26` and
`SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26`.

---

## 1. Message shell (B2, D1)

`JektBubble.tsx` and `AgentMessageBlock.tsx` are one component written twice:
- a root that toggles on click;
- a summary line starting with a `▸`/`▾` chevron;
- a body shown when open, with clicks stopped from toggling;
- the identical time + token peek (`useTick`, `useNodePeek`, two memos,
  `PeekOverlay`).

They differ only in the summary's content, the body's content, and the root's
classes. The two chevrons also have identical CSS under two names
(`.agent-message-chevron`, `.agent-jekt-chevron`).

**Change:** `components/CollapsibleMessage.tsx` owns the root, toggle,
chevron, body gate and peek. It takes:
- `rootClass` and `classes`
- `collapsed` and `onToggle`
- `summary` and `body` (JSX)
- `peekText` (the string the token estimate counts) and `timestamp`

Both components become thin wrappers that supply their summary, body and
classes. The existing class names stay (tests and styles use them). The two
chevron rules share one SCSS placeholder.

## 2. One scroll hand-off for every capped preview box (B3)

`SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03` found, live, that native scroll
chaining does not do what users expect. Chromium latches a wheel gesture to
the inner box, so reaching its edge mid-gesture either dead-ends or jerks the
pane.

The tool log's box therefore sets `overscroll-behavior: contain` and forwards
the next wheel tick to `.agent-document` itself (`ToolOverlayLog.tsx`, the
`onWheel` in its first `onMount`).

The jekt body and its raw-payload box (#3861) use the same cap but native
chaining. The #3861 comment chose that on purpose, but it's exactly the
behavior the scroll-chaining spec replaced.

**Change:** move the hand-off into `components/scroll-handoff.ts`:
`attachScrollHandoff(el)` returns a cleanup function. It covers the same logic:
- `Ctrl` + wheel is zoom, never forwarded;
- the 1 px boundary slack;
- `preventDefault` only when handing off.

Use it from `ToolOverlayLog`, the jekt body and the jekt raw payload. Give those
two jekt boxes `overscroll-behavior: contain`. Every capped transcript box then
scrolls the same way.

**Later (2026-10-04):** the hand-off no longer forwards the first wheel notch that
reaches a box's edge. The first few notches there (`SKID_NOTCHES`) are absorbed (a
"skid", with a line on that edge), and the next one scrolls the pane. See
`SPEC_TOOL_PREVIEW_WHEEL_EDGE_SKID_2026_09_27.md`.

## 3. Estimates follow the preview cap (B4)

The CSS cap is `$transcript-preview-max-height: calc(50vh / 3)`. The estimates
are fixed numbers that assume a 1400 px window:
- `TOOL_EXPANDED_PX` 200
- `CONTENT_FIRST_TOOL_ESTIMATE_PX` 280
- `JEKT_EXPANDED_MAX_ESTIMATE_PX` 290

**Change:** `previewCapPx()` returns `window.innerHeight / 6`, which is the
same value in unzoomed CSS px. Each estimate becomes the cap plus its row's
chrome:
- **Content-first tool:** cap + 47.
- **Jekt:** `min(text, cap + 57)`.
- **Generic expanded tool:** `min(200, cap + 40)`. The 200 stays at typical
  window sizes, so it never over-estimates on a small window.

The constants' 1400 px values are kept as the test expectations at a
1400 px `innerHeight`.

## 4. Shared file preview, and props that go stale (C6, C8)

- **`FilePreview`:** `renderRead` and `renderWrite` (`tool-renderers/builtins.tsx`)
  share a pipeline: head-cap → dedent/format → markdown or highlighted code →
  hidden-lines marker. They differ in the formatter (Read's gutter-aware
  `formatReadPreview`, Write's `formatCodePreview`) and the header (Write adds a
  byte count). A `FilePreview({ path, text, format, header })` component holds
  the pipeline. Write's reason for not using the gutter-aware formatter stays
  as that argument's documentation.
- **`BashOutputViewer`** destructures its props
  (`({ params, result }) =>`), the same frozen-at-mount hazard #3877 fixed in
  `CompactResult`. Switch to `props.x`.

## 5. Not doing

- **Report D2** (fold `ToolBlockOverlay`'s status-word header into the row):
  that removes the visible "failed" / "denied" / "canceled" label from an
  expanded panel. It's a design change, not a cleanup; it needs a design
  decision first.
- **Report C7** (a shared truncate helper): moot. The structured summary no
  longer truncates strings (#3877, #3902).

## 6. Phasing and tests

One PR per section.

1. **Message shell:**
   - The existing `JektBubble` and `AgentMessageBlock` tests pass unchanged.
   - A `CollapsibleMessage` test covers the toggle, the body gate, the body
     click not toggling, and the peek.
2. **Scroll hand-off:**
   - Unit tests for `attachScrollHandoff`: forwards at the top or bottom in
     the scroll direction; doesn't forward mid-range; ignores Ctrl+wheel;
     cleanup removes the listener.
   - The existing `ToolOverlayLog` wheel test passes.
   - Live: a wheel at a jekt body's edge moves the pane on the next tick.
3. **Estimates:** `renderers.test.ts` at a stubbed `innerHeight` of 1400
   (today's values), 700 (smaller values) and 2400.
4. **File preview and props:**
   - The Read/Write renderer tests and the `DocumentRow` `.md` preview test
     pass unchanged.
   - A `BashOutputViewer` test updates its result in place and shows the new
     output.
