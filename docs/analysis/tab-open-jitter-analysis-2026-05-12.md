# Smooth-load jitter — analysis (tabs, new windows, first boot)

**Date:** 2026-05-12 (revised)
**Author:** agent1
**Status:** analysis, not yet specced — input for prioritisation
**Revision note:** the original write-up was scoped to "tab open via `+` button". The `+` button doesn't exist — the hamburger-menu dropdown houses both **New Tab** and **New Window** entries — and the same piecemeal-mount cascade also runs at **first window boot** and **every subsequent window open**. This revision generalises the analysis to cover all three.

---

## Context

When any new top-level UI surface opens — a new tab, a new window, or the app's very first window — the content materialises in several visible stages over ~150–1000 ms instead of appearing atomically. Common manifestations: pane frames snap into place then transition; "Loading…" text flashes; the agent pane scrolls after virtualisation measures; terminal prompts reflow when the font resolves; the browser HWND lags behind its chrome; on first boot the body briefly shows the wrong theme background.

This document maps each open path, identifies the jitter sources by `file:line`, surveys the "hide until ready" patterns already in the codebase, and proposes a layered set of mitigations that apply uniformly across **tab open / new window / first window boot**.

## Companion artifact

A first-pass spec already exists: `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` on branch `agenta/spec-tab-content-reveal-gate`, tracked by [issue #774](https://github.com/agentmuxai/agentmux/issues/774). That spec proposes a **frame-budget gate** (`visibility: hidden` for ~80 ms of clean Long Tasks frames, 800 ms hard cap) as a single coarse solution. This analysis is the wider investigation that should inform whether the gate alone is sufficient or whether finer per-pane work is also needed.

---

## Triggers

There are three classes of "new surface opens" event, each with its own entry path. All three eventually converge on the same block-mount sequence (and therefore the same jitter sources), but they differ in what runs *before* the first block appears.

### A. New tab (within the current window)

The `+` button referenced in earlier writeups does not exist — `agenta/menu unify` (PR #796) moved tab actions into the hamburger menu. There are now three call sites:

1. **Hamburger menu → "New Tab"**
   `frontend/app/tab/tabbar.tsx:643-645`
   ```ts
   label: "New Tab",
   onClick: () => createTab(),
   ```
   Same `createTab()` as everywhere else — the menu chrome (per PR #796) is the only thing that changed.

2. **Agent App API: `tab.open`-style flow** (less common today)
   `frontend/app-init.ts:336` (`getApi().onAgentMuxInit(initWaveWrap)`). Non-host mode waits for the host app to emit `agentmux-init`; the event triggers `initWaveWrap` (line 428) → `initWave` (line 502). This path is for existing tabs being shown in a new window — no `CreateTab` here.

3. **Keyboard / context-menu shortcut** (Ctrl-T, tab-bar right-click → New Tab)
   Same `createTab()` entry; just different UI surface.

The common path:
- `frontend/app/store/global.ts:848-869` (`createTab`).
- Dispatches `WorkspaceService.CreateTab(ws.oid, "", true, false)`; sets default colour via `ObjectService.UpdateObjectMeta`.
- Line 863 invokes `applyTabPreset(tabId, DEFAULT_TAB_PRESET)` — **async, fire-and-forget**. The preset applies the default layout (agent left, sysinfo/swarm right) *after* `CreateTab` returns; blocks are added one by one over the next ~200–600 ms.
- Backend broadcasts `waveobj:update` events; the WS subscriber in `global.ts:250-254` hydrates the WOS cache.

### B. New window (independent top-level instance)

1. **Hamburger menu → "New Window"**
   `frontend/app/tab/tabbar.tsx:649-651`
   ```ts
   label: "New Window",
   onClick: () => getApi().openNewWindow().catch(console.error),
   ```
2. **Status-bar version click**, **Ctrl-Shift-N**, **`agentmux.exe` second invocation** — all eventually route to the same host command.

The path:
- Frontend → host IPC `open_new_window` → `agentmux-cef/src/commands/window.rs:615` (`open_new_window`) → `open_window_with_kind(state, WindowKind::FullInstance, None)` at line 664.
- Host generates a fresh `window-<uuid>` label, allocates IPC port + token, posts to the CEF UI thread (`ui_tasks::post_create_window`).
- CEF creates a top-level `WindowDelegate`, loads the frontend URL.
- Inside the new window, the frontend bootstraps from scratch (loads `index.html` again) and runs path **C** below.

Per spec, a new full instance gets its own backend sidecar process. This is the most expensive open: cold sidecar + cold CEF window + full frontend boot + tab preset.

### C. First window boot (app launch / cold start)

`frontend/bootstrap.ts:87-148` (`bootstrap()`) is the entry. The sequence:
- `initLogPipe()` → `initPerf()` → `setupCefApi()` (waits for CEF API shim).
- (New floating-pane branch in this same file branches to the floating-pane shell if `?floatingPaneId` is in the URL — see PR #811. Not relevant here.)
- `initApp()` → `initHostWave()` (`frontend/app-init.ts:114-189`) for the primary window OR `initHostNewWindow()` (`:208-280`) for a window opened from path **B**.
  - Primary window: `WindowService.GetWindow(windowId)` → `WorkspaceService.GetWorkspace(...)` → reads `workspace.activetabid` or first tab from `workspace.tabids[]`.
  - New window: `WindowService.CreateWindow(null, tearOffWsId)` creates a new workspace + tab server-side, then `GetWorkspace` retrieves the auto-created tab ID.
- Renders the App tree via `render(App, elem)` (line 588) once all WOS objects are loaded.
- Body visibility is restored from `hidden` to `visible` near the end of `initWaveWrap` (`app-init.ts:444`).

What's special about first boot: there's an additional class of jitter that doesn't exist for tab open — early FOUC, theme mis-flash, CEF Views white-flash. Some of these are already handled by the visibility guard in `bootstrap.ts` (set immediately, revealed when init completes) and the CEF white-flash work in `docs/analysis/cef-white-flash-retro.md`. The **tail end** of first-boot jitter — pane mounting, layout settling, virtualisation measurement — is identical to the tab-open path. That's the part this analysis applies to.

### Summary table

| Trigger | Frontend entry | Backend RPC | Cold cost added |
|---|---|---|---|
| Hamburger → New Tab | `tabbar.tsx:643` → `createTab()` | `WorkspaceService.CreateTab` | Tab preset application |
| Hamburger → New Window | `tabbar.tsx:649` → `getApi().openNewWindow()` | `commands::window::open_new_window` → `ui_tasks::post_create_window` | Full CEF window + frontend boot + tab preset |
| First boot | `bootstrap.ts:87` → `initApp` → `initHostWave` | `WindowService.GetWindow` (read path) | App startup + CEF white-flash window |

All three converge on the **block-mount cascade** described below — that's where most of the visible jitter originates.

---

## Mount sequence

When a tab becomes active, the UI mounts in this order:

1. **Workspace container** — `frontend/app/workspace/workspace.tsx:14-54`. All tabs are mounted; inactive tabs are hidden via `display:none` (line 37). No unmount/remount on switch.
2. **TabContent** — `frontend/app/tab/tabcontent.tsx:42-137`. Two `<Show>` gates: tab-exists (line 105) and tab-non-empty (line 109).
3. **TabModalLayer** — `frontend/app/tab/TabModalLayer.tsx:58-127`. Wraps TileLayout with `display:contents`; ResizeObserver dispatches a synthetic `window.resize` when inactive tabs collapse.
4. **TileLayout** — `frontend/layout/lib/TileLayout.win32.tsx:59-152`. `onMount` (line 65) sets a **50 ms timer** before flipping `animate: true`. Before then, transitions are disabled (`animate: false` class).
5. **Block components** — `frontend/app/block/block.tsx:281-320`. Each block waits for `blockData()`, `blockView`, and a registered `viewModel` (line 290, line 293). Gated by `<Show when={ready()}>` (line 313).
6. **View-specific renderer** — `block.tsx:73-88` (`getViewElem()`). Routes to AgentViewModel / TermViewModel / BrowserViewModel / etc. Each has its own async setup.

### Jitter-causing steps in order

| # | Step | Cause | Effect |
|---|---|---|---|
| 1 | Block `ready()` gate | `blockData()` + viewModel resolution latency | Suspense fallback "Loading…" 0-100 ms |
| 2 | TileLayout `animate` gate (50 ms) | Transitions disabled, then enabled | Panes snap into place, then animate |
| 3 | Block `onMount` measurement (`block.tsx:217-228`) | Reads `getBoundingClientRect()` post-mount | Pane content shifts a few pixels |
| 4 | Virtual list height estimator miss | `estimateNode()` vs actual height | Agent pane scroll shifts 10–200 px |
| 5 | ResizeObserver callbacks | Async fire | Late re-layout |
| 6 | Browser pane HWND visibility | IPC + host process creates HWND | Placeholder visible briefly |
| 7 | Terminal font resolution | xterm.js renders, font loads, re-measures | Prompt reflows |

---

## Identified jitter sources

### 1. Block data resolution fallback — High severity
**`frontend/app/block/block.tsx:310-319`**
```tsx
const ready = createMemo(() => !loading() && !isBlank(props.nodeModel.blockId)
    && blockData() != null && viewModel() != null);
return <Show when={ready()}>{...BlockFull...}</Show>;
```
Inside the view component, a Suspense boundary wraps the actual render (`block.tsx:274`): `<Suspense fallback={<CenteredDiv>Loading...</CenteredDiv>}>`.

**Jitter:** 0-100 ms of "Loading…" text before the agent / terminal / browser view mounts. Visible on every new tab open and every tab switch when blocks aren't cached.

### 2. TabContent "Tab Not Found" fallback — Low severity
**`frontend/app/tab/tabcontent.tsx:105-107`** — falls back to "Tab Not Found" if the Tab object hasn't propagated from backend yet. Typically <5 ms; rarely visible.

### 3. TileLayout animate gate (50 ms delay) — Very high severity
**`frontend/layout/lib/TileLayout.win32.tsx:65-71`**
```tsx
const [animate, setAnimate] = createSignal(false);
onMount(() => {
    setTimeout(() => {
        setAnimate(true);
        layoutModel.ready._set(true);
    }, 50);
});
```
Transitions are disabled for 50 ms. Panes render at final computed positions with `transform: none`. After 50 ms, `animate: true` enables transitions (`--animation-time-s`, typically 0.3 s).

**Jitter:** Nodes snap to final position (sometimes noticeably wrong because measurements aren't done), then animate. The **single most visible artifact** on tab open.

### 4. Block content offset measurement — Medium severity
**`frontend/app/block/block.tsx:217-228`** — `getBoundingClientRect()` on mount; before mount, `blockContentOffset` is `null` and CSS applies `width: calc(rect.width - 0)`. After mount, offset is set, causing a layout recalc.

**Jitter:** Pane content shifts by a few px (typically header height ~40 px). More visible in narrow panes.

### 5. Virtual list height estimator miss — High severity (agent pane)
**`frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx:91-99`**
```tsx
const virtualizer = createVirtualizer({
    estimateSize: (index) => {
        const node = partition().virtualizedNodes[index];
        return node ? estimateNode(node, props.documentState()) : 32;
    },
    // measureElement settles to actual height after paint
});
```
First paint uses estimates from `renderers.ts`. After `measureElement()` runs post-paint, actual heights are known. Mis-estimates produce scroll-offset shifts.

**Jitter:** Agent pane document scrolls up/down by accumulated estimate misses (0–200 px). Specific to the agent pane.

### 6. Browser pane HWND visibility — Medium severity
**`frontend/app/view/browser/browser-view.tsx:98-116`** — before `setPaneCreated(true)`, a placeholder `<div>` is visible. After IPC returns and the host creates the native HWND, the placeholder is hidden via `<Show when={!paneCreated()}>` (line 172). DPR conversion (`:68-77`) can produce 1–2 px misalignment.

**Jitter:** Browser chrome (address bar, loading spinner) visible briefly before HWND appears, or HWND misaligned. Timing-dependent.

### 7. Terminal prompt before font resolved — Medium severity
xterm.js renders at the default font on first paint. Backend sends font settings via RPC; when the font loads and term re-measures char cell size, the prompt line may wrap differently.

**Jitter:** Terminal lines reflow when font changes. Observable on slow font loads (Noto Sans Mono download).

### 8. Agent pane streaming buffer + virtualisation boundary — Low–Medium
**`frontend/app/view/agent/virtualization/streaming-buffer.ts`** — last N nodes are in the buffer (flex layout); oldest moves to the virtualised head when the buffer fills. If the virtualiser's estimate doesn't match the flex-layout height, scroll position shifts at the boundary.

### 9. Solid.js reactivity wave — distributed
Every component using `createEffect` with dependencies that resolve post-paint contributes. Debouncing helps but delays stabilisation. Multiple ResizeObservers (block, TabModalLayer, TileLayout drag handlers) fire asynchronously.

---

## Existing "hide until ready" patterns

### What's already in the codebase

1. **Global body visibility guard at app startup.** `frontend/app-init.ts:324-326,404-412,444` — sets `document.body.style.visibility = "hidden"` immediately, reveals when init completes (30 s safety net). Effective for FOUC at startup; **doesn't help tab switching**.
2. **Solid.js `<Show>` guards.** Component-level, per pane. No explicit ready signal — relies on data resolution.
3. **`nohover` class on tab switch.** `app-init.ts:454-460` — applies for ~100 ms after switch to suppress previous tab's hover state. Visual hack for hover flicker, not jitter.
4. **Suspense boundaries.** `block.tsx:274` etc. — renders "Loading…" text, which is itself visible jitter.
5. **TabModalLayer ResizeObserver.** Cleanup for inactive tabs going `display:none`. Indirect; doesn't address tab-open jitter.

### What's missing

- No `content-visibility` guards
- No two-pass render with `requestAnimationFrame` gate (the 50 ms `setTimeout` in TileLayout is the closest, but it's too early and ungated)
- No per-pane "ready" signal feeding a tab-level barrier
- No backend `tab:ready` event after preset application completes
- No explicit `display:none` of TabContent until its blocks have stabilised

---

## Related specs and issues

- **[Issue #774](https://github.com/agentmuxai/agentmux/issues/774)** — *Tab content reveal gate — eliminate piecemeal switch jank*. Primary tracking issue. Proposed solution: frame-budget `visibility: hidden` gate. Companion spec: `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` on `agenta/spec-tab-content-reveal-gate` (not yet merged).
- **`docs/analysis/cef-white-flash-retro.md`** — startup FOUC mitigation on Windows. Proven that `visibility: hidden → visible` is effective when timed correctly. Same pattern transfers to tab open.
- **`docs/analysis/agent-pane-rich-features-structure-2026-04-13.md`** — discusses agent-document mount/unmount side effects causing layout shifts. Relevant to the streaming buffer + virtualisation boundary.
- **`docs/specs/SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md`** and **`SPEC_TAB_GAPS_AND_NAMING_2026_04_25.md`** — tab-bar rendering specs; less directly relevant.

No existing spec for *per-pane* jitter mitigation. No issue with "jitter" or "jank" beyond #774.

---

## Tab switching vs tab opening

### Switching to an already-loaded tab
**`frontend/app/store/global.ts:880-914`** (`setActiveTab`). RPC → backend updates `Workspace.activetabid` → broadcast → `activeTabId()` signal updates → Solid fan-out → TabContent for the new tab transitions from `display:none` to `display:flex`.

- All blocks stay mounted in memory; xterm.js / virtualised lists / browser HWNDs preserved.
- Only re-renders: focus highlight, opacity fade-in, ResizeObserver fires if viewport changed.
- **Jitter profile:** Very low. Typically <100 ms.

### Opening a brand-new tab
- Tab object exists but empty until preset applies.
- First block broadcast → Block mounts, Suspense "Loading…" 0–50 ms.
- TileLayout `onMount` → 50 ms timer.
- After 50 ms → transitions enabled → panes snap then animate.
- Remaining blocks mount in order. Each adds 50–300 ms of per-pane-type jitter.
- **Jitter profile:** Very high. 300–1000 ms before visually stable.

The two paths have entirely different mitigations — switching needs almost nothing; opening needs everything.

---

## Per-pane-type render path

### Agent pane
1. Block mounts → BlockFull → AgentViewModel.
2. Initial state fetched via RPC; subscribes to agent events.
3. `AgentDocumentVirtualList` mounts (virtualisation lines 71–165).
4. Virtualiser estimates heights → renders virtualised rows + streaming buffer (flex).
5. Post-paint: `measureElement()` settles actual heights → scroll offset adjusts if estimates were wrong.

**Jitter:** Scroll shift from estimate miss. Anchor restoration. Streaming-buffer → virtualised head boundary.

### Terminal pane
1. Block mounts → TermViewModel.
2. xterm.js instance, default font (Courier New or monospace), sends size via `term_pane_create` IPC.
3. Backend spawns PTY, logs in shell.
4. xterm renders prompt at default font.
5. Backend RPC sends font settings → xterm re-measures cell size → prompt reflows.

**Jitter:** Font load latency, cell size change, terminal padding/margin re-measurement.

### Browser pane
1. Block mounts → BrowserViewModel.
2. Address bar renders immediately. Placeholder `<div>` visible.
3. `createPane()` IPC → backend spawns browser process → returns rect.
4. `setPaneCreated(true)` → `<Show>` hides placeholder → native HWND visible at final position.

**Jitter:** Placeholder sizing 0–100 ms; DPR conversion 1–2 px misalignment; HWND position lag 50–200 ms; address-bar background transition.

### Sysinfo pane
Minimal. CPU plot, memory gauge, disk info rendered synchronously. Only the block-offset measurement causes a small header-height shift.

### Subagent pane
Created on demand (click in Swarm). Inserted into TileLayout, which re-arranges siblings. Animation gate applies.

---

## Best practices and candidate solutions

### Currently in the codebase (proven)

| Pattern | Where | Effectiveness |
|---|---|---|
| Visibility guard | `app-init.ts:324-326` (startup only) | Proven by CEF white-flash fix |
| 50 ms settle delay | `TileLayout.win32.tsx:69` | Too early; produces snap-then-animate |
| Solid Suspense | `block.tsx:274` | Renders "Loading…" fallback (own jitter) |
| ResizeObserver debounce 250 ms | `block.tsx:153` | Helpful but delays stabilisation |

### Candidates not yet implemented

1. **Frame-budget visibility gate (Spec #774).** `visibility: hidden` on TabContent until ~80 ms of clean Long Tasks frames, hard cap 800 ms. Reuses `frontend/perf/observers.ts`. No per-pane coordination needed. **Lowest-cost, broadest impact.** *Status: spec drafted, not yet merged.*
2. **`content-visibility: hidden` per pane.** CSS-only; content laid out but not painted. Pros: zero JS overhead. Cons: requires explicit ready signal per pane.
3. **Two-pass render with `requestAnimationFrame` gate.** Component sets `visibility: hidden` until after onMount + rAF; reveals once layout settles. Per-component; coordination across pane types is fragile.
4. **Per-pane `onReady()` barrier.** Each ViewModel exposes a `paneReady` signal; TileLayout aggregates and only reveals when `allReady()`. Most explicit, highest engineering cost.
5. **Backend `tab:ready` event.** After all preset blocks are created and broadcast, backend emits a `tab:ready` event the frontend subscribes to. Definitive signal; requires backend changes.
6. **Push the TileLayout `animate` delay from 50 → 150-200 ms.** Single-number change. Gives block measurements time to complete before transitions enable. Cheapest possible mitigation.
7. **Audit the agent-pane height estimators** (`renderers.ts`). Reduce virtualisation scroll jitter from 100–200 px to <50 px.
8. **CSS stagger animation** on multiple blocks. Doesn't reduce jitter time; makes it feel sequential rather than chaotic. Polish, not a fix.
9. **Honour `prefers-reduced-motion`.** Already available via `prefersReducedMotionAtom` (`global.ts:109`). TileLayout should disable transitions when set. Accessibility + jitter elimination for users who want it.

---

## Summary and recommendation

**Root cause hierarchy (by severity):**

1. The TileLayout `animate` gate (50 ms) is **too early**. Panes snap then animate; this is the single most visible artifact.
2. Per-pane mounts produce 50–300 ms of additional jitter each, distributed across the tab-open window.
3. The agent-pane virtualisation estimator misses cause large scroll shifts (up to 200 px).
4. No coordination — every pane settles independently; no global "ready" signal.

**Recommended layered approach.** The same gating mechanism applies at three places, since all three converge on the same block-mount cascade:

| Where the gate goes | Triggers it protects |
|---|---|
| TabContent (per-tab) | Hamburger → New Tab; Ctrl-T; agent-API tab open; tab switch into a not-yet-stable tab |
| App root / `<App>` (per-window) | Hamburger → New Window; status-bar version click; Ctrl-Shift-N |
| `<body>` (first boot) | App startup. Existing visibility guard in `bootstrap.ts:324-326` is the right shape; the additional work is extending its "revealed when ready" condition to include block-mount stability, not just `initWaveWrap` completion. |

### Tier 1 — Ship immediately, near zero risk

- **Implement the frame-budget visibility gate from spec #774.** Place it on the **App root** (not just TabContent) so it covers tab open, new window, and the tail end of first boot in one go. Honour `prefers-reduced-motion` (disable the gate; show content immediately for users who've opted out of animations).
- **Bump TileLayout's `animate` delay from 50 → 150 ms** in `frontend/layout/lib/TileLayout.win32.tsx:69`. One-line change. Combined with the gate, transitions begin after layout has actually stabilised instead of mid-settle.
- **Extend the first-boot visibility guard.** Today `app-init.ts:444` flips `document.body.style.visibility = "visible"` when `initWaveWrap` resolves. After implementing the frame-budget gate, gate this on **both** init completion **and** the same clean-frames signal — so the body reveal includes the first tab's content stabilising, not just data loading. Hard cap stays at 800 ms (matching the gate) plus the existing 30 s safety net.

These three together should eliminate the visible snap-then-animate artifact for >90 % of cases across all three open paths.

### Tier 2 — Polish, separate PRs

- **Audit `estimateNode()` accuracy** in `frontend/app/view/agent/virtualization/renderers.ts`. Tune per node type. Reduces agent-pane scroll jitter significantly.
- **Browser pane:** investigate DPR misalignment in `browser-view.tsx:68-77`; pre-create the HWND on `paneCreated` flip to avoid the placeholder being briefly visible.
- **Terminal pane:** preload Noto Sans Mono via `<link rel="preload">` so the font-resolution reflow happens before the user notices.
- **New-window cold start:** the CEF white-flash work (`docs/analysis/cef-white-flash-retro.md`) covers most of the first-paint flash, but new full-instance windows still pay the full preset cost. Consider preloading the default-tab preset shape so block mounting can start before the first WaveObj broadcast round-trips.

### Tier 3 — Architecture, only if Tier 1 isn't enough

- **Per-pane `onReady()` barrier** wired into TileLayout's reveal logic. Each ViewModel exposes a ready signal; TileLayout reveals only when `allReady()`. Higher engineering cost; only justified if the frame-budget gate is consistently too coarse.
- **Backend `tab:ready` event** for full preset-aware reveal. The backend knows when all preset blocks have been created and their initial state broadcast; emitting a single `tab:ready` event eliminates the guesswork. Same justification.

The frame-budget gate (Tier 1) is already specced. **Recommended sequence:**
1. Merge spec #774's gate, scoped to App root (not just TabContent).
2. Implement Tier 1's three changes together — they're complementary and shouldn't be split.
3. Re-measure. With the gate in place, several per-pane jitter sources will become invisible — drop those from Tier 2.
4. Pick up only the Tier 2 items that remain user-visible.

---

## Open questions

1. Are there pane types not covered above (e.g. swarm, code preview, sysinfo subviews) that introduce additional jitter classes? Worth a focused review.
2. Does the frame-budget gate's 800 ms hard cap line up with real-world worst-case opens for all three paths? **New windows** (which include CEF startup + frontend boot + preset) may need a higher cap than the tab-open case; the per-trigger cap may need to differ.
3. Could the gate introduce latency-amplification effects (user clicks "New Tab" and feels a 200 ms blank screen rather than a 200 ms partial render)? UX trade-off worth testing — perceptions of "stalled" vs "loading" differ.
4. For first-boot, the existing visibility guard reveals at `initWaveWrap` completion. Extending it to wait for block-mount stability adds time-to-first-paint. Does this break the perception of "app started"? An alternative: keep the body-level reveal as-is, but apply the additional frame-budget gate at the `<App>` level inside the body — so the loading spinner shows quickly, then content replaces it atomically.

---

## File pointers

Key files referenced:
- `frontend/app/store/global.ts` — `createTab`, `setActiveTab`, `applyTabPreset`
- `frontend/app-init.ts` — init paths + visibility guards
- `frontend/app/tab/tabcontent.tsx` — TabContent Show gates
- `frontend/app/tab/TabModalLayer.tsx` — overlay observer
- `frontend/layout/lib/TileLayout.win32.tsx` — 50 ms animate gate
- `frontend/app/block/block.tsx` — block ready gate + offset measurement
- `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` — virtualiser
- `frontend/app/view/agent/virtualization/streaming-buffer.ts` — boundary
- `frontend/app/view/browser/browser-view.tsx` — HWND placeholder
- `frontend/perf/observers.ts` — Long Tasks observer (input for frame-budget gate)
- `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` (on `agenta/spec-tab-content-reveal-gate`) — first-pass spec

---

*Generated by the explore agent on 2026-05-12. Don't treat this as authoritative — file/line references should be re-verified before any implementation work since the agent-pane and tab-bar areas have been moving quickly.*
