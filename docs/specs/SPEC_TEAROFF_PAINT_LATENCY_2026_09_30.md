# SPEC: Tear-off paint latency: show the torn-off content as fast as the window

**Date:** 2026-09-30
**Status:** active — phase 1 in the PR that adds this spec (see §4 phase 1 for what shipped and what was dropped); phases 2 and 3 not started. Written against `main` @ `49ad410b8`; spot-verify file:line citations before trusting them.
**Author:** Korp@narko
**Related:** `SPEC_TAB_CONTENT_REVEAL_GATE` (the whole-tab reveal gate and the startup splash this spec shortens), `SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` (pane-tab tear-off, `pane-tab-tearoff.ts`), `SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md` (phase 5 cross-window work touches the same monitor code), `SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md` (earlier tear-off paint work).

---

## 0. The ask

> when tearing off a pane, it takes a while to paint, much longer than chrome or other apps. can you analyze the path and make sure it is as quick as possible?

> the floating window appears fast, but the content takes a long time to load

The window is fine. The content is the problem: a torn-off pane shows the pulsing brain splash for roughly half a second, then its view still has to fetch and draw its own data. Chrome's tab tear-off has no such gap, because it moves the live tab (its renderer and its last frame) into the new window instead of rebuilding it.

## 1. Measured

From the operator's own gestures on a Windows dev build (`task dev`, 2026-09-30, host log `dev/korp-redock-test`). Times are from the release (the source's `TearOffBlock` RPC starting).

**Pane tear-off** (a Swarm pane tab, 10:09:09 and 10:11:31; both runs agree within ~30 ms):

| Step | +ms | Log line |
|---|---|---|
| `TearOffBlock` returns | 26 | `[service] workspace.TearOffBlock` |
| Pane-pool window promoted **and shown** | 30–40 | `[pane-pool] promoting pane pool window` |
| Floater renderer starts bootstrapping | 40 | `pool:pane-promote received` |
| Discovery (client, window, workspace) done | 60 | `Phase 1 complete` |
| `render(App)` done | 90–125 | `initMux SolidJS render` |
| `initMux` done | 330–350 | `TOTAL initMux: ~252ms` |
| Reveal gate settles, splash starts its 200 ms fade | 420–450 | `tab-reveal whole-tab … elapsed=87–93ms` |
| The pane's own data (Swarm list, terminal replay, agent history) | after that | view-specific |

**Window-tab tear-off** (10:09:24): the pool window shows at +42 ms, `render(App)` at +130 ms, `initMux` ends at +580 ms and the reveal settles at +660 ms.

So the user sees, in order: the window at ~40 ms, then the brain splash until ~450 ms (pane) or ~660 ms (window tab), then a 200 ms fade, then the view filling in. Everything between "shown" and "content" is the problem.

**Caveat — dev inflates part of this.** In `task dev` every dynamic `import()` is a separate Vite module fetch; a packaged build bundles them (`vite.config.ts`). §2.3's tail is partly a dev artifact. §6 requires re-measuring on a packaged build before and after.

## 2. Where the time goes

Traced in code; S = on the critical path (awaited), not S = could move off it.

### 2.1 Before the floater has anything to do (source renderer and host)

1. `dragend` → **`await sleep(50)`** before anything else (`CrossWindowDragMonitor.win32.tsx:179`). S.
2. `getCursorScreenPoint` IPC (`:204`), then **`listWindows` IPC whose result is never used** (`:212`). S.
3. Pane-tab path: `startCrossDrag` + `updateCrossDrag` IPCs (`pane-tab-tearoff.ts:86-87`) to decide tear-off vs cross-window drop. S (needed).
4. `WorkspaceService.TearOffBlock` → srv `service/tear_off.rs:29-185`: creates the workspace, tab, moves the block and sets up the layout (needed), **then** prunes the source layout, queues the source delete and auto-closes an emptied tab before responding. S; the source clean-up doesn't need to block the floater.
5. Whole-pane path: an awaited diagnostic `getPaneDebugState` IPC (`:344`). S, diagnostic only.
6. `openFloatingPane` → `promote_pane_pool_window` (`pane_pool.rs:414`): pop, `SetWindowPos` + **`ShowWindow`** (`:475-485`, the window is visible from here), relabel, emit `pool:pane-promote` (`:543`), **and immediately spawn the replacement pool window** (`:554`), a whole new renderer process starting while the floater boots. The window-tab path does the same (`window_pool.rs:1860`).

Steps 1–5 cost ~30–40 ms together in the measurements; small, but all in series before the window exists.

### 2.2 The floater's bootstrap (floater renderer)

The pools pre-load only the bundle, CSS, fonts and the promote listener (`frontend/app/init/pool.ts:84-86`, `:143-145`). **No app state is pre-loaded.** After promotion, in series (`app-init.ts`):

- `initHostNewWindow`: `GetClientData` (`:476`), `CreateWindow` (`:492`), `GetWorkspace` (`:525`).
- `initMux` (`:1089`): `loadConnStatus` (`:1117`), re-fetches client/window/tab that the previous step just fetched (`:1138`), workspace + layout (`:1153`), `GetFullConfig` (`:1178`), widget import + `LoadWidgets` (`:1188-1189`), `render(App)` (`:1196`).

Of these, only `CreateWindow`, `GetWorkspace` and the layout depend on *which* workspace was torn off. `GetClientData`, `loadConnStatus`, `GetFullConfig` and `LoadWidgets` could all run while the pool window sits warm.

### 2.3 The ~190–450 ms after render (the biggest single gap)

After `render(App)`, `initMux` **awaits four dynamic imports in series** (`app-init.ts:1202-1220`): `pane-overlay-auto`, `notification/sound`, `os-notify-bridge`, `reveal-block-events`. Nothing else is awaited there (the providers-models refresh is fire-and-forget).

`initMuxWrap`'s `finally` is what calls `scheduleRevealLift` and un-hides `body` (`app-init.ts:870-873`). So **the reveal gate's clock can't even start until those imports finish**. In between, each `await` yields, so the imports queue behind the mount work (TileLayout, Block, the view) the render kicked off. In dev each is a Vite fetch; in a packaged build they're in-bundle and should be much cheaper (to be measured, §6).

None of the four services is needed to paint the pane.

### 2.4 The reveal gate and the splash

The gate lifts once there are ≥80 ms without a long task, capped at 800 ms (`frontend/app/store/tab-reveal.ts:106,112`), then the splash fades over 200 ms (`frontend/app/init/startup-splash.ts:18`). Until then `body` is `visibility:hidden; opacity:0` (`app-init.ts:658-660`) under the brain (`index.html:20-34`). For a floater the gate protects against a piecemeal mount, which is worth keeping, but the settle only starts after §2.3, and the fade adds 200 ms of animation on top of content that is already there.

### 2.5 The pane itself is rebuilt, not moved

`MoveBlock` keeps the block and its backend process; everything on the frontend is recreated in a different renderer:
- **Terminal** (`termwrap.ts:242-419`): font load (≤1000 ms cap, normally warm), fit, then **serial** fetch of the `cache:term:full` snapshot and the `term` delta since it (`:899`, `:921`), replayed into xterm, then `ControllerResync` and a rAF refit. The source only saves the snapshot every 5 s when idle (`:942-958`), so the delta to replay can be up to 5 s of output.
- **Agent pane**: history (`AgentSessionRead` → `BlockfileReadRange`, `useHistoryPagination.ts:276,366`), parse, double rAF (`agent-view.tsx:917-941`), **plus the full launch flow on mount** (`agent-view.tsx:1444` → `launch-flow.ts:131-292`: identity lookups, `ResolveCli`, `SetMeta`, `CheckCliAuth` with a 30 s timeout). The pane's cover waits for that flow to pass `checking-auth`, or 3 s (`agent-view.tsx:1378-1388`), then fades 220 ms. For a block whose controller is already running, the launch flow re-checks what is already known. Not measured; likely the largest per-pane cost for agent panes.
- **Swarm / others**: re-fetch their data on mount.

### 2.6 Contention

The replacement pool window (§2.1 step 6) starts a new renderer process in the same ~500 ms the floater is booting; the source window also relays out (mother resize, `floating_pane.rs:238`). On a busy machine these compete with the floater for CPU. Not measured separately.

## 3. Goals

1. **The torn-off content, not a splash, is on screen within ~150 ms of release** for a pane on a packaged Windows build, and within ~250 ms for a window tab. (Measured today: ~450 ms and ~660 ms to reveal, plus the 200 ms fade, plus the view's own load.)
2. **No blank or half-mounted frame** at any point: whatever is shown first is either the real content or a faithful picture of it. The reveal gate's guarantee stays.
3. **The view's own data doesn't start from zero** when the block was moved rather than created: a terminal shows its last screen immediately; an agent pane doesn't re-run its launch flow.
4. Nothing is lost: all current behaviour (focus, DPI placement, redock, splash on genuine cold start) is preserved.

## 4. Design

Ordered by payoff per risk. Phases can ship independently; each ends with a measurement (§6).

### Phase 1 — take the free time back (small, low risk)

**Shipped** (with this spec):

1. **Post-render imports stop gating the reveal.** The four services (§2.3) start loading alongside `render(App)` and install when loaded (`fireAndForget`); `initMux` no longer awaits them, so the reveal gate's clock starts right after the render. None of them paints; each subscribes to events or observes the DOM.
2. **The dead `listWindows` IPC is removed** from the Windows cross-window monitor.
3. **The 50 ms wait only applies to a drop on another window.** Its original comment (`f03066108`, the SolidJS migration) reads "Brief delay to allow native drop handlers to run first"; only a release over another AgentMux window can have any. A tear-off (no window under the cursor) no longer waits. The pane-tab path never needed it: a drop on another window is a cancel there.
4. **The pane pool refills after the floater reveals**, not at promotion: the floater reports `set_window_init_status("revealed")` when its splash starts fading, and the host spawns the replacement then (or after 2 s if no reveal is reported).
5. **A promoted pool window's splash fades in 90 ms** instead of 200 ms; the long fade stays for a cold start.
6. **Measurement:** the floater logs `[tearoff-perf] reveal Nms after promote`.

**Dropped, with reasons:**
- *`getPaneDebugState` fire-and-forget:* it's awaited on purpose, to capture state before the IPC starts; it cost ~6 ms in the measurements.
- *The duplicate `registerBackendWindow`:* it's fire-and-forget (not on the critical path), documented as idempotent, and the main-window path relies on it.
- *`TearOffBlock` responding before the source clean-up:* the whole RPC took 13–26 ms, and its response carries the source tab and workspace *after* the clean-up, which the source window applies. Not worth the ordering risk.
- *Deferring the window pool's refill:* the launcher's pool reducer also refills it (`SpawnPoolWindow` saga), so deferring it means changing the launcher's reconciliation; a separate change.

### Phase 2 — pre-bootstrap the pools (medium)

Move everything workspace-independent to pool warm-up, so promotion only does what depends on the torn-off workspace:
- in pool mode, run `GetClientData`, `initWshrpc`, `initEventSubs`, `loadConnStatus`, `GetFullConfig`, `LoadWidgets` and the post-render imports before promotion;
- on promotion: `CreateWindow` (whose response should carry the workspace, removing the separate `GetWorkspace`), layout, `render(App)`;
- stop re-fetching the objects `initHostNewWindow` already loaded (`initMux :1138`).

Risk: a warm pool renderer now holds config/connection state that can go stale while it waits. It must re-subscribe or refresh on promotion (the event subscriptions keep config current if they're live; verify). Pool windows already live for minutes, so this is a real concern, not hypothetical.

Expected: `render(App)` at ~40–60 ms after release instead of ~90–125 ms.

### Phase 3 — the content doesn't start from zero (larger, per view)

1. **Snapshot-first paint (the Chrome-like part).** At release, the source captures the pane's current pixels (the browser-pane drag catcher already does this for pages: `use-drag-snapshot.ts`), hands the image to the floater with the promote event, and the floater shows it **instead of the brain splash** until its live content reveals, then cross-fades. The user sees their content at ~40 ms, the same moment the window appears. Requirements: the snapshot is sized to the floater's DPI; it is only used for a tear-off (never a cold start); it's dropped if the live content isn't ready within the gate's cap. Capture must not delay the tear-off itself: start it at drag start (like the browser pane's prewarm) or in parallel with `TearOffBlock`.
2. **Terminal: flush before tear-off.** The source calls `processAndCacheData` for the block before `TearOffBlock`, so `cache:term:full` is current and the delta to replay is empty or tiny. The floater's first frame is then the last screen, from one fetch.
3. **Agent pane: skip the launch flow for a moved block.** A block whose controller is running (known from the block's state, or passed in the promote payload as `moved: true`) goes straight to history + stream subscribe; no `ResolveCli`/`CheckCliAuth`, and the pane cover doesn't wait for `checking-auth`. Write the agent-session snapshot in the source before tear-off so the floater takes its fast history path.
4. **Other views** (Swarm, editor, media): each gets a "moved" fast path only if measurement shows its mount is significant.

Phase 3.1 alone meets goal 1 for perception. 3.2–3.4 make the live content catch up quickly behind it.

### Not proposed

- **Moving the live renderer into the new window** (what Chrome does). AgentMux panes aren't separate renderers; a pane is DOM inside a window's renderer, and the floater is a separate CEF browser. Re-parenting DOM across processes isn't possible; re-parenting a whole CEF browser would mean one browser per pane, a different architecture.
- **Removing the reveal gate.** It exists because a piecemeal mount looked worse; snapshot-first paint (3.1) replaces what the user sees during it instead.

## 5. Same work, window tabs

Phases 1 and 2 apply unchanged to a window-tab tear-off (`initHostNewWindow` → `initMux`, `window_pool.rs` refill). Phase 3.1 applies with the whole window tab's area as the snapshot. The window-tab reveal gate has no target tab (`workspace.tsx:149-152`); it only removes the splash, so it is a candidate for the shorter fade too.

## 6. Measurement

Before and after each phase, on **both** `task dev` and a packaged portable (the dev build overstates §2.3):

- Add a `[tearoff-perf]` timeline with one clock: the source stamps `release` and passes it in the promote payload; the floater logs `promoted`, `render`, `reveal-start`, `revealed`, `content-ready` (per view: terminal first frame after replay, agent history painted, Swarm list loaded) as deltas from `release`.
- Five tear-offs each of: a terminal pane, an agent pane, a Swarm pane, a pane tab from a multi-tab pane, and a window tab. Report median and worst.
- A screen recording at 60 fps of one pane tear-off before and after, to confirm what the user actually sees frame by frame (the log can't show a blank frame).

## 7. Risks

- **Stale warm state** (phase 2): config or connection status cached in a pool renderer before promotion. Mitigation: live subscriptions plus a cheap re-check on promotion.
- **Snapshot mismatch** (phase 3.1): the picture shows a state the live content no longer has (a terminal that printed more in between). Acceptable for ≤ a few hundred ms; the cross-fade must be quick and the snapshot never shown longer than the gate cap.
- **Skipping the launch flow** (phase 3.3) for a block that isn't really running would leave an agent pane without its auth checks. Gate strictly on the block's own running state, not on the tear-off alone.
- **Skipping the 50 ms wait** (phase 1.3) on a tear-off: its stated reason (another window's drop handlers) only applies to a drop on another window, where it still runs.

## 8. Open questions

1. ~~Why the 50 ms `sleep` after `dragend` exists.~~ Answered in phase 1: for drops on another window only.
2. How much of §2.3 remains on a packaged build.
3. Whether the four post-render services can start after the first paint without missing events (e.g. the OS-notification bridge missing a toast emitted during mount).
4. For phase 3.1 on Windows: capture via the renderer (`html-to-image`, already a dependency of `TileLayout.core.tsx`) or via the host (`CaptureWindow`/`PrintWindow` on the source HWND region). The host path is faster and exact for GPU-composited content; the renderer path can't see native browser-pane pixels.
