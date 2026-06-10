# SPEC: Responsive Aux Info + Color System for Agent Pane Tool Blocks

**Date:** 2026-06-09  
**Status:** Design / Pre-implementation  
**Scope:** `frontend/app/view/agent/`

---

## Problem

Tool blocks in the agent pane are one-size-fits-all. Every width gets the same collapsed row: icon + name + duration, nothing else until you click. On a 1200px-wide pane that's three wasted columns of empty space. On a 300px pane the current row is actually fine. The design needs to scale across the full width range.

Separately, all aux info is the same color. Params look like results look like metadata look like file paths. There's no visual grammar — users have to read everything to know what they're looking at.

---

## Container Query Context (already in place)

`agent-view.scss` already sets:

```scss
.agent-view {
  container-type: inline-size;
  container-name: agent-pane;
}
```

All breakpoints are `@container agent-pane (min-width: Xpx)` — no new infrastructure needed.

---

## Responsive Tiers

### Tier 1 — Narrow  `< 380px`

**Current behavior. No changes.**

- Collapsed row: `[icon] [tool name…] [duration]`
- Live-tail hidden (too cramped)
- Result hidden — two clicks to see (expand → read)
- Action bar: icon-only buttons, no labels

### Tier 2 — Default  `380px – 599px`

**Current behavior. This is the baseline the user described.**

- Collapsed row: `[icon] [tool name] [duration] [live-tail↳…]`
- Result hidden — click to expand, click again to pin
- Action bar: icon-only buttons

### Tier 3 — Medium  `600px – 899px`

**One-click tier.** Key result summary surfaces in the collapsed row. No click needed to see "5 files", "exit 0", "3 matches".

**Collapsed row:**
```
[icon] [tool name]  [result-pill]  [duration]
```

The **result pill** is a small inline tag rendered from the same `summarize()` logic already in `CompactResult`. It shows:
- Bash: `exit 0` (green) or `exit 1` (red)
- Glob: `12 files`
- Grep: `8 matches`
- Read: filename only (basename)
- Write: `written` or `N bytes`
- Edit: `+N / -N` line diff counts
- Agent: truncated first line of response

The full panel (log, JSON, diff) still requires one click to pin open.

**Implementation touch points:**
- `ToolBlock.tsx`: add `resultPill()` accessor, conditionally render via `@container` CSS class or JS width signal
- `_responsive.scss`: `@container agent-pane (min-width: 600px)` shows `.agent-tool-result-pill`, hides nothing currently shown

### Tier 4 — Wide  `900px – 1199px`

**Zero-click tier.** The panel opens inline automatically — always visible, no click, no timer. Params and result shown side-by-side in a two-column layout within the panel.

```
┌─────────────────────────────────────────────────────────┐
│ ⏳ Bash  chmod +x ./scripts/release.sh       0.3s  [⊞]  │
│ ┌──────────────────┬──────────────────────────────────┐ │
│ │ PARAMS           │ RESULT                           │ │
│ │ cmd: chmod +x …  │ exit 0                           │ │
│ └──────────────────┴──────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

**Panel layout change:** `display: grid; grid-template-columns: 1fr 1fr` for the log body.  
**Auto-expand:** Panel starts in `flow` mode (no click needed), but is still pinnable/collapsible.

**Implementation touch points:**
- `ToolBlock.tsx`: export `isWide` signal from container query observer (or CSS-driven via class on `.agent-view`)
- `ToolBlockOverlay.tsx`: at wide tier, wrap log + result in grid
- `expansion-source.ts`: add `"wide"` as a `via` value; treat it like `"auto"` but without the post-completion collapse

### Tier 5 — Ultra-wide  `≥ 1200px`

Same as Tier 4 but the panel gets more horizontal breathing room. Params column widens. Glob file list renders as a proper multi-column grid instead of a comma-separated string.

```
PARAMS                  │  RESULT
pattern: src/**/*.tsx   │  ┌──────────────┬──────────────┐
path: frontend/         │  │ app/foo.tsx  │ app/bar.tsx  │
                        │  │ app/baz.tsx  │ …12 more     │
                        │  └──────────────┴──────────────┘
```

---

## Color System

### Philosophy

Three semantic axes:
1. **What kind of info?** (input vs output vs metadata vs status)
2. **What tool produced it?** (file ops, search, shell, agent)
3. **What was the outcome?** (success, failure, neutral)

Colors are CSS custom properties on `:root` / `.agent-view`, dark-mode aware via `prefers-color-scheme`. All below are design targets; exact hex values TBD in implementation.

---

### Semantic Color Variables

```scss
// ─── Input / Params ──────────────────────────────────────────────────────────
// Warm amber — "what went in"
--tool-param-color:        #c9924a;   // param label text
--tool-param-value-color:  #e8b87a;   // param value text (slightly brighter)
--tool-param-bg:           rgba(201, 146, 74, 0.08);  // param block background

// ─── Output / Result ─────────────────────────────────────────────────────────
// Cool teal/cyan — "what came out"
--tool-result-color:       #4ab8c9;   // result label text
--tool-result-value-color: #7dd4e0;   // result value text
--tool-result-bg:          rgba(74, 184, 201, 0.08);  // result block background

// ─── File paths ──────────────────────────────────────────────────────────────
// Soft lavender — file system references
--tool-path-color:         #a78bfa;   // full path
--tool-path-basename-color:#c4b5fd;   // basename (brighter, the "key" part)
--tool-path-dir-color:     #7c6dc4;   // directory prefix (dimmer)

// ─── Shell / Code ────────────────────────────────────────────────────────────
// Muted cyan-green — inline commands, shell output
--tool-cmd-color:          #6ee7b7;   // command text (monospace)
--tool-stdout-color:       #d1fae5;   // stdout lines
--tool-stderr-color:       #fca5a5;   // stderr lines (warm red, not alarming)

// ─── Status ──────────────────────────────────────────────────────────────────
--tool-status-running-color:  #f59e0b;   // amber — ⏳
--tool-status-success-color:  #34d399;   // emerald — ✓
--tool-status-failed-color:   #f87171;   // red — ✗
--tool-status-denied-color:   #9ca3af;   // gray — ⊘
--tool-status-canceled-color: #6b7280;   // darker gray — ⏹
--tool-status-pending-color:  #fbbf24;   // yellow — ⚠

// ─── Metadata ────────────────────────────────────────────────────────────────
--tool-meta-color:         #6b7280;   // duration, byte counts, line counts
--tool-duration-color:     #9ca3af;   // slightly brighter than meta

// ─── Exit codes ──────────────────────────────────────────────────────────────
--tool-exit-ok-color:      #34d399;   // exit 0 — green
--tool-exit-err-color:     #f87171;   // exit N≠0 — red

// ─── Diff ────────────────────────────────────────────────────────────────────
--tool-diff-add-color:     #34d399;
--tool-diff-add-bg:        rgba(52, 211, 153, 0.10);
--tool-diff-del-color:     #f87171;
--tool-diff-del-bg:        rgba(248, 113, 113, 0.10);

// ─── Agent / Subagent ────────────────────────────────────────────────────────
--tool-agent-color:        #818cf8;   // indigo — agent identity
--tool-agent-output-color: #c7d2fe;   // agent response text

// ─── Match counts / Search ───────────────────────────────────────────────────
--tool-match-color:        #fb923c;   // orange — match count highlights
--tool-match-term-color:   #fdba74;   // matched term itself

// ─── Live-tail ───────────────────────────────────────────────────────────────
--tool-livetail-color:     #67e8f9;   // cyan — streaming progress
--tool-livetail-bg:        rgba(103, 232, 249, 0.06);
```

---

### Per-Tool Color Identity

Each tool gets a subtle left-border accent on its collapsed row to give instant visual identity before reading the name:

| Tool | Border Color | Rationale |
|------|-------------|-----------|
| `Bash` | `--tool-cmd-color` (#6ee7b7) | Shell — code green |
| `Read` | `--tool-path-color` (#a78bfa) | File access — lavender |
| `Write` | `--tool-diff-add-color` (#34d399) | Creating — green |
| `Edit` | `#facc15` (yellow) | Modifying — caution yellow |
| `Glob` | `--tool-path-basename-color` (#c4b5fd) | File search — lighter lavender |
| `Grep` | `--tool-match-color` (#fb923c) | Search — orange |
| `Agent` | `--tool-agent-color` (#818cf8) | Subagent — indigo |
| `Task` | `#60a5fa` (blue) | Task management — blue |
| `WebFetch` | `#38bdf8` (sky) | Network — sky blue |
| `WebSearch` | `#0ea5e9` (darker sky) | Search — slightly darker sky |

Applied via `data-tool` attribute (already on `.agent-tool-block` as of the latest main merge):

```scss
.agent-tool-block[data-tool="bash"]    { --tool-identity-color: var(--tool-cmd-color); }
.agent-tool-block[data-tool="read"]    { --tool-identity-color: var(--tool-path-color); }
.agent-tool-block[data-tool="write"]   { --tool-identity-color: var(--tool-diff-add-color); }
.agent-tool-block[data-tool="edit"]    { --tool-identity-color: #facc15; }
.agent-tool-block[data-tool="glob"]    { --tool-identity-color: var(--tool-path-basename-color); }
.agent-tool-block[data-tool="grep"]    { --tool-identity-color: var(--tool-match-color); }
.agent-tool-block[data-tool="agent"]   { --tool-identity-color: var(--tool-agent-color); }
.agent-tool-block[data-tool="task"]    { --tool-identity-color: #60a5fa; }
.agent-tool-block[data-tool="webfetch"]  { --tool-identity-color: #38bdf8; }
.agent-tool-block[data-tool="websearch"] { --tool-identity-color: #0ea5e9; }

// The identity color drives the left border — overrides status color while not running/failed
.agent-tool-block:not(.running):not(.failed):not(.pending-approval) {
  border-left-color: var(--tool-identity-color, var(--border-color));
}
```

---

### Result Pill Colors (Tier 3+)

The inline result pill in the collapsed row:

```scss
.agent-tool-result-pill {
  font-size: 10px;
  padding: 1px 5px;
  border-radius: 3px;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;

  &.exit-ok    { color: var(--tool-exit-ok-color);  background: rgba(52,211,153,0.12); }
  &.exit-err   { color: var(--tool-exit-err-color); background: rgba(248,113,113,0.12); }
  &.file-count { color: var(--tool-path-color);     background: rgba(167,139,250,0.12); }
  &.match-count{ color: var(--tool-match-color);    background: rgba(251,146,60,0.12); }
  &.written    { color: var(--tool-diff-add-color); background: rgba(52,211,153,0.12); }
  &.agent-out  { color: var(--tool-agent-color);    background: rgba(129,140,248,0.12); }
}
```

---

### Param/Result Labels in Panel

At Tier 4+ (wide), the two-column panel uses colored section headers:

```scss
.agent-tool-section-label {
  font-size: 9px;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  margin-bottom: 4px;

  &.params  { color: var(--tool-param-color); }
  &.result  { color: var(--tool-result-color); }
  &.output  { color: var(--tool-cmd-color); }
}
```

---

## Implementation Plan

### Phase 1 — Color system (no layout changes)

1. Add CSS variables to `:root` in `agent-view.scss` or a new `_tool-colors.scss`
2. Apply `data-tool`-driven border colors in `_document-nodes.scss`
3. Apply status colors to `.agent-tool-status-icon` (currently just text)
4. Color `.agent-tool-live-tail` with `--tool-livetail-color`
5. Color `.agent-tool-duration` with `--tool-duration-color`
6. Color param labels vs result labels in `ToolOverlayLog` output

**Risk:** Low. CSS-only, no JS changes. Visual regression possible if existing color vars conflict — audit `--warning-color`, `--success-color` usage.

### Phase 2 — Result pill (Tier 3, 600px+)

1. Add `resultPill()` logic to `ToolBlock.tsx` or a new `useToolResultPill` hook  
   - Calls same `summarize()` as `CompactResult`, but returns a typed `{ label, variant }` object
2. Render `<span class="agent-tool-result-pill {variant}">` in the summary row
3. In `_responsive.scss`: hide pill below 600px via `@container agent-pane (max-width: 599px) { .agent-tool-result-pill { display: none } }`
4. Hide live-tail at ≥600px when pill is present (avoid duplicate info)

**Risk:** Low-medium. The `summarize()` logic already exists; just needs extraction into a shared util.

### Phase 3 — Always-visible panel (Tier 4, 900px+)

1. In `expansion-source.ts`: add width-aware expansion — if pane is ≥900px, all completed tools default to `{ open: true, via: "wide" }`
2. In `ToolBlock.tsx`: consume width signal (from a `useContainerWidth()` hook reading `ResizeObserver` on `.agent-view`, or via a CSS custom property)
3. In `ToolBlockOverlay.tsx`: at wide tier, wrap log body + result in a grid
4. In `renderers.ts`: update `estimateTool()` to use `TOOL_EXPANDED_PX` when `isWide` — virtualization needs correct height estimates

**Risk:** Medium. Virtualization height estimates are critical — wrong estimates cause scroll jitter. The `ResizeObserver` on `.agent-view` needs debouncing. Auto-expand-all at wide tier could be jarring if the user switches from a wide to a narrow pane mid-session.

### Phase 4 — Multi-column Glob / file grids (Tier 5, 1200px+)

1. In `CompactResult.tsx` Glob case: at ≥1200px, render `result.files` as CSS grid (2-col or 3-col) instead of comma-joined string
2. Gate via container query class on parent

**Risk:** Low. Purely visual, no logic changes.

---

## Files to Change

| File | Phase | Change |
|------|-------|--------|
| `agent-view.scss` or new `_tool-colors.scss` | 1 | Add all CSS variable definitions |
| `_document-nodes.scss` | 1, 2, 3 | Tool identity border colors; pill show/hide; wide panel grid |
| `_responsive.scss` | 2, 3, 4 | New container query breakpoints: 600px, 900px, 1200px |
| `ToolBlock.tsx` | 2, 3 | Result pill render; width signal consumer |
| `ToolBlockOverlay.tsx` | 3 | Grid layout at wide tier |
| `expansion-source.ts` | 3 | `"wide"` expansion via |
| `renderers.ts` | 3 | Height estimates for wide-tier expanded tools |
| `CompactResult.tsx` | 4 | Multi-column Glob file grid |

---

## Open Questions

1. **Width signal source:** CSS container queries for pure-CSS hiding, but JS needs the width for `expansion-source.ts` logic. Options: (a) `ResizeObserver` on `.agent-view`, (b) CSS custom property `--pane-width` set by JS on mount/resize, (c) a SolidJS context providing pane width. Option (b) is lightest.

2. **Post-completion collapse at wide tier:** When the pane is wide and all tools auto-expand, should the 3s post-completion timer still fire? Probably not — at wide tier, always-visible is the contract. The `via: "wide"` expansion source would bypass the hold timer.

3. **User preference override:** Should users be able to force "always collapsed" even at wide widths? Could be a `settings.json` key `agent:tool-expansion-tier` (auto / compact / always-expanded). Defer to Phase 3+.

4. **Transition when resizing:** When the user drags a pane from 950px to 550px mid-session, tools that were auto-expanded (via `"wide"`) should gracefully collapse without a flash. Requires the expansion source to react to width changes, not just compute at mount.
