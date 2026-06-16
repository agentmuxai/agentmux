# SPEC: Remove the per-row hover strip (`NodeHoverStrip`)

**Date:** 2026-06-15
**Status:** Draft — implementation plan
**Goal:** Remove the `NodeHoverStrip` / `.node-strip` — the strip that fades into the top-right of each conversation row on hover, showing a **timestamp** + an **expand/collapse** button.
**Analysis:** `docs/analysis/ANALYSIS_AGENT_ROW_HOVER_STRIP_2026_06_15.md`

> **Why:** it's the last remnant of the legacy "popups on hover" model the tool block already moved away from (`ToolBlock.tsx:18-21` — the deliberate consolidation away from "three popups on hover: browser title tooltip + larger log panel + fast expand/collapse"). Removing it makes rows quieter and removes a moving surface that floats over line content.

---

## 1. What the strip provides today (and what removal must handle)

The strip carries **two** things. Deleting it must consciously decide the fate of each:

| Carry | Today | On removal |
|-------|-------|-----------|
| **A. Timestamp** | localized time, only shown here, on hover/focus | **Lost entirely** unless relocated — see §3. |
| **B. Expand/collapse toggle** (⊞/⊟) | toggles the row for `TOGGLEABLE_KINDS` = `tool`, `shell`, `agent_message`, `section` | Must be covered by each kind's *own* affordance + keyboard — see §2. |

Everything else about the strip (CSS reveal, anchoring) is pure chrome and goes away with it.

## 2. Expand/collapse must survive (the load-bearing concern)

`DocumentRow` computes `canExpand()` / `isExpanded()` / `onExpand()` and uses them in **two** places:
1. The strip's button (being removed).
2. The **row keyboard handler** (`DocumentRow.tsx:107`): `if (canExpand()) { onExpand(); … }` on Enter/Space — the row is `tabindex=0`. **This stays** — keyboard expand is unaffected by the removal.

So `canExpand`/`isExpanded`/`onExpand` are **kept**; only the `<NodeHoverStrip/>` JSX render is deleted. The open question is the **mouse** affordance per toggleable kind:

| Kind | Own mouse toggle today? | Action |
|------|------------------------|--------|
| `tool` | ✅ click the tool summary header (`ToolBlock.tsx:237`, `onTogglePin`) | none — covered |
| `shell` | ✅ `PersistentShellBlock` `onTogglePin` (same header-click pattern) | verify in impl; covered |
| `agent_message` | ✅ `AgentMessageBlock` `onToggle` (header click) | verify in impl; covered |
| `section` | ❌ renders only `<h1/h2/h3>` — **no own toggle**; strip + keyboard were its only affordances | **DECISION (§4 Q2):** make the section header clickable to toggle (recommended), or accept keyboard-only |

**Net:** for tool/shell/agent_message the strip's expand button is already redundant. Only `section` needs a replacement mouse affordance — a one-line `onClick={onToggle}` on the section header is the clean fix (mirrors the tool header).

## 3. The timestamp — decide its fate (§4 Q1)

The per-line timestamp is shown *only* in the strip. Options:
- **(a) Drop it (recommended, matches the ask).** Simplest; rows get quieter. Per-line times are rarely consulted; session-level timing lives elsewhere (worked-row duration, activity log).
- **(b) Relocate** to a non-hover surface (e.g. a subtle right-aligned time on tool/message headers). More work; keep only if per-line time is actually valued.
- **(c) Keep on a different trigger** (e.g. a single `title=`/native tooltip on the row). Re-introduces a hover popup — defeats the goal; not recommended.

Default: **(a) drop**. The plan below assumes (a); (b) is an additive follow-up if wanted.

## 4. Decisions / open questions

1. **Timestamp fate** — drop (default), relocate, or keep-as-tooltip? (§3)
2. **Section mouse-toggle** — add a clickable section header (recommended) or accept keyboard-only collapse for sections? (§2)
3. **ActivityLogPanel** also renders a `NodeHoverStrip` (`ActivityLogPanel.tsx:90`) — remove there too (assumed yes; the log rows lose their hover time + expand, same tradeoff). Confirm.

## 5. File-by-file changes

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/NodeHoverStrip.tsx` | **Delete** the component file (and `formatLocalized`). |
| `frontend/app/view/agent/virtualization/DocumentRow.tsx` | Remove the `import` + the `<NodeHoverStrip …/>` render. **Keep** `canExpand`/`isExpanded`/`onExpand` (used by the keyboard handler). |
| `frontend/app/view/agent/components/ActivityLogPanel.tsx` | Remove the `import` + the `<NodeHoverStrip …/>` render (§4 Q3). |
| `frontend/app/view/agent/styles/_document.scss` | Delete the `.node-strip`, `.node-strip-time`, `.node-strip-btn` rules and the `&:hover .node-strip / &:focus-within .node-strip` reveal block. **Audit** whether `.agent-document-node-wrapper { position: relative }` is still needed by any other absolutely-positioned child (search highlight, pin marker); keep it only if so, else drop. |
| `frontend/app/view/agent/components/ToolBlock.tsx` *(only if §4 Q2 = section header)* | n/a — section lives in `DocumentRow`'s `DocumentNodeBody`; add `onClick={() => props.onToggleCollapse(node.id)}` + `cursor` to the `.agent-section` header there. |
| Tests | Grep for any test asserting `.node-strip` / `NodeHoverStrip` (e.g. DocumentRow/ActivityLog tests) and remove/adjust. |

## 6. Implementation steps

1. **Section affordance first (if Q2 = clickable header):** in `DocumentNodeBody`'s `section` branch (`DocumentRow.tsx`), wrap the `<h1/2/3>` in a clickable header that calls `onToggleCollapse(node.id)`; add `cursor: var(--cursor-interactive)`. This preserves mouse-collapse for sections before the strip goes.
2. **Delete the renders:** remove `<NodeHoverStrip/>` from `DocumentRow.tsx` and `ActivityLogPanel.tsx`; drop the imports.
3. **Delete the component:** remove `NodeHoverStrip.tsx`.
4. **CSS:** delete the `.node-strip*` rules from `_document.scss`; audit/keep `position: relative` on the wrapper.
5. **Timestamp:** (default) nothing — it's gone with the strip. (Option b) add the relocated time to headers.
6. **Tests + lint:** remove strip assertions; `npm run build`; `npx stylelint` the touched SCSS; run the agent doc/virtualization vitest suite.

## 7. Verification

- `npm run build` clean; `stylelint` clean on `_document.scss`.
- Tool/shell/agent_message rows still expand/collapse by **clicking their header** and by **Enter on the focused row**.
- Section rows still collapse (via the new header click if Q2=clickable, else via keyboard).
- No `.node-strip` element renders on hover; no console refs to `NodeHoverStrip`.
- `grep -rn "NodeHoverStrip\|node-strip"` returns nothing under `frontend/app/view/agent`.
- Manual: hover several rows — no floating strip appears; line content isn't overlaid.

## 8. Scope / non-goals

- Not touching the **tool overlay** (`_tool-overlay-portal.scss` / the expanded tool panel) — that's the separate, intentional expanded view; only the lightweight hover strip is removed.
- Not removing native `title=`/`aria-label` tooltips on individual buttons elsewhere.
- Effort: ~1 small PR (delete + one section-header click + CSS). Low risk given expand already has header + keyboard paths; the only real product call is the timestamp (§4 Q1).
