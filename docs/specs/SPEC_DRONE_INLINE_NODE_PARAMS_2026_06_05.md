# SPEC — Drone: Inline In-Node Parameter Editing

- **Date:** 2026-06-05
- **Author:** AgentX
- **Status:** Draft / proposed
- **Widget:** `defwidget@drone` → view `drone`
- **Scope:** Frontend (SolidJS) — replace the right-side inspector overlay with compact in-node editing. No backend changes.
- **Builds on:** `SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md` (merged: #1289 + #1290).

---

## 0. TL;DR

The right-side inspector slide-over (`InspectorPanel`, `drone-view.tsx:609`) covers too much canvas. **Remove it.** Move parameter editing **into the node** as compact inline widgets, using the leading-editor pattern (ComfyUI / LiteGraph / Blender / React-Flow custom nodes): real DOM inputs in the node body, `nodrag`/`nowheel` so editing never moves the canvas, and **progressive disclosure** — a one-line summary when the node is unselected, the editable field stack when selected.

Three decisions carry the design:

1. **Two-state node.** Unselected → existing `nodeSummary()` one-liner. Selected → inline editable fields (the current `InspectorForm` body, restyled to fit a node).
2. **Complex fields don't bloat the node.** Long text (agent task, API body, response template), the variables list, and the identity/memory pickers open in a **field-anchored popover** (floats over the canvas, closes on blur/Esc) — never a persistent panel. Optional power move: **double-click a node → zoom-to-node "edit mode"**.
3. **Ports stay anchored to the header row** so a node growing taller never breaks edge attachment — no per-node re-measure needed for v1.

Net result: the canvas is 100% canvas again; you edit where you look.

---

## 1. Problem

`InspectorPanel` renders a 320px `<aside class="drone-inspector">` overlay pinned to the right (`drone-view.scss` `.drone-inspector { position:absolute; right:0; width:320px; ... }`). On a typical pane it covers a large fraction of the working area, and it spatially decouples the controls from the node being edited — the opposite of the "expansive canvas" the drone redesign set out to deliver.

## 2. Current state (what moves)

- **`InspectorPanel`** (`drone-view.tsx:609-636`) — the `<Show keyed>` overlay. **Delete** this and its `.drone-inspector*` styles.
- **`InspectorForm`** (`drone-view.tsx:638-725`) — the per-kind field stack. **Relocate** its body into the node; keep the per-kind `<Show>` branches and the `update()` helper.
- **Field editors** reused as-is (restyled compact): `Field` (`:727`), `VariablesEditor` (`:734`), `AgentRefEditor` (`:813`), `AgentResultPanel` (`:872`).
- **Node rendering** (`drone-view.tsx:456-510`): node `<div class="drone-node">` with header (emoji + label + ×), `<div class="drone-node-body">{nodeSummary(n)}</div>`, and the input/output ports. The body becomes state-driven (summary vs fields).
- **Ports** computed by `portFlowPos` / `portOffsetY` (`drone-view.tsx`) from a fixed top offset (`26 + …`). This already anchors ports near the header — we keep that (see §6).
- **Selection** lives in `DroneViewModel._selected` (`selectedAtom`/`setSelected`). Drives expansion.

Per-kind config (from `InspectorForm`): **Agent** = identity/memory/instance pickers + `task` textarea + last-run result; **API** = method select + url + body; **Condition** = expr; **Response** = template; **Variables** = name/value rows.

## 3. Design principles (from research)

- **Inline real DOM controls**, not canvas-painted — free keyboard/focus/AT support (ComfyUI uses real `<textarea>`; LiteGraph's canvas widgets are inaccessible — avoid that).
- **Progressive disclosure** — show decision-critical info at rest; reveal editing on demand. Common fields first; advanced behind an expander.
- **`nodrag` / `nowheel` / `nopan` discipline** — any interactive element must not drag the node, scroll-zoom the canvas, or pan it.
- **No persistent side panel.** Room for big fields comes from on-demand **popovers** and an optional **zoom-to-node** mode.
- **Semantic zoom / level-of-detail** — render heavy inputs only when selected and zoom ≥ threshold; cheap placeholder when zoomed out (perf + clarity).
- **Header-anchored ports** keep edges correct as nodes grow (pragmatic 80/20 vs measure-and-recompute).

## 4. Target UX

### 4.1 Node states

```
 Unselected (compact)                Selected (inline edit)
 ┌────────────────────┐              ┌────────────────────────┐
●│ 🤖 Agent         ✕ │●           ●│ 🤖 Agent             ✕ │●
 │ researcher · "summa…│             ├────────────────────────┤
 └────────────────────┘             │ Identity [researcher ▾]│
                                     │ Task  "Summarize th…" ✎│  ← click ✎ → popover
                                     │ ▸ Advanced             │
                                     │ ⤢ last run · $0.0021   │  ← compact result
                                     └────────────────────────┘
```

- **Unselected:** current `nodeSummary(n)` one-liner. Unchanged behavior; cheap.
- **Selected:** inline field stack. Fixed node width (**~248px**); height grows with content. Selecting a node expands it; clicking empty canvas / selecting another collapses it. (Selection already toggles via `setSelected`.)
- **Pin-expanded (optional, Phase 2):** a header pin so a node stays expanded while unselected. Not required for v1.
- **Zoom LOD:** below ~0.5 zoom, render only the title chip (no inputs), regardless of selection.

### 4.2 Per-kind inline layout

Fixed-width rows, label-left / control-right, ~28px tall. `Field` is reused; restyle `.drone-field` to a horizontal compact row inside nodes.

| Kind | Inline (always when selected) | Behind "▸ Advanced" | Popover (click to open) |
|---|---|---|---|
| **Agent** 🤖 | Identity `<select>`; Task summary line + ✎ | Memory `<select>`, Instance name | **Task** prompt editor (big textarea) |
| **API** 🌐 | Method `<select>`; URL `<input>` | Headers (kv) | **Body** editor (JSON textarea) |
| **Condition** 🔀 | Expr `<input>` | — | — (expr is short; inline is enough) |
| **Response** 🏁 | Template summary line + ✎ | — | **Template** editor (big textarea) |
| **Variables** 🔢 | First 2 rows inline + "＋ n more" | — | **Variables** list editor (full `VariablesEditor`) |

Rationale: short fields (method, url, identity, expr) edit fine inline; long free-text and lists escalate to a popover so the node stays compact.

### 4.3 Complex-field escalation (the inspector replacement)

**Field-anchored popover** is the primary mechanism. Clicking a ✎ affordance (or the summary line) opens a popover anchored to that field:

- Floats **above** the canvas (`position: fixed`, anchored via the field's screen rect), generously sized (e.g. 360×220 for a prompt).
- Contains the full editor for one field (the existing textarea / `VariablesEditor` / picker), `nodrag`+`nowheel`.
- **Commits live** to `updateNodeData` on input (same as today); **Esc** closes (value already committed), click-outside closes.
- Only one popover open at a time.

**Optional — zoom-to-node edit mode (Phase 2):** double-click a node → `fitView` to that single node and expand every field full-size in place; Esc returns. Gives panel-level room without a panel. Nice for the Agent block.

### 4.4 Inline run state

`AgentResultPanel` becomes a compact, collapsible strip at the bottom of the Agent node (status dot + cost + truncated response; click to open in the same popover). During a run, the existing per-node status classes still drive the header glow.

## 5. Interaction spec

- **nodrag:** every inline `<input>/<select>/<textarea>/<button>` carries `nodrag` (the node-drag handler already early-returns on `.closest(".nodrag")`, `drone-view.tsx` `onNodePointerDown`). Verify selects/inputs are covered.
- **nowheel:** any scrollable inline area (capped textarea, result strip) needs `onWheel={(e)=>e.stopPropagation()}` so the wheel scrolls content instead of zooming the canvas (the canvas `onWheel` zooms).
- **Commit:** on `input`/`change` → `updateNodeData` (live, as today). **Enter** in single-line commits + blurs; **Esc** closes popover / blurs field.
- **Selecting vs editing:** pressing a field selects the node (already true via capture-phase focus) and must NOT start a node drag (nodrag) or a connection.
- **Tab order:** fields in DOM order; ensure canvas key handlers don't swallow Tab while a field is focused (the keydown handler ignores `INPUT/TEXTAREA/SELECT` targets — keep that).
- **Ports unaffected:** port pointerdown still starts a connection; fields sit between the ports, which remain on the node edges.

## 6. Node sizing + port alignment

- **Width fixed** (~248px); **height auto** (grows with the selected field stack).
- **Ports anchored to the header row.** Keep `portOffsetY` referencing the header band (top ~26px) so inputs/outputs and their bezier endpoints stay put as the body grows downward. Edges never need re-measuring. (Condition's two outputs remain stacked near the header.)
- **Future option (not v1):** per-port DOM anchors + a measure-on-change pass (React Flow's `useUpdateNodeInternals` analog) if we later want ports aligned to specific field rows. Explicitly out of scope here.

## 7. Performance

- Render the **full field stack only when `selected()` and zoom ≥ ~0.5**; otherwise render the summary/title only. Caps live DOM inputs to ~one node at a time.
- Solid's fine-grained reactivity already prevents sibling re-renders; keep field state reads scoped to the node (the store proxy per node).
- Keep node CSS cheap (avoid heavy shadows/animations on the expanded node).

## 8. Accessibility

- Real `<label>` (reuse `Field`) per control; no placeholder-as-label.
- "▸ Advanced" is a `<button aria-expanded>`; the ✎ popover trigger has an `aria-label` ("Edit task"); popover is focus-trapped and Esc-dismissible, returning focus to the trigger.
- Tab/Shift-Tab traverse fields; canvas shortcuts don't fire while a field is focused.

## 9. Component / file changes

- **Remove:** `InspectorPanel` (`drone-view.tsx:609`), its render in `DroneView` (`:26`), and `.drone-inspector*` SCSS.
- **Add:** `NodeBody` (or inline in the node `<For>`) that renders `nodeSummary` when `!selected`/zoomed-out, else `NodeFields`.
- **Add:** `NodeFields(model, node)` — the relocated per-kind `<Show>` stack from `InspectorForm`, restyled compact; reuses `Field`, `VariablesEditor`, `AgentRefEditor`, `AgentResultPanel`.
- **Add:** `FieldPopover` — a fixed-position editor anchored to a field rect; holds the big textarea / list / picker; `nodrag`+`nowheel`; Esc/click-outside to close.
- **Model (`drone-model.ts`):** no new persistent state required for v1 (selection drives expansion). Optional Phase-2 `pinnedExpanded`/`collapsed` per-node flag if pinning is added → store in `node.data` so it persists.
- **SCSS:** new `.drone-node--expanded`, `.drone-node-fields`, compact `.drone-field` (horizontal), `.drone-field-edit` (✎ button), `.drone-popover`, `.drone-node-result` (compact). Drop `.drone-inspector*`.
- **Keep:** ports, edges, pan/zoom, wiring — untouched.

## 10. Phased plan

| PR | Scope |
|---|---|
| **1** | Remove `InspectorPanel`; render `NodeFields` inline for the selected node (all 5 kinds, simple inline controls); short fields inline, long fields as a **capped auto-grow textarea** placeholder. Header-anchored ports already work. |
| **2** | `FieldPopover` for Task / Body / Template / Variables / pickers; ✎ affordances + summary-click; `nowheel`. Compact inline `AgentResultPanel`. |
| **3** (opt) | Zoom-to-node edit mode (double-click); pin-expanded; "▸ Advanced" grouping; zoom LOD placeholder. |

**Minimum to kill the side panel: PR 1.** PR 2 makes the heavy fields pleasant.

## 11. Risks / open questions

- **Agent node density** — even compact, identity + task + result is a lot. Mitigation: identity inline, task→popover, memory/instance under Advanced, result as a one-line strip. Validate live.
- **Popover + canvas transform** — popover is screen-positioned (not inside the zoomed viewport), so it must read the field's `getBoundingClientRect()` and reposition on scroll/pan/zoom (or close on viewport change). Closing on pan/zoom is acceptable for v1.
- **Header-anchored ports on tall nodes** — many fields make a node much taller than its single header-row of ports; visually fine (n8n-style) but confirm it reads clearly with 2 condition branches.
- **Click-to-select vs click-to-edit** — first click selects+expands; the now-visible field takes a second click to focus. Acceptable; revisit if it feels slow.

## 12. Appendix — concrete shape (research-backed)

Fixed-width node (~248px), height grows. Unselected = title chip + summary line. Selected = label-left/field-right rows (~28–32px), real DOM controls, all `nodrag`; scrollable areas `nowheel`. Common fields inline; advanced behind "▸ Advanced"; long text / lists / pickers in a **field-anchored popover** (or double-click → zoom-to-node). Render full inputs only when selected + zoom ≥ 0.5. Ports stay on the header row. No persistent side panel anywhere.

**References:** ComfyUI/LiteGraph inline widgets + multiline textarea; Blender geometry-node inline fields; Rete.js controls (`NoDrag`, attach-to-input); React Flow custom nodes + utility classes (`nodrag`/`nopan`/`nowheel`) + `useUpdateNodeInternals` (the measure-on-change analog we defer); n8n/cables.gl as the *panel* counter-examples (justified only for 10+ heterogeneous params / embedded code editors — not drone's blocks).
