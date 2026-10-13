# Spec: a collapsed tool panel lets go of its body

**Date:** 2026-10-12
**Status:** Active, implemented together with this spec in #4756. Item 2 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Evidence:** `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md` §4.1.

## 1. The problem

A tool row's panel body is the full tool output: a shiki-highlighted diff, a file preview, Bash output. `ToolBlock` mounts it the first time the panel opens and never unmounts it (`ToolBlock.tsx:240-249`):

```ts
let bodyEverShown = false;
const bodyMounted = createMemo(() => bodyEverShown || (bodyEverShown = expanded()));
```

This was meant for restored history: a row that is never opened never builds its body (`SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` F4).

**Why every live tool keeps its body**
- `rowDisclosure` opens a panel automatically while its tool runs or waits for approval.
- It also holds the panel open after finishing on screen, until the scroll-off scan releases the hold.
- So in a live session every tool call opens once, builds its body, and keeps it after collapsing, for as long as the row exists.

**Why the collapsed body still costs memory**
- The collapsed panel uses `content-visibility: hidden` (`SPEC_TOOL_BLOCK_COLLAPSED_OVERLAY_LAYOUT_2026_06_02.md`).
- That skips layout and paint work, but by design it keeps the subtree's DOM and its rendering state. The memory stays.

**Measured**, in the lab, a production build with scripted agents:
- In one agent pane, 24,187 of 25,479 elements (95%) were inside collapsed tool panels.
- Blink kept their layout and text-shaping objects (`LayoutText`, `ShapeResult`, HarfBuzz glyph data), and SolidJS kept about 8 JS objects per element.
- A prototype that unmounted the body after collapse, combined with the `<Key>` fix:
  - 4 panes: renderer **850 → 407 MB**, JS heap 246 → 76 MB, elements 137k → 5.3k.
  - 12 panes over 4 tabs: renderer **2,318 → 1,009 MB**.

`PersistentShellBlock` (an in-pane shell) has the same shape: it renders its log inside the panel whether the panel is open or not.

## 2. The rule

**A tool body is mounted while its panel is open, and for a short linger after it closes.** Then it's unmounted. Opening the panel again builds it again.

```ts
const BODY_LINGER_MS = 1000;
const [bodyMounted, setBodyMounted] = createSignal(untrack(expanded));
let unmountTimer: ReturnType<typeof setTimeout> | undefined;
createEffect(() => {
    if (expanded()) {
        clearTimeout(unmountTimer);
        setBodyMounted(true);
    } else if (untrack(bodyMounted)) {
        clearTimeout(unmountTimer);
        unmountTimer = setTimeout(() => setBodyMounted(false), BODY_LINGER_MS);
    }
});
onCleanup(() => clearTimeout(unmountTimer));
```

**Why the original reasons for keeping it still hold**

The comments in `ToolBlock.tsx` and `_document-nodes.scss` give two reasons for keeping the body: the collapse animation, and the virtualizer's row heights. Neither is affected.

- **The collapse animation.**
  - The panel closes over 120 ms: `max-height`, margins and opacity transition to 0, and `content-visibility` flips to `hidden` at the end (`allow-discrete`).
  - The body stays mounted through the whole linger (1,000 ms), so the animation runs on the real content exactly as today.
  - Under reduced motion there is no animation, and nothing changes.
- **The virtualizer's heights.**
  - The warning is against mounting or unmounting **mid-animation**, when the row's height is changing.
  - A fully collapsed panel is `max-height: 0` with zero padding and margins, so it adds nothing to the row's height whatever it holds.
  - Removing its children after the collapse leaves the row's height unchanged, so the row ResizeObserver reports nothing new.
  - Opening is unchanged: the body mounts when `expanded()` turns true, exactly as a never-opened row's first open does today.
- **Flapping.**
  - The linger absorbs quick close/open sequences (a hold ending as the mouse re-enters, a re-pin), so the body isn't torn down and rebuilt.
  - A reopen inside the linger finds the body still mounted.

**What a reopen costs:** the body is built again, shiki highlighting included. That is the cost a never-opened history row pays on its first open today (F4 measured about 94 ms for a large diff, part of it the one-time wasm load, which a later reopen doesn't repeat). It's paid only on a user's click, not while streaming.

**What a reopen loses:** state that lives only in the body's DOM, mainly a scrolled-up position in a long log. The log follows the latest output by default, and the panel's own state (the "taller preview" toggle, pin, collapse) lives in `ToolBlock` or the document state, so it's kept.

## 3. Scope

- **`ToolBlock`:** the rule above replaces `bodyEverShown`. A row whose result was unloaded (`resultUnloaded`) still shows its fallback; nothing changes there.
- **`PersistentShellBlock`:** its log (`ChunkPreview`) follows the same rule, with the panel header kept. The shell's own output stays in the document node, so a reopened log shows everything.
- **Unchanged:**
  - `ActivityRow` already mounts its log only while expanded.
  - Rows outside the virtualizer's window are unmounted wholesale, as before.

**No kill switch.** The change is local to two components and reverts cleanly. A setting would add a code path that never runs in practice.

## 4. Tests

**`ToolBlock.test.tsx`**
- "Opening mounts the body; collapsing again keeps it" becomes: opening mounts it; collapsing keeps it through the linger (it's the same element during the linger); after the linger it's gone; reopening mounts a new one.
- A reopen inside the linger keeps the same element and cancels the unmount.
- Never-opened and open-at-mount rows: unchanged.

**`PersistentShellBlock`:** the log is absent while collapsed past the linger, and present when expanded.

The existing virtualization and agent-pane suites run unchanged.

## 5. Verification in the lab

Same workload as the root-cause report: an isolated production portable, scripted agents, forced GC. Measured with this change and the `<Key>` fix (item 1) applied:

- **4 panes, 30 rounds:** expect the renderer near the prototype's 407 MB, with agent-pane elements in the low thousands.
- **12 panes over 4 tabs, 40 rounds:** expect it near 1,009 MB, with typing (20 keys/s while streaming) no worse than before.
- **No rows moving:** `scripts/ui-screenshots/one-way-soak.mjs` (the repo's check that no visible row moves down or shows blank space and then takes it back) on four streaming panes, before and after: no new violations. This is the direct check of the measurement argument in §2.

The PR records the numbers.
