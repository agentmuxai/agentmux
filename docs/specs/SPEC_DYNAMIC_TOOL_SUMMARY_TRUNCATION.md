# Dynamic ellipsis truncation for tool summaries

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-10
**Driving observation:** Bash command (and other tool) summaries in the agent pane are pre-truncated to a fixed character count + "…". Zooming out leaves a wide blank to the right of the ellipsis because the truncation point doesn't recompute. The ellipsis should "slide" based on available row width.

## Symptom

A `Bash` tool node currently renders its summary as:

```
🔨 Bash  npm install --package-lock-only --ignore-scripts >/dev/n…
```

Or similar, computed at node-creation time by a fixed-width truncation. When the user zooms the pane out (Ctrl+−), the agent pane gets wider in CSS pixels but the rendered summary still ends at the same hard-coded character count, leaving:

```
🔨 Bash  npm install --package-lock-only --ignore-scripts >/dev/n…    <-- empty space to here
```

Re-zooming or resizing the pane should make the ellipsis move further right (more text visible) or further left (less text visible) so the summary always fits the available width with no waste.

## Goal

The full text is stored complete in the node; only the rendered output truncates. Truncation point is **purely a CSS / layout concern** — recomputes for free on zoom and resize. No JavaScript measurement loop.

## Design

### Render contract

`ToolBlock`'s collapsed summary row keeps the full command text in the DOM:

```tsx
<div class="tool-summary">
    <span class="tool-summary-icon">{node.icon}</span>
    <span class="tool-summary-name">{node.tool}</span>
    <span class="tool-summary-args" title={fullText()}>
        {fullText()}
    </span>
</div>
```

The `<span class="tool-summary-args">` carries the full text. CSS truncates:

```scss
.tool-summary {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    width: 100%;
    overflow: hidden;
}

.tool-summary-args {
    flex: 1 1 0;            // take remaining width
    min-width: 0;           // override the default `auto` so flex can shrink it
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
```

Three pieces matter:
- **`flex: 1 1 0` + `min-width: 0`** — by default flex items can't shrink below their content size. Setting `min-width: 0` lets the args span shrink to whatever's left after icon + name take their share.
- **`white-space: nowrap`** — keeps the args on one line so `text-overflow: ellipsis` can apply.
- **`title={fullText()}`** — native browser tooltip surfaces the complete text on hover; replaces our pre-truncated summary's loss of information.

That's the entire change for the visible portion. On zoom in/out, browser re-flows; the ellipsis position recomputes naturally.

### Source of `fullText()`

For each tool kind, the existing `summary` field on the node is currently a pre-formatted string ("📖 Read auth.ts (0.3s) ✓"). We need the **un-truncated args** as a separate string the renderer can lay out.

Two options:
1. **Add a `summaryArgs` field to `ToolNode`.** Keep `summary` as the icon + name display; `summaryArgs` is the full text. Reducer fills it from `params.command` (Bash), `params.file_path` (Read/Edit/Write/Grep/Glob), etc.
2. **Compute on the fly in `ToolBlock`.** Pull from `node.params` based on `node.tool` discriminator. Per-kind formatter inline.

Recommendation: **option 2** — keeps the data model unchanged and the formatter local to the rendering layer. ~30 LOC switch + helpers.

### Per-kind formatter

```ts
function toolSummaryArgs(node: ToolNode): string {
    switch (node.tool) {
        case "Bash":   return (node.params as BashParams).command ?? "";
        case "Read":   return (node.params as ReadParams).file_path ?? "";
        case "Edit":
        case "Write":  return (node.params as { file_path?: string }).file_path ?? "";
        case "Grep":   return (node.params as GrepParams).pattern ?? "";
        case "Glob":   return (node.params as GlobParams).pattern ?? "";
        case "Task":
        case "Agent":  return (node.params as { description?: string }).description ?? "";
        case "Other":
        default:       return "";
    }
}
```

Lives in `frontend/app/view/agent/components/ToolBlock.tsx` next to the existing render code.

## Virtualization compatibility

**No conflict with virtualization** — in fact it's better aligned than the current pre-truncation:

- Collapsed tool rows have a **fixed height** (one line). The estimator (`estimateTool` returns 32) is exact regardless of the args text length, because the args span clips to one line via `nowrap + ellipsis`.
- `measureElement` (Phase 3 perf probe) sees the same one-line height every time — **estimator-miss rate stays at zero** for collapsed tool rows. Currently it's already low; this keeps it low.
- Width changes (zoom, pane resize, splitter drag) trigger CSS reflow on the args span. No virtualizer notification needed because the row's HEIGHT doesn't change. `ResizeObserver` won't fire.
- Pinned/expanded tool rows still work — the full args are always in the DOM, so expanding to show full output doesn't have to re-fetch or reformat the args.

The only caveat: if a future change adds a "wrap to N lines" mode (e.g., showing 2 lines of args when collapsed), the row height becomes width-dependent, and the virtualizer's per-kind estimator would need to know the row's current width. That's out of scope here; flag if it comes up.

## Edge cases

- **Very long single args** (e.g., a bash one-liner that's 1000 chars) — CSS handles cleanly. Ellipsis at clip boundary; tooltip shows full.
- **Chinese/CJK characters** — CSS `text-overflow: ellipsis` handles per-grapheme; no special work.
- **`title` tooltip on touch devices** — degraded: no hover. Acceptable for desktop-first app. Long-press shows it on touch where supported.
- **Multi-arg tools** (Task, Agent, Other with arbitrary `params`) — formatter returns a sensible default (e.g., `description`); fall back to `JSON.stringify(node.params).substring(0, 200)` if no recognized field. Tooltip surfaces the full structure.

## Out of scope

- **Multi-line wrapping for collapsed rows.** Possible future work; would need width-aware estimator.
- **Custom truncation styles** (middle-ellipsis, head-ellipsis). CSS only supports tail-ellipsis natively. JS-based middle truncation is a separate spec if requested.
- **Args formatting beyond a single string** — syntax-highlighted command, `path>` styling, etc. Different concern.

## Effort

| Component | LOC | Days |
|---|---|---|
| `toolSummaryArgs` helper + per-kind switch in ToolBlock | ~35 | 0.25 |
| SCSS rules (`.tool-summary` + `.tool-summary-args`) | ~15 | — |
| Update ToolBlock render to use full text + tooltip | ~10 | — |
| Visual verification (zoom in/out, pane resize) | — | 0.25 |
| **Total** | ~60 | **~0.5 day** |

## Cross-references

- Agent pane virtualization: `docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md` — confirms one-line collapsed rows have stable height which this design preserves.
- `frontend/app/view/agent/components/ToolBlock.tsx` — render site
- `frontend/app/view/agent/styles/_tool-block.scss` (or similar) — CSS additions
- `frontend/app/view/agent/types.ts` — `ToolNode` shape, `ToolParams` discriminated union

## Driving observation (verbatim)

> "is it possible the text of a bash command to be stored complete and painted to the screen with ellipse but resizes depending on the size of the screen or zoom? right now the ellipsis are fixed, if I zoom out there is a wide white space to the right. instead we want the ellipsis to dynamically move depending on the room available."
