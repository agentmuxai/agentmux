# SPEC: Tool-hover consolidation (2026-05-28)

**Author:** AgentA
**Reporter:** user (this session) — "in the agent pane, there appears to be 3 separate events happening on a tool hover, a small time popup, a larger log popup, and a fast expand/collapse. There should be NO expand/collapse on hover. It only is supposed to happen on new messages. We want the small popup (it's just the time) to be at the top of the larger popup, so it's just 1 popup. Also, in the large log popup, we repeat the same info in the line and in the popup, lets get rid of the dupe."
**Affected files (primary):**
- `frontend/app/view/agent/components/ToolBlock.tsx` (335 LOC)
- `frontend/app/view/agent/components/ToolBlockOverlay.tsx` (80 LOC)
- `frontend/app/view/agent/components/ToolOverlayLog.tsx` (259 LOC — body only, no header change)
- `frontend/app/view/agent/styles/_document-nodes.scss` (tool styling)
- `frontend/app/view/agent/components/ToolBlock.test.tsx` (tests for hover behavior)

**Supersedes/amends:** `docs/specs/tool-collapse.md` (the hover-expand model that this spec removes).
**Predecessors:**
- `docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md` — three-slot overlay layout
- `docs/specs/SPEC_TOOL_BLOCK_UX_POLISH_2026_05_23.md` — 150 ms enter delay, mouseleave-only collapse
- `docs/specs/SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md` — Phase B auto-expand for running/pending_approval
- `docs/specs/SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md` — hover-anchor mechanism (overlay direction picker)

---

## 1. Symptom inventory

The user observes **three distinct visual events** when hovering a tool row in the agent pane:

1. **Small time popup** — a small box appears briefly. It currently shows the tool *summary* text (browser-native `title=` attribute on `.agent-tool-name` at `ToolBlock.tsx:242`), but the user reads it as "the time popup" — likely conflated with the duration `(N.Ns)` shown beside the tool name. The browser-native tooltip has its own delay (~500 ms on Windows Chrome) so it appears *before or after* the larger panel depending on cursor speed.
2. **Larger log popup** — the `.agent-tool-panel` overlay with header / log body / action bar, rendered by `ToolBlockOverlay`. Appears 150 ms after `mouseenter`. Contains the full streaming log + structured result.
3. **Fast expand/collapse** — the panel animating in (after the 150 ms enter delay) and then out (on `mouseleave`, instantly). Even a brief cursor pass triggers the full open/close cycle because the leave is instant once the enter delay fires.

All three were intentional at one point — but together they overlap and conflict:

- (1) and (2) overlap visually. (1) shows the summary, (2)'s header *also* shows the summary. Stacked redundancy.
- (3) is the user's primary complaint. The intent of `SPEC_TOOL_BLOCK_UX_POLISH_2026_05_23` was a smooth peek-on-hover; the lived experience is "the row dances around my cursor."
- The overlay header (when (2) is open) repeats the status icon, tool name, summary, and duration — all four of which are already in the collapsed row right above it.

## 2. Desired behavior (per user)

| Trigger | Behavior |
|---|---|
| Tool is **running** or **pending_approval** | Auto-expand the unified panel under the row. (unchanged) |
| Tool transitions running → success/failed | Stay expanded for `POST_COMPLETION_HOLD_MS` (3 s), then collapse. (unchanged) |
| User clicks the collapsed row | Pin: unified panel stays open until next click. (unchanged) |
| User **hovers** any tool row (collapsed) | **Nothing visible happens.** No browser-native tooltip, no panel expand, no time popup. The row stays as-is. **(THIS IS THE CHANGE.)** |
| The unified panel is rendered (auto-expand, post-completion hold, or pinned) | Header shows **only** info that is *not* already on the collapsed row: timestamp (top), status label. Status icon + tool name + summary + duration are NOT repeated — they live on the row only. |

Net: **one popup, not three**. Hover-to-peek is removed entirely.

## 3. Rationale for removing hover-to-peek

Three prior PRs have already chased fallout from hover-induced expansions:

- **PR #988** — initial hover-anchor + 150 ms enter delay (intended to reduce flicker)
- **#988 round 2 codex P1** — fix for "loaded transcripts briefly auto-expand every completed tool row on initial render" (gating bug)
- **`SPEC_TOOL_BLOCK_UX_POLISH_2026_05_23`** — replaced the leave-delay with instant leave to address mouse-trap complaints

Each fix was correct at the time but the *cumulative* surface area is still flicker-prone. The user has now redefined the requirement: hover is for reading, not for revealing. Pin (click) is for revealing.

The active-tool auto-expand path is preserved verbatim. The expand-on-completion-hold is preserved verbatim. Only the hover-as-expand-trigger is removed.

## 4. Implementation

### 4.1 `ToolBlock.tsx` — remove the hover infrastructure

Delete the following state + effects:

- `hovering` signal (line 86)
- `expandDirection` signal (line 91)
- `overlayMaxHeight` signal (line 92)
- `enterTimer` + `rootEl` refs (lines 93-94)
- `handleMouseEnter` (lines 101-125)
- `handleMouseLeave` (lines 126-130)
- `onCleanup(() => clearTimeout(enterTimer))` (line 131)
- `HOVER_ENTER_DELAY_MS` constant (line 78)
- `TOOL_BODY_ESTIMATE_PX` constant (line 83)
- Imports of `findScrollContainerRect`, `maxOverlayHeight`, `pickExpandDirection`, `ExpandDirection` from `./hover-anchor` (lines 42-46)
- The `panelMode()` "overlay" branch (lines 217-221) — collapses to a binary `"hidden" | "flow"` mode.

`expanded()` simplifies:

```ts
const expanded = () => props.pinned || autoExpanded();
```

`panelMode()` simplifies:

```ts
const panelMode = (): "hidden" | "flow" => {
    return expanded() ? "flow" : "hidden";
};
```

Remove the `onMouseEnter` / `onMouseLeave` handlers from both the root div (lines 228-229) and the panel div (lines 321-322). The panel-row pair becomes a static collapsed-or-expanded pair driven purely by status + pin state.

Remove the `agent-tool-panel--overlay-below` / `agent-tool-panel--overlay-above` classes from the panel div (lines 301-304) and from the SCSS — they are now unreachable.

Delete the inline `max-height` style on the panel (lines 306-310) — it only mattered in the overlay branch.

The `inert={!expanded()}` / `aria-hidden={!expanded()}` accessibility gating (lines 318-319) stays.

### 4.2 `ToolBlock.tsx` — remove the browser-native `title` tooltip

Drop the `title={props.node.summary}` attribute from `.agent-tool-name` (line 242). The full summary is already visible in the row (no ellipsis truncation in current CSS) — the title tooltip was redundant *and* a source of the "small time popup" the user complained about. The `title=`s on the live-tail (line 260) and open-in-pane button (line 268) are unrelated and stay.

### 4.3 `ToolBlockOverlay.tsx` — remove duplicate header fields, add timestamp

Current header (lines 49-67):

```
[status-icon] [tool-name] [summary]                    [duration] [status-label]
```

Each of those repeats the collapsed row above it. New header — strip everything that's already on the row, and add the timestamp the user wants at the top of the unified popup:

```
[timestamp]                                                       [status-label]
```

- `timestamp` — formatted local time, e.g. `13:51:19` or `2 min ago`. Derived from `props.node.timestamp` (or whatever field already carries the tool's start time — see §5). Left-aligned.
- `status-label` — kept; `STATUS_LABEL[status]` ("running", "ok", "failed", …). Right-aligned. The user did not ask to remove this and it carries a different signal than the row's status icon (text vs glyph).

Delete from the header:

- `<span class="agent-tool-overlay-status">{STATUS_ICONS[status]}</span>` — status icon is already on the row.
- `<span class="agent-tool-overlay-tool">{props.node.tool}</span>` — tool name is already on the row (via `summary` which begins with the tool name for most tools; if the user disagrees we treat this as a follow-up).
- `<span class="agent-tool-overlay-summary">{props.node.summary}</span>` — summary is on the row.
- `<span class="agent-tool-overlay-duration">{duration}s</span>` — duration is on the row.

Keep the surrounding `.agent-tool-overlay-header` div + `.agent-tool-overlay-meta` div, just with fewer children. SCSS layout adjusts in §4.5.

### 4.4 Timestamp field — what to render

The `ToolNode` type (path: `frontend/app/view/agent/types.ts`, search for `interface ToolNode`) already carries either `startedAt` / `timestamp` / equivalent. Use that. Render via a tiny helper:

```ts
function formatToolTime(ms: number | undefined): string {
    if (ms == null) return "";
    const d = new Date(ms);
    // Local time in 24-h HH:MM:SS — minutes precision is too coarse
    // for distinguishing rapid tool sequences.
    return d.toLocaleTimeString(undefined, { hour12: false });
}
```

If `ToolNode` does not currently carry a timestamp, add it in the reducer at tool-create time (`agent-document/reducer.ts`, search for the `ToolNodeAppend` arm); use `Date.now()` at the dispatch site. This is a one-line addition; no migration concern because the field is purely visual.

### 4.5 SCSS adjustments — `_document-nodes.scss` and `_tool-overlay-portal.scss`

- Remove `.agent-tool-panel--overlay-below` and `.agent-tool-panel--overlay-above` rules — the overlay positioning mode is gone (`_document-nodes.scss` block around lines 253-348).
- Remove the `&:hover { ... }` rule on `.agent-tool-block` if any (line 132) that visually telegraphed the now-removed peek behavior. Keep the row's own subtle hover-background (visual feedback that the row is interactive) but make it identical to siblings — no expand affordance.
- Reduce the overlay header SCSS (`_tool-overlay-portal.scss` lines 13-43) to a two-column flex row: timestamp left, status-label right. Drop the styles for `.agent-tool-overlay-status`, `.agent-tool-overlay-tool`, `.agent-tool-overlay-summary`, `.agent-tool-overlay-duration` — leave their classes in the file behind a one-line `// removed in tool-hover-consolidation 2026-05-28` breadcrumb or delete outright.

### 4.6 `hover-anchor.ts` — leave alone

`UserMessageBlock` still uses the hover-anchor mechanism (per existing memory note `feedback_agent_pane_tool_display` predecessor work). Do not touch `hover-anchor.ts` or `hover-anchor.test.ts`. The functions `pickExpandDirection`, `findScrollContainerRect`, `maxOverlayHeight`, and the `ExpandDirection` type remain in use by user-message hover. Only `ToolBlock`'s consumption of them goes away.

### 4.7 Tests — `ToolBlock.test.tsx`

Update / delete tests that assert hover behavior:

- The test described in the source comment "a completed (success/failed) tool + no pin, hover puts …" (line 86) needs to flip: now assert that hover does NOT change `expanded()`.
- Any test driving `mouseenter` / `mouseleave` to assert overlay visibility should be removed.
- Add a positive test: a running tool is auto-expanded; status flips to success; after `POST_COMPLETION_HOLD_MS + ε`, panel collapses; subsequent `mouseenter` does NOT re-expand.
- Add: clicking the collapsed row toggles `pinned` and the panel becomes visible.
- Add: when the panel is visible (auto-expand or pinned), the overlay header contains the timestamp text and the status label text but NOT the tool name string nor the summary string. Use `screen.queryByText(toolNameValue)` and assert it is `null` (or only matched once — the row, not the header).

## 5. Risk + reversibility

**Risk**: low. The change removes UI surface area, doesn't add. The auto-expand-while-running path (currently the dominant user-visible behavior for active tools) is preserved verbatim. Failure mode is "user wanted hover-to-peek and now has to click to pin" — a discoverability question, not a correctness question. The collapsed row remains clickable; the `cursor: pointer` style still signals interactivity.

**Reversibility**: high. The removed code is well-bounded (~50 LOC + helper imports) and lives in one component. If the user changes their mind, restoring the hover model is a single revert.

**Won't fix in this PR**:

- Click-to-pin discoverability (no visual cue for "this row is clickable to expand"). Note this in the PR body; address as a follow-up if users notice.
- The `cursor: pointer` style on the row is the only affordance. If we want a stronger cue, add an unobtrusive expand glyph (e.g. ▸ at right of row) — but that *adds* a fourth visual event, contradicting the spirit of "one popup."

## 6. Acceptance smoke

1. Open an agent pane with at least one completed tool (Bash, Read, or Edit).
2. Move the mouse over the tool row. **Nothing** should appear — no browser tooltip, no expand, no flicker. Hold there for 5 seconds. Still nothing.
3. Click the row. The unified panel appears in flow under the row. Header shows time on the left, status label on the right. No status icon, tool name, summary, or duration repeated in the header.
4. Click again. Panel collapses.
5. Send a message that triggers a new Bash tool. The panel auto-expands immediately on `running`, header timestamp matches the dispatch time. Tool completes; panel stays for 3 seconds; collapses. Hovering after that does nothing.

## 7. Out of scope

- `UserMessageBlock` hover-anchor behavior (unchanged).
- The "open in pane" / "open in window" / bookmark action bar at the bottom of the overlay (unchanged).
- The `ToolOverlayLog` body — log rendering rules, chunk vs structured-result switch, scroll-stick behavior all unchanged.
- The "AI Chat Panel" (`aitooluse.tsx`) referenced in `docs/specs/tool-collapse.md` §1 — this spec covers `ToolBlock` (agent view) only. If the AI Chat Panel has analogous hover behavior, file a follow-up.

## 8. Estimated diff

- `ToolBlock.tsx`: ~50 LOC removed, ~10 LOC modified (`expanded()`, `panelMode()`)
- `ToolBlockOverlay.tsx`: ~20 LOC removed, ~5 LOC added (timestamp helper + header)
- `ToolBlock.test.tsx`: ~30 LOC modified
- SCSS: ~40 LOC removed
- New helper `formatToolTime` (~6 LOC)

Net: roughly **-140 LOC, +40 LOC**. The diff is dominated by deletion — a healthy sign that the new behavior is simpler than the old.

## 9. Open question for the user (one)

The "small time popup" — is it the browser's native `title=` tooltip showing the summary text? That is this spec's working assumption. If it is something else (a custom tooltip component, a wallclock indicator I haven't found), the §4.2 removal will not address it. Confirm before implementing.
