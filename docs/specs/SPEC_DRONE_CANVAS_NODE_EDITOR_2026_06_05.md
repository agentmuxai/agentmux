# SPEC — Drone Canvas: Expansive Node-Graph Editor

- **Date:** 2026-06-05
- **Author:** AgentX
- **Status:** Draft / proposed
- **Widget:** `defwidget@drone` → view `drone` (pinned)
- **Scope:** Frontend (SolidJS) canvas rewrite + small backend touch-ups. Backend execution engine is **out of scope** (Phase-1 complete).
- **Related:** `SPEC_DRONE_INLINE_NODE_PARAMS_2026_06_05.md`, `SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md`
- **Tracking:** GitHub **issue #753** (RFC: Workflows pane — Sim-modeled DAG executor) and **discussion #832** (Workflows pane — long-term tracking thread). ⚠️ **Both have drifted from the shipped code — see §0.1.**

### 0.1 Relationship to the existing RFC (#753) — this spec supersedes two stale decisions

The feature is tracked by **issue #753** + **discussion #832**, but the RFC predates the code as shipped and is wrong on two points this spec corrects:

1. **Naming.** The feature shipped as **Drone**: the widget is `defwidget@drone` and the code lives in `frontend/app/view/drone/`. ("Workflows" is unrelated — that term now refers strictly to Claude's own workflows feature.) This spec uses the shipped names.
2. **Canvas library.** #753's headline decision (§2 Q1) was "drop the ~750-LOC custom canvas, adopt `solid-flow` (miguelsalesvieira/solid-flow)." The team **did not** do this — a custom SVG canvas shipped instead. Independent research (2026-06-05) confirms `miguelsalesvieira/solid-flow` is **abandoned since 2022** (v1.0.4, Oct 2022; different non-xyflow API), and the alternative `@dschz/solid-flow` is a single-maintainer v0.1.x alpha. So the team's instinct to keep a custom canvas was correct. §3 here formalizes that: **build custom, borrow `@xyflow/system` for the d3/geometry math + `@dagrejs/dagre` for layout.**

Everything else in #753 (5-block taxonomy, Mustache `{{...}}` interpolation, Agent-block-references-identity, SQLite run history, validator rules) **matches the shipped backend** and stands. Recommend posting this spec under discussion #832 and adding a correcting note to #753 rather than opening a new tracking issue.

---

## 0. TL;DR

The drone pane today is a 3-column form (left palette · cramped SVG canvas · right inspector) where **you cannot drag nodes, pan, zoom, or drag from the palette** — so it reads as broken. The backend DAG engine behind it is already complete and solid.

This spec rebuilds the *frontend canvas* into the requested experience:

> **A horizontal node-type bar at the top (emoji + name chips), and the entire rest of the pane is one expansive DAG canvas.** You drag node-types down from the bar onto the canvas, wire nodes together by dragging between ports, pan/zoom freely, and the inspector + run state appear as overlays so they never eat canvas space.

**Architecture decision:** **build out the existing custom SolidJS SVG/HTML canvas** rather than adopt a library. There is *no* production-grade SolidJS flow library (React Flow is React-only; the one Solid port is a bus-factor-1 alpha). We borrow only the framework-agnostic hard-math pieces: **`@xyflow/system`** (pan/zoom via d3, edge-path geometry, connection hit-testing) and **`@dagrejs/dagre`** (auto-layout). Everything else is Solid code we own — which suits the team's "control + minimal deps" bias and reuses the model/backend that are *already* xyflow-shaped.

---

## 1. Current state (ground truth)

### 1.1 Frontend — `frontend/app/view/drone/`

| File | Role | State |
|---|---|---|
| `drone.tsx` | Barrel; assigns `DroneView` to `DroneViewModel.prototype.viewComponent` | OK |
| `drone-view.tsx` | The whole UI: `Toolbar`, `BlockPalette`, `Canvas`, `InspectorPanel`, `RunPanel` | **The rewrite target** |
| `drone-model.ts` | `DroneViewModel` — draft graph state + RPC + run-state subscription | Mostly reusable; needs store migration + new mutations |
| `block-registry.ts` | Per-kind metadata (label, color, icon, defaultData, handles) | Reusable; add `emoji` |
| `drone-types.ts` | `FlowNode`, `FlowEdge`, `DroneGraph`, `DroneDefinition`, `DroneViewport` | Reusable (already xyflow-shaped) |
| `drone-view.scss` | Styles | Largely rewritten |
| `store/drone-run-state/*` | Reducer slice #10 — live run state (status, blockResults) | Reusable as-is |

**Layout today** (`drone-view.tsx:16-29`): `Toolbar` (header) → `drone-body` = **`BlockPalette` (left aside) · `Canvas` (center) · `InspectorPanel` (right aside)** → `RunPanel` (footer). Three columns + two horizontal bars = the canvas is the *smallest* region, the opposite of the vision.

**What the canvas can do today** (`drone-view.tsx:130-242`):
- ✅ Add node — **click** a palette item → node spawns at `{80 + rand·60, 80 + rand·60}` (`drone-view.tsx:90`). Nodes stack near the origin.
- ✅ Select node — click → `setSelected`; inspector edits work.
- ✅ Remove node — `×` on node header.
- ✅ Wire — **shift-click source then shift-click target** (`drone-view.tsx:136-174`); Condition branch handle auto-assigned by order (1st edge `true`, 2nd `false`).
- ✅ Edges rendered as straight `<line>` with hard-coded offsets (`x1 = src.x+80, y1 = src.y+28`, `drone-view.tsx:190-196`).
- ❌ **No node dragging** — `moveNode()` exists (`drone-model.ts:447`) but **no pointer handlers are wired**. This is the #1 "nothing works" symptom.
- ❌ **No pan / zoom** — `viewport` is stored but never applied to a transform.
- ❌ **No drag-from-palette** — explicitly deferred ("Phase 2 will use drag + drop", `drone-view.tsx:87`).
- ❌ **No port handles** — wiring is whole-node→whole-node; no visible ports, no typed validation, no drag-to-connect.
- ❌ **No edge delete / select**, no multi-select, no undo/redo, no fit-view, no minimap, no auto-layout.
- ❌ **No emoji** — `icon` is an unused FontAwesome class string; palette shows a colored dot only.
- ⚠️ `AgentRefEditor` references types `IdentityBundle` / `Memory` that may be missing from `gotypes` (`drone-view.tsx:467-470`) — verify during the rewrite.

**State shape** (`drone-model.ts:74`): the entire draft is one `createSignal<DroneDefinition>`, mutated by **spread-copying the whole graph** on every change (`addNode`/`moveNode`/etc., `drone-model.ts:409-472`). Under Solid this means *every* node-position change replaces the array and re-runs the whole nodes `<For>` — fine at 5 nodes, janky at 50, and fatal for smooth dragging. **Migrate to `createStore` with path mutations** (§9).

### 1.2 Backend — `agentmux-srv/src/drone/` (complete, do not rewrite)

- **Model:** `DroneDefinition { id, name, description, graph: {nodes: FlowNode[], edges: FlowEdge[]}, viewport: {x,y,zoom}, created_at, updated_at }`. `FlowNode { id, position:{x,y}, data: JSON, node_type }`. `FlowEdge { id, source, target, sourceHandle?, targetHandle? }` — **camelCase handles on the wire** (`types.rs`, `#[serde(rename_all="camelCase")]`). This is the xyflow shape verbatim.
- **Engine:** Kahn topological layering, sequential per-layer execution, `{{block_id.field}}` / `{{var.name}}` / `{{env.AGENTMUX_DR_*}}` interpolation, Condition branch-pruning gated on edge `sourceHandle ∈ {"true","false"}`. SSRF-guarded API block.
- **RPC (WSH, no REST):** `listdrones() · getdrone(id) · upsertdrone(DroneDefinition) → normalized · deletedrone(id) · rundrone(drone_id) → {run_id} · listdroneruns(drone_id, limit?)`. **There is no per-node / per-edge / per-viewport delta API by design — the frontend owns the graph and upserts the whole `DroneDefinition`.**
- **Live run state:** subscribe to broker event `dronerun:<run_id>`; events `run_started · block_started · block_done · block_error · run_done · run_failed` (snake_case `kind` tag). Already wired through slice #10.

**Node taxonomy (canonical — the top bar's contents):** `agent`, `condition`, `api`, `response`, `variables`. Phase-2 engine adds `function`, `loop`, `parallel`, `router`, `subdrone` (reserve UI slots, don't build).

---

## 2. Vision & target UX

```
┌─────────────────────────────────────────────────────────────────────────┐
│  [Untitled Drone ▾]            🔢 Variables  🤖 Agent  🌐 API  🔀 Condition │  ← node-type bar
│                                🏁 Response          🔍search   ⟳ Tidy  ▶ Run │     (drag chips down)
├─────────────────────────────────────────────────────────────────────────┤
│ · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · │
│ · · · · ┌──────────┐ · · · · · · · · · · · · · · · · · · · · · · · · · · · │
│ · · · · │🤖 Agent  ●─────────●┌──────────┐· · · · · · · · · · · · · · · · │
│ · · · · │ researcher│         │🔀 Condition│ ──true──●┌──────────┐· · · · · │
│ · · · · └──────────┘         │ {{x}} > 0  │          │🏁 Response│· · · · · │
│ · · · · · · · · · · · · · · · └──────────┘ ──false─● └──────────┘· · · · · │
│ · · · · · · · · · · · · ·  (infinite, pannable, zoomable canvas) · · · · · │
│ · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · ┌────┐│
│ · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · · │mini││
│  [�――●――]  ⊕ ⊖ ⤢                                          (overlays) └────┘│
└─────────────────────────────────────────────────────────────────────────┘
        ▲ controls (bottom-left)              ▲ inspector slides in from right when a node is selected
```

**Principles**

1. **Canvas is the product.** Top bar is the only permanent chrome. Inspector and run-log are **overlays** (slide-over / collapsible drawer) that float above the canvas, never columns that shrink it.
2. **Node types live at the top, horizontal, emoji + name.** They are **draggable chips** (drag onto canvas to create) *and* clickable (click drops at viewport center) for accessibility.
3. **Direct manipulation.** Drag nodes; drag from a port to another port to wire; box-select; delete; pan with space/middle-drag; zoom with wheel; "Tidy" auto-lays-out.
4. **Live execution is on the canvas.** During a run, nodes glow by status (pending/running/done/error/skipped); the run panel is a thin overlay, not a footer.

---

## 3. Architecture decision

### 3.1 Decision: build the custom Solid canvas; borrow only hard-math libs

| Concern | Decision | Why |
|---|---|---|
| Flow framework | **None.** Build custom. | No production-grade SolidJS flow lib exists. React Flow is React-only. `@dschz/solid-flow` is v0.1.x, 44★, single maintainer, pinned to an old `@xyflow/system`. Adopting it = inheriting an alpha + a rewrite of working code. |
| Pan / zoom | **Borrow `@xyflow/system`** (`XYPanZoom`, d3-zoom under the hood) | Hardest thing to hand-roll correctly (transform origin, wheel/pinch, bounds). Proven; Solid-consumable (the alpha port proves the wiring). |
| Edge geometry | **Borrow `@xyflow/system`** (`getBezierPath`, `getSmoothStepPath`) | ~50 lines of bezier we'd otherwise own and get subtly wrong. |
| Connection hit-test / validation | **Borrow `@xyflow/system`** (`XYHandle`) *or hand-roll* | Optional; can hand-roll handle hit-testing if we want zero coupling. |
| Coord transforms (screen↔flow) | **Borrow** `pointToRendererPoint` / `rendererPointToPoint` | The math behind `screenToFlowPosition`. Needed for drag-from-palette drops. |
| Auto-layout ("Tidy") | **Borrow `@dagrejs/dagre`** (rank dir `LR`) | Framework-agnostic; feed nodes/edges, get x/y, write back to the store. Add `elkjs` only if we later need ports/orthogonal routing. |
| Node render, ports, drag, selection, inspector, run overlay | **Hand-roll in Solid** | Our domain UI; where Solid's fine-grained reactivity shines. |

**Net new deps:** `@xyflow/system` (pin exact version — it's `0.0.x`, internally scoped, treat minor bumps as breaking) and `@dagrejs/dagre`. *Escape hatch:* if `@xyflow/system`'s instability bites, depend on raw `d3-zoom` and copy the 2–3 path/transform functions — they're small and stable.

### 3.2 Rejected alternatives

- **Adopt `@dschz/solid-flow`** — alpha, bus-factor 1, pinned-old core, forces a component-model rewrite. Keep it as a *reference implementation* only.
- **Adopt React Flow** — impossible; the app is SolidJS (`solid-js ^1.9.11`), no React anywhere.
- **Keep pure hand-rolled (no `@xyflow/system`)** — viable, but re-deriving d3-zoom semantics and bezier math is avoidable risk; borrow the math, own the UI.
- **Swap the whole frontend to React for React Flow** — absurd cost; rejected.

---

## 4. Node taxonomy & emoji design (the top bar)

Add an `emoji` field to `BlockKindMeta` (`block-registry.ts`). Keep `color` (used for the header strip + handle accents) and `icon` (drop the unused FA string or repurpose for the overflow menu).

| Kind | Emoji | Label | Color | Inputs | Outputs | One-liner |
|---|---|---|---|---|---|---|
| `variables` | 🔢 | Variables | `#a855f7` | — | `out` | Declare drone-scope `{{var.*}}` |
| `agent` | 🤖 | Agent | `#3b82f6` | `in` | `out` | Run an agent with a task prompt |
| `api` | 🌐 | API | `#10b981` | `in` | `out` | HTTP request (`{{...}}` in url/headers/body) |
| `condition` | 🔀 | Condition | `#eab308` | `in` | `true`,`false` | Branch on a boolean expression |
| `response` | 🏁 | Response | `#ef4444` | `in` | — | Terminal output (exactly one) |

**Reserved (Phase-2 engine; show greyed/"coming soon" or omit):** `function` 🧩 · `loop` 🔁 · `parallel` 🪢 · `router` 🧭 · `subdrone` 🛸.

Emoji rendering: plain Unicode in a `<span class="chip-emoji">` — no icon font, no asset pipeline, renders identically in the CEF webview. Provide an `aria-label` per chip (e.g. `aria-label="Agent node — drag onto canvas"`).

---

## 5. Target layout & component tree

```
<DronePane>                         // root; CSS grid: [topbar] / [canvas]   (full-bleed)
 ├─ <NodeTypeBar model>             // the horizontal emoji-chip palette (replaces BlockPalette aside)
 │   ├─ <DroneTitle/>               // name field + New/Save/Open menu (left)
 │   ├─ <NodeChip kind> × N         // draggable + clickable emoji chips (center, horizontal, wraps)
 │   ├─ <NodeSearch/>               // fuzzy filter when catalog grows (Cmd-K style, optional Phase 2)
 │   └─ <CanvasActions/>            // Tidy (dagre) · Fit · Run  (right)
 └─ <Canvas model>                  // fills 100% of the area under the bar
     ├─ <svg class="dr-viewport">   // single <g transform> = pan/zoom; ALL graph content inside
     │   ├─ <Background/>           // dot grid (re-rendered cheaply on zoom)
     │   ├─ <EdgeLayer>             // <For edges> → <EdgePath> (bezier/smoothstep)
     │   ├─ <ConnectionPreview/>    // live wire while dragging from a port
     │   └─ <NodeLayer>             // <For nodes> → <DroneNode> (HTML in <foreignObject> OR overlaid div layer)
     ├─ <Controls/>                 // zoom +/- , fit, lock  (bottom-left overlay)
     ├─ <MiniMap/>                  // bottom-right overlay (Phase 2)
     ├─ <InspectorDrawer model>     // slide-over from right; shown when selectedNode != null
     └─ <RunOverlay model>          // thin collapsible run-log; live status; bottom overlay
```

**Rendering choice — HTML node layer over SVG edge layer (recommended).** Render edges in one `<svg>` and nodes as absolutely-positioned `<div>`s in a sibling layer that shares the *same* pan/zoom transform. Reason: rich node bodies (inputs, selects, textareas, the AgentRef pickers) are far easier and more accessible as HTML than `<foreignObject>`, and HTML inputs inside `<foreignObject>` have known focus/caret quirks in Chromium. Both layers are wrapped by one transformed container so they stay aligned. (This is exactly how React Flow renders.)

---

## 6. Interaction specifications

### 6.1 Node-type bar (top)

- Horizontal flex row, wraps to a second line on narrow widths; chips are `draggable`.
- **Chip = `<button draggable>` with `🤖 Agent`**. `onDragStart` sets the dragged kind into a small module-level signal (don't rely on `dataTransfer.getData` during `dragover` — it's write-only there) **and** `e.dataTransfer.setData("application/x-drone-kind", kind)` as a fallback.
- **Click** (no drag) → `addNode(kind, centerOfViewport())` so keyboard/non-drag users can still build.
- Accessibility: `role="button"`, `tabindex=0`, Enter/Space = click-add, `aria-label` includes "drag onto canvas".
- Search (Phase 2): filter chips by fuzzy match; later a `Cmd-K` command palette and "double-click empty canvas → searchable add menu".

### 6.2 Drag-from-bar → drop-on-canvas

Canonical recipe adapted to Solid:

```tsx
// module scope
const [dragKind, setDragKind] = createSignal<BlockKind | null>(null);

// NodeChip
<button draggable
  onDragStart={(e) => { setDragKind(kind);
    e.dataTransfer!.effectAllowed = "copy";
    e.dataTransfer!.setData("application/x-drone-kind", kind); }}
  onDragEnd={() => setDragKind(null)}>
  <span class="chip-emoji">{meta.emoji}</span><span>{meta.label}</span>
</button>

// Canvas
const onDragOver = (e: DragEvent) => { e.preventDefault(); e.dataTransfer!.dropEffect = "copy"; };
const onDrop = (e: DragEvent) => {
  e.preventDefault();
  const kind = dragKind() ?? (e.dataTransfer!.getData("application/x-drone-kind") as BlockKind);
  if (!kind) return;
  const pos = screenToFlow({ x: e.clientX, y: e.clientY });   // uses @xyflow/system transform + canvas rect
  m.addNode(kind, pos);
};
```

`screenToFlow` = invert the current `{x, y, zoom}` viewport relative to the canvas bounding rect (or `rendererPointToPoint` from `@xyflow/system`). **Use `clientX/clientY`, not `screenX/screenY`.**

### 6.3 Node rendering (`<DroneNode>`)

```
┌───────────────────────────┐
●in  │ 🤖  Agent          ✕ │   header: emoji + label + (color strip) + close
     ├───────────────────────┤
     │ researcher · "summari…│   body: nodeSummary(node)  (existing helper)
     └───────────────────────┘ ●out
                               (Condition: ●true / ●false stacked on the right)
```

- Header strip tinted with `--block-color`; emoji in a `.dr-node__emoji`.
- **Ports = `<Handle>`-equivalent dots** on left (inputs) / right (outputs), positioned from `meta.inputs/outputs`. Each port carries `{nodeId, handleId, kind, dataType}`.
- During a run, node gets a status class (`is-running` pulse, `is-done` green, `is-error` red, `is-skipped` dim) driven by slice #10 `blockResults` / `status`.
- Body inputs that shouldn't start a node-drag get `class="nodrag"` (the drag handler ignores `pointerdown` originating inside `.nodrag`).
- `memo` is **not** needed (Solid doesn't re-render); correctness comes from store granularity (§9).

### 6.4 Wiring (drag-to-connect, typed, acyclic)

- **Initiate:** `pointerdown` on an **output** port → start a connection; render `<ConnectionPreview>` (bezier from port to cursor) following `pointermove`; `pointerup` over an **input** port → commit `addEdge`.
- **Typed validation** (`isValidConnection`): reject if
  - source === target (no self-loop),
  - target handle already satisfied where single-input semantics apply,
  - **type mismatch** (compare `outputs[h].type` vs `inputs[h].type`; `any` matches all),
  - **would create a cycle** (walk existing edges; reject if `target` can already reach `source` — preserves DAG-ness for the topological engine).
- **Condition branches:** dragging from the `true` / `false` output sets `sourceHandle` explicitly — replaces today's fragile "first edge = true, second = false" ordering hack (`drone-view.tsx:142-171`). Color the two ports (green `true`, grey `false`) and label on hover.
- **Edge select + delete:** click an edge → select (thicken/highlight); `Delete`/`Backspace` → `removeEdge`. Right-click → context menu (delete, for Condition: "swap branch").
- Keep **shift-click-to-connect** as a secondary affordance (accessibility / no-drag).

### 6.5 Node drag (in zoomed space)

- `pointerdown` on node header (not `.nodrag`) → `setPointerCapture`; record start cursor + node origin.
- `pointermove` → `moveNode(id, origin + (cursorΔ / zoom))` — **divide client delta by zoom** so drag tracks the cursor at any zoom level. Snap to grid (`Math.round(p / 16) * 16`) when snap is on.
- `pointerup` → release capture; mark draft dirty → debounced autosave (§9.3).
- Multi-select drag: move all selected nodes by the same delta.

### 6.6 Pan / zoom / fit / background / minimap

- **Pan:** space-drag or middle-mouse-drag or two-finger; updates the `viewport` store `{x,y}` only → one transform attribute changes, **zero** node bindings re-run (Solid win).
- **Zoom:** wheel (zoom toward cursor) and `+`/`-` controls; clamp `zoom ∈ [0.2, 2.5]`.
- **Fit-view:** compute bbox of all nodes → center + zoom to fit with padding. Bound to a Controls button and `⇧1`.
- **Background:** dot grid via a tiled SVG `<pattern>` whose transform follows the viewport (cheap; no per-dot nodes).
- **MiniMap (Phase 2):** scaled bbox render + draggable viewport rect, bottom-right overlay.

### 6.7 Selection, delete, multi-select, clipboard, undo/redo, shortcuts

- **Select:** click node/edge; shift-click to add; **box-select** by dragging on empty canvas.
- **Delete:** `Delete`/`Backspace` removes selected nodes (cascade their edges) + selected edges.
- **Copy/paste** (Phase 2): `⌘C`/`⌘V` duplicate selected nodes with offset + rewired internal edges + fresh ids.
- **Undo/redo** (Phase 2): snapshot the graph store on each committed mutation into a bounded past/future stack; `⌘Z` / `⌘⇧Z`. (Debounce drag into one snapshot per gesture.)
- **Shortcut table:**

| Keys | Action |
|---|---|
| `Del` / `Backspace` | Delete selection |
| `Space`+drag / middle-drag | Pan |
| Wheel / `⌘`+wheel | Zoom |
| `⇧1` | Fit view |
| `⌘A` | Select all |
| `⌘Z` / `⌘⇧Z` | Undo / redo (Phase 2) |
| `⌘C` / `⌘V` | Copy / paste (Phase 2) |
| `Esc` | Cancel connection / clear selection |
| `⌘S` | Force save |

### 6.8 Inspector as a slide-over

- When `selectedNodeAtom()` is non-null, slide a panel in from the right **over** the canvas (≈360px, translucent backdrop edge), with the existing `InspectorForm` content (agent/api/condition/response/variables editors + `AgentResultPanel`).
- Pin/unpin toggle; closing deselects. Never a permanent column.
- Verify `ListIdentityBundlesCommand` / `ListMemoriesCommand` + `IdentityBundle` / `Memory` types exist; fix imports if `gotypes` lacks them (`drone-view.tsx:467-470`).

### 6.9 Run overlay & live execution

- Run from the top bar (validation gate unchanged: ≥1 node, exactly one Response).
- During a run, drive node status classes from slice #10 (`BlockStarted`→running, `BlockDone`→done, `BlockError`→error; pruned branches→skipped). Edges on the active path animate (dash-flow).
- Run-log overlay (bottom, collapsible): per-run rows (`status · id · output/error`) — today's `RunPanel`, restyled as an overlay drawer, plus a per-node "last result" already in the inspector.

### 6.10 Auto-layout ("Tidy")

- Button in the top bar → run `@dagrejs/dagre` (`rankdir: "LR"`, sensible `nodesep`/`ranksep`) over current nodes/edges using measured node sizes → write resulting `x/y` back via `moveNode` (batched in one store update) → `fitView`. Animate transition (CSS transform) for polish.

---

## 7. Backend touchpoints (small)

The engine and RPC are complete. Only persistence ergonomics need attention:

1. **Viewport + positions persist through `upsertdrone`** — no new endpoint required. The frontend autosaves the whole `DroneDefinition` (debounced) on graph/viewport change.
2. **(Optional, nice-to-have) `update_drone_viewport(id, viewport)` lightweight RPC** to avoid rewriting the whole graph row on every pan/zoom. *Defer unless autosave write volume proves to be a problem* — Phase-1 contract is whole-graph upsert, and a debounce makes the cost negligible.
3. **(Optional) Orphaned-run recovery** — on server start, mark stale `running` rows as `interrupted` (already noted as a TODO in `drone_handlers.rs`). Out of scope for this spec; file separately.

No schema changes. No engine changes.

---

## 8. State model migration (the key correctness/perf change)

### 8.1 `createSignal<DroneDefinition>` → `createStore`

Today every mutation spreads the whole graph (`drone-model.ts:417-472`), so Solid sees a brand-new `nodes` array each time and re-runs the entire `<For>`. Replace with a Solid store so each node/field is independently reactive:

```ts
import { createStore, produce, reconcile } from "solid-js/store";

const [draft, setDraft] = createStore<DroneDefinition>(BLANK_DRONE());

// move ONE node — touches only that node's transform binding:
moveNode(id, pos) { setDraft("graph", "nodes", n => n.id === id, "position", pos); }
updateNodeData(id, patch) { setDraft("graph", "nodes", n => n.id === id, "data", d => ({ ...d, ...patch })); }
addNode(node)   { setDraft("graph", "nodes", ns => [...ns, node]); }   // add still appends; <For> keys by id
removeNode(id)  { setDraft(produce(d => {
  d.graph.nodes = d.graph.nodes.filter(n => n.id !== id);
  d.graph.edges = d.graph.edges.filter(e => e.source !== id && e.target !== id);
})); }
// backend → canvas without nuking DOM:
loadDrone(wf)   { setDraft(reconcile(wf, { key: "id" })); }
```

Expose accessors so the rest of the codebase is unaffected (`draftAtom()` can stay as a thin getter returning `draft`). `selectedNodeAtom` becomes a store path read.

### 8.2 Viewport store (isolated from nodes)

Keep `viewport` either as its own `createSignal<{x,y,zoom}>` or a store slice, applied as the single `<g transform>` on the viewport container. Panning/zooming must **not** touch any node binding.

### 8.3 Debounced autosave

- After any committed graph/viewport mutation, schedule a debounced (~800ms) `upsertdrone(draft)`; coalesce rapid drags into one write. Optimistic local state; on failure surface via `errorAtom` and retry.
- Drag gestures: write to the store live (for render), but only mark dirty / schedule autosave on `pointerup`.

### 8.4 Solid perf rules (replacing React's "memo everything")

Solid doesn't re-render components, so the perf model is *reactive granularity*:
- `createStore` for nodes/edges (above); never a signal holding the array.
- `<For>` keyed by node id; stable references via path mutation (no array spreads on hot paths).
- `reconcile` when replacing graph data from the backend.
- One viewport transform outside the node loop.
- Per-edge `createMemo` reading only its two endpoints' positions → moving a node recomputes only incident edges.
- Pointer-capture drag writing straight to the store; no global listeners.
- For >~1–2k nodes (unlikely near-term): viewport-bbox `createMemo` cull before `<For>`.

---

## 9. Phased implementation plan

Each phase is an independently shippable PR (changesets workflow — `task changeset -- ...`, **no manual version bump**).

| PR | Title | Deliverable | Unblocks "nothing works" |
|---|---|---|---|
| **1** | Canvas store migration | `createSignal<DroneDefinition>` → `createStore`; path mutations; `reconcile` on load. No UX change yet. | Foundation |
| **2** | Pan/zoom + node drag | Add `@xyflow/system`; viewport transform; wheel-zoom; pointer-capture node drag (zoom-aware); Controls (zoom/fit). | **Drag + navigate works** ✅ |
| **3** | Top node-type bar + drag-from-bar | Replace `BlockPalette` aside with `<NodeTypeBar>` (emoji chips); HTML5 DnD drop → `screenToFlow` → `addNode`; click-to-center fallback; add `emoji` to registry. | **Layout matches vision; intuitive add** ✅ |
| **4** | Ports + drag-to-connect + typed/acyclic validation | Visible handles; connection preview; `isValidConnection` (type + cycle); explicit Condition branch handles; edge select/delete. | **Real wiring** ✅ |
| **5** | Inspector slide-over + run overlay + node run-state | Move inspector to overlay drawer; run-log overlay; live node status glow + animated active edges. | Expansive canvas; live feedback |
| **6** | Auto-layout + polish | `@dagrejs/dagre` "Tidy"; bezier/smoothstep edges; dot-grid background; debounced autosave; a11y pass. | Pro feel |
| **7** (opt) | MiniMap · undo/redo · copy-paste · search/Cmd-K | Power-user features. | — |

**Minimum to kill "nothing works": PRs 1–3** (store + pan/zoom + drag + top bar with drag-drop). PR-4 makes it genuinely useful.

## 10. File-level change map

- `frontend/app/view/drone/block-registry.ts` — add `emoji` to `BlockKindMeta` + each entry; (drop unused `icon` or repurpose).
- `frontend/app/view/drone/drone-model.ts` — `createSignal`→`createStore`; rewrite `addNode/removeNode/updateNodeData/moveNode/addEdge/removeEdge` as path mutations; `reconcile` in `openDrone/newDrone/save`; add `setViewport`, dirty/autosave scheduler, `isValidConnection`, `addNodeAtCenter`, multi-select state.
- `frontend/app/view/drone/drone-view.tsx` — **split** into:
  - `node-type-bar.tsx` (`<NodeTypeBar>`, `<NodeChip>`)
  - `canvas.tsx` (`<Canvas>`, `<NodeLayer>`, `<EdgeLayer>`, `<DroneNode>`, `<Handle>`, `<ConnectionPreview>`, `<Background>`, `<Controls>`, `<MiniMap>`)
  - `inspector-drawer.tsx` (existing `InspectorForm`/editors moved here)
  - `run-overlay.tsx` (existing `RunPanel` restyled)
  - `drone-view.tsx` becomes the thin `<DronePane>` shell (grid: topbar / canvas).
- `frontend/app/view/drone/canvas-geometry.ts` (new) — `screenToFlow`, `flowToScreen`, bbox/fit math, dagre adapter, `@xyflow/system` wrappers.
- `frontend/app/view/drone/drone-view.scss` — rewrite to full-bleed grid + overlays + node/handle/edge styles.
- `package.json` — add `@xyflow/system` (pinned exact) + `@dagrejs/dagre`.
- **No backend changes** (optional viewport RPC tracked separately).

## 11. Testing

- **Unit (vitest):** store mutations (add/remove/move/connect cascade), `isValidConnection` (self-loop, type mismatch, **cycle rejection**), `screenToFlow` round-trip at various zoom/pan, dagre adapter writes valid coords, Condition branch-handle assignment.
- **Component:** drag-from-chip creates a node at the drop point; node drag updates position by `Δ/zoom`; edge connect/delete; inspector opens on select.
- **E2E (`e2e/`):** open drone widget → drag Agent + Condition + Response from bar → wire them → Run → assert live status classes + run-log row. Reuses backend RPC end-to-end.
- **Perf smoke:** 100-node graph stays smooth on drag/pan in the CEF webview (profile: dragging one node updates only that node + incident edges).

## 12. Risks & open questions

- **`@xyflow/system` is `0.0.x` / internally scoped** → pin exact; wrap all usage behind `canvas-geometry.ts` so a breaking bump is a one-file fix; escape hatch = raw `d3-zoom` + copied path math.
- **HTML-over-SVG alignment** under zoom — both layers must share the identical transform; cover with a round-trip test.
- **`gotypes` missing `IdentityBundle`/`Memory`** — verify in PR-1; regenerate/declare if absent.
- **Autosave write volume** on pan/zoom — debounce; revisit optional viewport RPC only if measured cost warrants.
- **Emoji rendering consistency** in CEF — verify the chosen glyphs render across the bundled Chromium; swap any that fall back to tofu.
- **Reserved Phase-2 node types** — show as disabled or omit; don't half-wire engine-less kinds.

## 13. Appendix

**Dependencies to add**
```jsonc
"@xyflow/system": "0.0.77",   // EXACT pin — vendored math: XYPanZoom, edge paths, coord transforms
"@dagrejs/dagre": "^3.0.0"     // auto-layout (LR) for "Tidy"
```

**`BlockKindMeta` addition**
```ts
export interface BlockKindMeta {
  kind: BlockKind;
  emoji: string;          // NEW — top-bar chip + node header
  label: string;
  description: string;
  color: string;          // header strip + handle accent
  defaultData: Record<string, unknown>;
  inputs: BlockHandleSpec[];
  outputs: BlockHandleSpec[];
}
// variables 🔢 · agent 🤖 · api 🌐 · condition 🔀 · response 🏁
```

**Canonical references**
- Existing: `drone-view.tsx`, `drone-model.ts`, `block-registry.ts`, `drone-types.ts`, `store/drone-run-state/*`; backend `agentmux-srv/src/drone/*`, `server/drone_handlers.rs`.
- External: `@xyflow/system` source (`packages/system/src/xypanzoom`, `xyhandle`, `utils`), `@dagrejs/dagre` README, React Flow drag-and-drop / validation / custom-node guides (as conceptual reference — APIs are React, patterns are portable), SolidJS stores + fine-grained reactivity docs.
```
