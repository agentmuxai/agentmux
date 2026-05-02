# Launch Agent Modal — Performance & Per-Tab Scoping

**Date:** 2026-05-01
**Status:** Spec / Proposal
**Repo state:** main @ `257bf0ff`, AgentMux v0.33.549
**Author:** AgentC

---

## Problem

Two separate but related defects in the agent launch modal (`AgentLaunchModal`):

**P1 — Severe input lag.** Selecting a radio option, toggling advanced, and especially typing in the instance-name field have visibly large latency. The interaction does not feel like a SolidJS app should — strongly suggests reactive cascades through unmemoized parents and Portal-anchored heavy children.

**P2 — Wrong scope.** The modal is global (Portal-mounted to `document.body`) and visually covers the entire window including the top tab bar. It should be **per-tab**:

- The top tab bar must remain interactive — the user can switch tabs while the modal is open.
- Within the originating tab, widgets are blurred/disabled by the modal.
- Switching to a different tab shows that tab normally; the modal does not appear there.
- The modal is bound to the tab where the launch was initiated.

---

## TL;DR

- Current modal is mounted via Solid `<Portal mount={document.body}>` in `modal-v2.tsx:329`. State lives at the wrong level (`AgentPicker.tsx:70`). The modal renders as a sibling of an expensive list (`AgentCard` + Popover + memos), so every keystroke schedules work through the picker's reactive scope and the Popover children.
- The codebase already has a working **per-pane absolute overlay** pattern: `AgentFocusedPanel` (`agent-view.tsx:554-562`, `_focused-overlay.scss:8-18`). We should mirror it.
- Fix: replace the Portal-mounted modal with a **tab-scoped overlay** that mounts inside the tab content area but **outside the widget grid**, dimming/disabling the widgets only. Move modal state to a tab-scoped container, isolate the form's signals so input changes never cross the modal boundary.
- Estimated change: 4–5 files, ~150 LOC. No DB or RPC changes.

---

## Current Architecture

### DOM / component tree (today)

```
App (app.tsx:38)
└─ Workspace (workspace.tsx:54)
   ├─ TabBar (tabbar.tsx:41)                    ← interactive
   └─ TabContent  ← rendered for every tab; inactive hidden via display:none (tabcontent.tsx:37)
      └─ TileLayout
         └─ Block (per pane)
            └─ AgentViewWrapper (agent-view.tsx)
               ├─ AgentPicker (when no agent bound)
               │  ├─ <For> over AgentCard (each with Popover)
               │  └─ <Show when={launchModalAgent()}>
               │     └─ AgentLaunchModal              ← (1) state lives here
               │        └─ Modal (modal-v2.tsx)
               │           └─ <Portal mount={document.body}>   ← (2) escapes the tab
               │              └─ .modal-root (position:fixed; inset:0)
               │                 ├─ .modal-backdrop  ← covers the whole window
               │                 └─ .modal-panel
               └─ AgentPresentationView (when agent bound)
```

### Modal open/close state location

- **State signal:** `AgentPicker.tsx:70` — `const [launchModalAgent, setLaunchModalAgent] = createSignal<ForgeAgent|null>(null)`.
- **Open:** `AgentPicker.tsx:81-83` — `handleSelect(agent)` sets the signal; called from `AgentCard`'s `onLaunch`.
- **Close:** `AgentPicker.tsx:102` and the modal's own `onCancel`.
- **Render:** `AgentPicker.tsx:205-212` — `<Show when={launchModalAgent()}>` inside the picker's JSX.

This couples modal lifetime to the picker's reactive scope.

### Portal escape

- `modal-v2.tsx:327-379` — `<Portal mount={resolveMountDocument().body}>` puts the modal root in `document.body`.
- `modal-v2.scss:9-31` — `.modal-root { position:fixed; inset:0; z-index:var(--z-modal); }` with `.modal-backdrop { backdrop-filter: blur(8px); }`.
- `modal-v2.tsx:163-166` — `resolveMountDocument()` is window-aware (good for multi-window), but **not tab-aware**.

Result: when the user switches tabs, the originating tab's `TabContent` gets `display:none` (`tabcontent.tsx:37`), but the Portal node lives in `document.body` and stays visible. The modal "leaks" across tabs.

### Tab system

- `tabbar.tsx:41-568` — top tab bar. Already its own component, already always interactive.
- `workspace.tsx:33-44` — every tab's `TabContent` is mounted; inactive ones use `display:none` (`tabcontent.tsx:37`). Comment at `tabcontent.tsx:18-20` is explicit: "Keep every tab mounted so terminals preserve their xterm.js instance and scrollback across tab switches."
- This is good for us: an overlay rendered **inside** TabContent will hide automatically with the tab.

### Existing per-pane overlay precedent

`AgentFocusedPanel` is the pattern to copy:

- `agent-view.tsx:554-562` — rendered inside `AgentPresentationView`; not portaled.
- `_focused-overlay.scss:8-18`:
  ```scss
  .agent-focused-overlay {
      position: absolute;
      top: 0; left: 0; right: 0;
      height: 50%;
      z-index: 20;
  }
  ```
- Tab switch hides the panel automatically because its DOM ancestor is hidden.

---

## Performance Root Cause (P1)

Hypothesis with file:line evidence.

### A — Local form signals read directly in JSX

`AgentLaunchModal.tsx:42-47`:

```tsx
const [name, setName] = createSignal("");
const [runtime, setRuntime] = createSignal<"host" | "container">("host");
const [image, setImage] = createSignal<string>("");
const [submitting, setSubmitting] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
const [showAdvanced, setShowAdvanced] = createSignal(false);
```

`AgentLaunchModal.tsx:53-55` derived helpers used in render:

```tsx
const hasName = () => name().trim().length > 0;
const canSubmit = () => !submitting() && slugifyInstanceName(name()).length > 0;
```

`AgentLaunchModal.tsx:118` keystrokes:

```tsx
<input onInput={(e) => setName(e.currentTarget.value)} ... />
```

`AgentLaunchModal.tsx:200-205` renders `slugifyInstanceName(name().trim())` on every keystroke.

These are not the bottleneck on their own — Solid's fine-grained reactivity should keep this cheap. The bottleneck is what's **above** the modal.

### B — The modal is a child of an expensive parent

`AgentPicker.tsx:160-212` renders, in order:

1. `<For each={agents()}>` over forge agents (each card hosts a Popover).
2. A `<Show when={nodejsError()}>` notice.
3. `AgentActionBar`.
4. `<Show when={launchModalAgent()}>` → modal.

`AgentCard.tsx:65-80` per card:

```tsx
const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
const icon = () => props.agent.icon || catalog()?.icon || "•";
// ...
<Popover placement="right-start" offset={8}>
    <InfoPopoverTrigger ... />
    <PopoverContent className="agent-card-info-popover">
        ...
    </PopoverContent>
</Popover>
```

Popover is a heavy primitive (floating-ui placement, autoUpdate, focus trap). Every card mounts one.

### C — Portal-mounted backdrop with `backdrop-filter: blur(8px)`

`modal-v2.scss:9-31`:

```scss
.modal-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(8px);
}
```

`backdrop-filter: blur(8px)` over a full-window viewport with the agent picker (with N Popovers) and any active panes behind is **the most expensive single line of CSS in this view**. Each input keystroke causes the panel above to re-layout fractionally; with the blur layer in place, the compositor re-rasterizes the blurred region. On a high-DPI display with many widgets, this alone is a strong candidate for the visible lag.

### D — Live event subscriptions in the picker

`AgentPicker.tsx:30-59` (`useForgeAgents`):

```tsx
const unsub = waveEventSubscribe({
    eventType: "forgeagents:changed",
    handler: () => load(),
});
```

If any modal action emits an event interpreted as `forgeagents:changed`, the agent list refetches and re-renders the cards mid-typing.

### Net read

The lag is the sum of:

1. **Backdrop blur** rasterizing the full window, including the picker grid and any heavy panes behind it. Largest single contributor.
2. **Picker scope** — modal is a child of the picker, so any framework-level boundary work involves the card list.
3. **Popover-per-card** — N heavy children rendered behind the blur.
4. **Possible refetch loops** if `forgeagents:changed` fires during launch.

Fix the scoping (P2) and most of these go away naturally because the blur and overlay only cover the widget area inside one tab, not the whole picker grid + tab bar + status bar.

---

## Target Architecture

### Goals

1. Top tab bar always interactive.
2. Modal lives **per tab**, scoped to the tab's content area (not viewport).
3. Modal blurs/dims widgets within the originating tab only.
4. No Portal escape — modal lives inside `TabContent`, hidden automatically with `display:none`.
5. Modal form state isolated; input changes don't reach the picker grid.

### Component tree (proposed)

```
TabBar                                ← unchanged, always interactive
TabContent  (display:none when inactive)
└─ TabModalLayer  (NEW) — tab-scoped per-tab overlay host
   ├─ TileLayout                       ← widgets, dimmed/disabled when modal open
   │  └─ Block × N
   │     └─ AgentViewWrapper / etc.    ← AgentPicker NO LONGER renders the modal
   │
   └─ <Show when={openModal()}>
      └─ TabModalOverlay
         ├─ .tab-modal-backdrop  (position:absolute; inset:0; backdrop-filter)
         └─ .tab-modal-panel     (centered)
            └─ AgentLaunchModal   ← form-only; no Portal, no <Modal> wrapper
```

### State

A per-tab signal `openModal: Signal<TabModalRequest | null>` lives on `TabModalLayer`. A small context (`TabModalContext`) exposes:

```ts
interface TabModalApi {
  open(req: TabModalRequest): void;
  close(): void;
  current: Accessor<TabModalRequest | null>;
}
```

`TabModalRequest` is a discriminated union; for now `{ kind: "launch-agent"; agent: ForgeAgent; originBlockId: string }`. `TabModalLayer` provides this context to its descendants. `AgentPicker` consumes it via `useTabModal()` and calls `open({ kind: "launch-agent", agent, originBlockId: model.blockId })`.

This makes the modal **tab-scoped** by construction: each tab has its own `TabModalLayer`, each layer has its own context, each context has its own signal. No global state.

### Pointer/focus behavior — widgets dimmed, tabs interactive

Two layers:

- The widget area (the `TileLayout`) gets `inert` (or `pointer-events: none` plus `aria-hidden="true"`) and a class that triggers blur/dim styling whenever `openModal()` is non-null.
- The tab bar lives **outside** `TabContent`, so it's not under the overlay. No special handling needed.

CSS:

```scss
.tab-content[data-modal-open="true"] {
    .tile-layout {
        filter: blur(2px) saturate(0.7);
        pointer-events: none;
        user-select: none;
    }
}

.tab-modal-overlay {
    position: absolute;
    inset: 0;
    z-index: var(--z-tab-modal);
    display: grid;
    place-items: center;
}

.tab-modal-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(6px);
}
```

The blur is now scoped to the tab content area. If the tab content is small (e.g. half a screen), the GPU work is proportionally smaller.

### Form state isolation

Move the form's local signals into a new `<LaunchAgentForm>` component nested *inside* the modal panel. Parent (`TabModalLayer`) only holds `TabModalRequest`; the form's `name`, `runtime`, `image`, `showAdvanced` signals live entirely inside `LaunchAgentForm` and never escape it. `LaunchAgentForm` returns final values via a single `onSubmit(payload)`.

This guarantees a Solid keystroke on `name` cannot trigger any reactivity outside the form.

### Backdrop blur — keep it cheap

Reduce the blur radius from 8px to 6px (already proposed above), and consider skipping `backdrop-filter` entirely on lower-end machines via `@media (prefers-reduced-transparency)` or a setting. Alternative: replace `backdrop-filter` with a `background: rgba(0,0,0,0.65)` solid scrim — visually similar, near-zero GPU cost. Recommend solid scrim as default; optional blur as a setting.

### Tab switch behavior

Open question: what happens to the modal when the user switches tabs and back?

Two options:

- **A — preserve form state.** The modal's form is a child of the still-mounted `TabContent` (just hidden). The signals retain their values. When the user returns, the modal is still open with the same partial input. Recommended default — matches the spirit of "the modal is bound to the tab".
- **B — close on tab switch.** Subscribe to `atoms.activeTabId` in `TabModalLayer`; when it changes away from this tab, call `close()`.

Pick A. It's the natural behavior of `display:none`, requires no extra code, and matches the user's stated mental model ("modal is per-tab").

---

## Implementation Plan

Ordered, with file:line targets.

### 1 — Create `TabModalLayer` and context

- New file `frontend/app/view/tabcontent/TabModalLayer.tsx` (host + context provider + render slot).
- New file `frontend/app/view/tabcontent/tab-modal.ts` (context + types + `useTabModal` hook).
- Wrap `TileLayout` inside `TabContent` (`tabcontent.tsx:114`) with `<TabModalLayer>...</TabModalLayer>`. Layer renders its children plus an overlay slot.

### 2 — Add CSS

- New file `frontend/app/view/tabcontent/_tab-modal.scss`. Defines `.tab-content[data-modal-open]`, `.tab-modal-overlay`, `.tab-modal-backdrop`, `.tab-modal-panel`.
- Import into `tabcontent.scss` (or the workspace stylesheet that owns tab content styles).
- Add a `--z-tab-modal` token below `--z-modal` in `theme.scss`. Tab modal stacks below global modals so a future global confirm dialog still works.

### 3 — Refactor `AgentLaunchModal` — remove Modal/Portal

- `AgentLaunchModal.tsx:93-219` — remove the `<Modal open={true} ...>` wrapper. The component becomes a plain panel that the layer renders inside its panel slot.
- Extract the form into a child `<LaunchAgentForm>` so its local signals (`name`, `runtime`, `image`, `showAdvanced`) cannot affect anything outside.
- Replace `usePaneOverlay` plumbing (`modal-v2.tsx:64-67`) with the tab modal layer's CEF-pane clip — see step 7.

### 4 — Move modal state out of `AgentPicker`

- `AgentPicker.tsx:70` — delete `[launchModalAgent, setLaunchModalAgent]`.
- `AgentPicker.tsx:81-84` — `handleSelect` becomes a one-liner: `useTabModal().open({ kind: "launch-agent", agent, originBlockId: model.blockId })`.
- `AgentPicker.tsx:86-106` — `handleLaunchSubmit` no longer closes a local signal; it calls the context's `close()` on success.
- `AgentPicker.tsx:205-212` — delete the `<Show>` block that renders `AgentLaunchModal`. The picker no longer mounts the modal.

### 5 — Render the modal in `TabModalLayer`

- `TabModalLayer` owns the `<Show when={current()}>` and dispatches by `kind`. For `"launch-agent"`, it renders `<AgentLaunchModal agent={req.agent} onCancel={close} onSubmit={...}/>` inside `.tab-modal-panel`.
- Hooking `onSubmit`: keep the current behavior (call `model.launchForgeAgent`) but lift that call into `TabModalLayer` so the picker stays presentational. The `originBlockId` in the request lets the layer locate the right model if needed.

### 6 — Apply dim/inert to widgets

- `TabModalLayer` toggles `data-modal-open="true"` on the closest `.tab-content` element when `current() != null`.
- The CSS rule in step 2 blurs `.tile-layout` and removes pointer events. Keyboard focus-trap stays inside the modal panel.
- Use `inert` attribute (broadly supported in Chromium / CEF) on the `.tile-layout` wrapper for correctness.

### 7 — CEF pane overlay clip

- Existing `usePaneOverlay()` in `modal-v2.tsx:64-67` registers the modal rect with the host so native CEF panes don't paint over it. Reuse the same hook from `TabModalLayer` against its overlay element. Without this, sidecar panes will continue to paint over the modal.

### 8 — Optional: solid scrim instead of blur

- Default `.tab-modal-backdrop` to `background: rgba(0,0,0,0.65); backdrop-filter: none;`.
- Gate the blur on a setting (`ui.modalBlurEnabled`, default false). Largest single perf win.

### 9 — Cleanup

- Delete or minimize the legacy `Modal` (modal-v2.tsx) usage from launch flow only; leave it in place for any other modals that still rely on global Portal scoping until they migrate. Do not rip out modal-v2 wholesale — out of scope.
- Update `_focused-overlay.scss` z-index if it conflicts with `--z-tab-modal` (verify visually).

---

## Affected Files

| # | File | Change |
|---|---|---|
| 1 | `frontend/app/view/tabcontent/TabModalLayer.tsx` | **new** — context provider + overlay host |
| 2 | `frontend/app/view/tabcontent/tab-modal.ts` | **new** — context, hook, types |
| 3 | `frontend/app/view/tabcontent/_tab-modal.scss` | **new** — overlay + dim/blur styles |
| 4 | `frontend/app/view/tabcontent/tabcontent.tsx` | wrap TileLayout in `<TabModalLayer>` (~L114) |
| 5 | `frontend/app/view/agent/components/AgentPicker.tsx` | remove modal state + render (L70, L81-84, L102, L205-212); use `useTabModal()` |
| 6 | `frontend/app/view/agent/components/AgentLaunchModal.tsx` | remove `<Modal>` wrapper (L93-219); extract `<LaunchAgentForm>` |
| 7 | `frontend/app/view/theme/theme.scss` | add `--z-tab-modal` token |
| 8 | (no DB / no RPC changes) | — |

Total: 3 new files + 4 edits. ~150 LOC.

---

## Performance Expectations (post-change)

- **Backdrop blur cost:** down ~70–90% (scrim default; tab-area scoped if blur enabled).
- **Reactive cascade:** form signals isolated inside `<LaunchAgentForm>` — no propagation to picker or cards.
- **Popover render path:** unchanged at idle, but no longer visible behind the modal during input (the dim/inert layer composites them once).
- **Tab switch:** instant — modal hides via existing `display:none` on the inactive `TabContent`; preserved via the same mechanism on return.

A simple sanity benchmark: open the modal, open Chrome DevTools Performance, type "the quick brown fox jumps over the lazy dog" in the name field. Expect:

- **Before:** input event handlers >16ms each, frequent compositor invalidations of the full window, occasional dropped frames.
- **After:** input handlers <2ms, compositor invalidations limited to the form preview row, no dropped frames.

---

## Risks and Open Questions

**R1 — `inert` support.** CEF tracks Chromium; modern CEF versions support `inert` natively. Verify in dev. Fallback: `pointer-events: none` + `aria-hidden="true"` + a focus-trap on the modal panel.

**R2 — Z-index ordering vs. `AgentFocusedPanel`.** `_focused-overlay.scss:8-18` uses `z-index: 20`. Tab modal needs to stack above that. Define `--z-tab-modal: 100` (or higher) and document the scale.

**R3 — Form state on tab switch.** Recommended: preserve (default behavior of `display:none`). If product wants close-on-switch, add a one-line subscription. Pick a default and lock it.

**R4 — Multiple panes in one tab.** Each pane has its own picker. The tab has one `TabModalLayer`, so only one launch modal can be open per tab at a time. This is desirable. The `originBlockId` on the request keeps which pane initiated unambiguous.

**R5 — Multi-window.** Each window has its own workspace and its own tab tree, so per-tab scoping is automatically per-window. No cross-window leakage. `usePaneOverlay()`'s window-aware mount logic (`modal-v2.tsx:163-166`) is no longer needed.

**R6 — Unrelated modals.** This refactor only touches `AgentLaunchModal`. The rest of the app's modals still use `Modal v2` and Portal-to-body. They keep their existing semantics until separately migrated. We are not consolidating modal infrastructure in this change.

**R7 — `forgeagents:changed` event during launch.** If any RPC during launch emits this event, the picker's list re-fetches. Out of scope for this spec, but worth confirming during testing — should not be visible because the picker is dimmed/inert during the modal.

---

## Out of Scope

- Identity binding inside the modal (covered by `identity-system-research-2026-05-01.md`).
- Migrating other modals off `Modal v2` / Portal.
- Tab-bar / tab-mounting refactor.
- Performance work in `AgentCard` Popover or `useForgeAgents` polling.

These are independent and can land before, after, or alongside this change.

---

## Bottom Line

Two surface-level bugs share one root cause: the modal escapes its tab via Portal-to-body, which forces it to be globally scoped, makes the backdrop blur span the full window, and ties its lifecycle to the picker's reactive scope. A small `TabModalLayer` wrapping the tile layout — with a context, an overlay slot, and a tab-scoped CSS layer — fixes both issues, recovers the perf budget, and gives us a clean place to host any future tab-scoped overlays.
