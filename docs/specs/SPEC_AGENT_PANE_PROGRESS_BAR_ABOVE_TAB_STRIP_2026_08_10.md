# SPEC: Move the agent pane's marching-ants progress bar above the tab strip

**Date:** 2026-08-10
**Status:** implemented.
**Related:** `docs/specs/SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md` (§4 —
original progress-bar design), `docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`
(tab strip shrink-to-fit/transparency — a separate, already-shipped fix
investigated alongside this one; see §0).

---

## 0. Context

This spec was written while investigating a live report: "at the top of
the [agent] pane, there is a `+` to add tabs, but to the right, there is
black space that hides the conversation." Auditing the tab strip itself
(`PaneTabStrip.tsx`/`.scss`) found that exact symptom already fixed on
`main` (PRs #2282, #2289 — shrink-to-fit sizing, transparent container
background). A production build from current source confirmed the compiled
CSS is correct.

While re-investigating why the symptom might still be observed, the user
asked for something narrower and concrete instead: the pane's marching-ants
progress bar ("the progress swirly bar") should render in its own row,
above the tab strip and below the pane's own header — not wherever it
currently ends up. That's what this spec covers. Whether a stuck/lingering
`.agent-pane-loading-overlay` (a separate, full-pane-cover element shown
during initial history load — untouched by this spec) is the real
explanation for the original black-space report is a live open question,
not resolved here — see §5.

---

## 1. Current behavior (audited against source before this change)

The agent pane's full vertical stack, verified across two separate
components in `agent-view.tsx`:

1. **`.block-frame-default-header`** (`frontend/app/block/blockframe.tsx`)
   — the pane's own header (icon, title, controls). Entirely outside
   `agent-view.tsx`; not touched by this spec.
2. **`AgentViewWrapper`** — renders `.agent-pane-stack` (a flex column):
   `<PaneTabStrip>` (28px row) then `.agent-pane-stack-content` (fills the
   rest), which mounts `AgentPresentationView`.
3. **`AgentPresentationView`** (the component whose root is `.agent-view`)
   — nested *inside* `.agent-pane-stack-content`, i.e. **below the tab
   strip in DOM order**. Its own root div carries `position: relative`,
   `overflow: hidden`, and a per-pane CSS `zoom`.

The progress bar (`.agent-pane-progress-bar`) lived inside
`AgentPresentationView`, `position: absolute; top: 0; left: 0; right: 0`
relative to `.agent-view`. Because `.agent-view` is nested inside
`.agent-pane-stack-content` — which only begins *after* the tab strip's own
28px row in the flex column — the bar's `top: 0` was already anchored to
the top of the *content* region, not overlapping the tab strip. It could
never have visually rendered "above the tab strip" through CSS alone: every
ancestor between `AgentPresentationView`'s root and the tab strip
(`.agent-view` itself has `overflow: hidden`) clips anything that tried to
escape upward via a negative offset before it could become visible there.
Moving it above the tab strip requires the bar's rendered DOM node to
actually live in `AgentViewWrapper`'s tree — a real structural change, not
a CSS tweak.

---

## 2. Design

### 2.1 Why not lift the bar's state up to `AgentViewWrapper` instead?

Considered and rejected. The bar's visibility depends on
`agentAtoms().turnPhaseAtom` (`createAgentAtoms(model.blockId)`) and
`showingLaunchActivity()` (from `useAgentControllerStatus(...)`), both
currently computed inside `AgentPresentationView`. `useAgentControllerStatus`
sets up its own polling/event subscriptions scoped to a block id — calling
it a second time from `AgentViewWrapper` for the same block would double-
subscribe, not just duplicate a cheap read. Keeping the state in
`AgentPresentationView` (single source of truth, already correctly scoped)
and moving only *where its output renders* is the smaller, safer change.

### 2.2 Portal into a slot `AgentViewWrapper` owns

`AgentViewWrapper` renders a new, always-present, fixed-height div between
`<PaneTabStrip>` and `.agent-pane-stack-content`:

```tsx
const [progressBarSlot, setProgressBarSlot] = createSignal<HTMLDivElement>();
...
<PaneTabStrip ... />
<div class="agent-pane-progress-bar-slot" ref={(el) => setProgressBarSlot(el)} />
<div class="agent-pane-stack-content">
    <AgentPresentationView ... progressBarMount={progressBarSlot} />
</div>
```

A signal, not a plain ref variable assigned once — `<Portal mount={...}>`
needs a value `AgentPresentationView` can read reactively on its own first
render without racing the sibling `ref` callback that sets it. In practice
DOM refs fire before a later sibling's child component renders (JSX order),
but the signal removes any doubt and lets the render gracefully handle the
one-frame window before the ref fires (`<Show when={progressBarMount()}>`).

`AgentPresentationView` receives `progressBarMount: () => HTMLDivElement |
undefined` as a new prop and wraps its (otherwise unchanged) progress-bar
JSX in `<Show when={progressBarMount()}><Portal mount={progressBarMount()!}>
...</Portal></Show>` instead of rendering it inline in its own tree. All of
the bar's state reads, classes, and ARIA attributes are unchanged — only
where the resulting DOM node is mounted moves.

### 2.3 The slot is always present, at a fixed height — no layout shift

`.agent-pane-progress-bar-slot` is unconditionally rendered at `height: 3px`
regardless of whether the bar is currently active. The bar itself still
fades in/out via `opacity` exactly as before; because the *slot* never
changes size, toggling the bar's visibility never shifts the tab strip or
the content area by even a pixel. (An alternative — only reserving the
row's height while the bar is active — was rejected specifically to avoid
that shift.)

### 2.4 Dropping the zoom-compensation math

The original CSS divided every pixel dimension by `--agent-pane-zoom`
(`agent-view.tsx` sets this custom property, alongside the CSS `zoom` it
applies, scoped to `.agent-view`'s own subtree) so the bar rendered at a
fixed screen size regardless of per-pane zoom. Since the bar's new home
(`AgentViewWrapper`'s tree, alongside the tab strip) is never inside
`.agent-view`'s zoomed subtree at all — Portal moves the rendered DOM node
out of that subtree entirely, regardless of the component-tree
relationship — that compensation is now unnecessary weight, not a
correctness gap: the bar was already meant to render at a fixed screen size,
and living in unzoomed pane chrome achieves that automatically. Removed.

---

## 3. Files touched

- `frontend/app/view/agent/agent-view.tsx` — `Portal` import;
  `progressBarSlot` signal + slot div in `AgentViewWrapper`; new
  `progressBarMount` prop on `AgentPresentationView`; the progress bar's
  JSX wrapped in `<Show>`/`<Portal>` instead of rendered inline.
- `frontend/app/view/agent/agent-view.scss` — new
  `.agent-pane-progress-bar-slot` rule (in `.agent-pane-stack`'s block) and
  the relocated, zoom-math-simplified `.agent-pane-progress-bar` +
  `@keyframes agent-ant-march`.
- `frontend/app/view/agent/styles/_control-bar.scss` — old
  `.agent-pane-progress-bar` rule and `@keyframes agent-ant-march` removed
  (was nested under `.agent-view`, no longer applicable since the bar isn't
  a DOM descendant of `.agent-view` anymore).
- No `agentmux-srv` (Rust) changes. No wire-format changes — purely a
  frontend DOM/CSS restructuring; the bar's own visibility logic (turn
  phase, launch activity) is byte-for-byte unchanged.

---

## 4. Verification

- `npx tsc --noEmit` — clean.
- Full `frontend/app/view/agent/` suite — 85 files / 1102 tests passing (no
  existing test targeted this component tree directly; none needed
  updating).
- `bash scripts/vite-build.sh --mode production` — compiles cleanly;
  confirmed the compiled CSS output contains the new
  `.agent-pane-progress-bar-slot` rule with the expected properties.
- **Not verified visually** — this sandbox has no display. The actual
  on-screen result (bar rendering in its own row, above the tab strip,
  below the pane header, with no layout shift when it fades in/out) needs a
  live `task dev` or packaged-build check before treating this as fully
  confirmed, same caveat as this session's other UI changes.

---

## 5. Open: is this actually what caused the original "black space" report?

Not established either way. `.agent-pane-loading-overlay` — a separate,
untouched-by-this-spec element (`frontend/app/view/agent/agent-view.tsx`,
styled in `styles/_loading-overlay.scss`) — is `position: absolute; inset:
0` relative to `.agent-view`, with an opaque `--main-bg-color` background
and a high z-index (`--zindex-elem-modal`), shown "from mount until the
initial history load resolves." If that overlay ever lingers longer than
expected, or fails to clear promptly, it would cover the *entire* content
region (though still not the tab strip itself, per §1's DOM ordering) with
a solid, theme-background-colored rectangle — a materially closer match to
"black space that hides the conversation" than anything about the tab
strip's own background, which was already independently confirmed correct.
This is a live lead, not a confirmed diagnosis — flagged here rather than
silently dropped, since it directly bears on the report that started this
whole investigation and this spec doesn't resolve it.
