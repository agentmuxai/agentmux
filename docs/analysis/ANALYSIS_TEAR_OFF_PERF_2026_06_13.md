# Tear-off performance & flash analysis — 2026-06-13

**Symptom (user):** Tearing off a **pane** to a new window is slower than VSCode and "goes through a bunch of flash changes" instead of staying behind the pulsating brain splash. Suspected ordering issue. Minimum ask: hide the flashes behind the brain. Optimize **both** pane and tab tear-off, but **pane is the common case** and should be prioritized.

**TL;DR**
1. **Pane tear-off and tab tear-off are two completely different mechanisms.** Pane → a **floating child window** (`floating_pane.rs`, *not* pooled). Tab → a **full instance** from the **warm window pool** (`window_pool.rs`, pre-painted). My first pass only looked at the tab/pool path — the pane path is the one the user is hitting, and it's worse.
2. **Pane tear-off is the flashiest** because it is **not pooled**: the native window is `ShowWindow`-n **blank, before CEF has painted anything** (`floating_pane.rs:629`). So the user sees: blank/dark window → CEF first paint → brain → re-bootstrap cascade → content. The tab path avoids the first two of those by pre-painting in the pool.
3. **Both paths** then share the same two problems: (a) the brain splash (`#startup-loading`) is removed **mid-mount** and decoupled from the content-reveal gate, so intermediate states show uncovered; (b) a full **re-bootstrap** (3 serial RPCs + full mount) per window — the latency VSCode avoids entirely by moving DOM in-process.

---

## 1. The two tear-off paths (the key distinction)

| | **Pane tear-off** (drag a pane/block out) | **Tab tear-off** (drag a tab out) |
|---|---|---|
| Frontend entry | `CrossWindowDragMonitor.{win32,darwin,linux}.tsx` → `performTearOff(dragType:"pane")` | `tabbar.tsx` `performTabTearOff` / `CrossWindowDragMonitor` `dragType:"tab"` |
| Backend move | `WorkspaceService.TearOffBlock` | `WorkspaceService.TearOffTab` |
| Window IPC | `open_floating_pane_window` → `floating_pane.rs` | `tearOffPoolPromote` → `window_pool.rs::promote_pool_window` |
| Window kind | **Floating CHILD window** (frameless, owned popup), chromeless `<FloatingPaneWorkspace>` (`?floatingPaneId=`) — no tab bar/widgets/status bar | **Full instance** (own taskbar entry), normal `<Workspace>` |
| **Uses warm pool?** | **No** — fresh window each time | **Yes** — 2 pre-painted off-screen windows |
| First-paint flash | **Yes** — native HWND shown blank before CEF paints | **No** — promoted window is already painted |
| Re-bootstrap | Yes (`initHostNewWindow`) | Yes (`initHostNewWindow`) |
| Spec | `SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` (#1077), `SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md` | `SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26` |

The user is describing the **pane** path. It carries every flash the tab path has **plus** a cold blank-window flash the pool was built to eliminate — and it's the more common action. So it should be fixed first.

---

## 2. How VSCode does it (the "instant" baseline)

VSCode's floating/auxiliary windows (v1.85) open the new window in the **same renderer process** via same-origin `window.open()` (Electron pairs it with a `BrowserWindow`), then **moves the editor's DOM nodes** into it. No reload, no re-bootstrap, no state re-fetch, no re-mount — that's the "nearly instant." The big cost was a refactor to make the workbench window-aware (`DOM.getWindow(node)` instead of global `window`/`document`). Limitation: iframe content (notebooks, custom editors) can't move and is blocked in aux windows.

**Why AgentMux can't copy it directly:** CEF here is **process-per-window**; there is no shared renderer/DOM to re-parent. So AgentMux pre-warms windows (tab pool) and/or re-bootstraps content into a fresh window (pane floater). The realistic wins are: pre-warm the pane path too, hide the gaps behind the brain, and shrink the re-bootstrap.

Sources:
- [VS Code introduces floating editor windows (InfoWorld)](https://www.infoworld.com/article/2335473/visual-studio-code-introduces-floating-editor-windows.html)
- [Floating windows implementation idea — microsoft/vscode #101730](https://github.com/microsoft/vscode/issues/101730)
- [Test: floating windows — microsoft/vscode #199023](https://github.com/microsoft/vscode/issues/199023)
- [Adopt DOM.getWindow in terminal — microsoft/vscode #195804](https://github.com/microsoft/vscode/issues/195804)
- [Opening windows from the renderer (Electron `window.open`)](https://www.electronjs.org/docs/api/window-open)

---

## 3. Pane tear-off flow (the common, flashy path)

Frontend `CrossWindowDragMonitor.win32.tsx::performTearOff` (`dragType==="pane"`, L241-320):
1. `measureSourcePaneSize(blockId)` — snapshot the source pane size (L259).
2. `WorkspaceService.TearOffBlock(blockId, tabId, wsId, true)` — backend moves the block into a fresh workspace+tab (L263).
3. `invokeCommand("open_floating_pane_window", { pane_id, workspace_id, x, y, width, height })` (L284).
4. On success, delete the docked layout node so the pane doesn't render twice (L311-320).

Backend `agentmux-cef/src/commands/floating_pane.rs::open_floating_pane_window` → Windows `floating_pane.rs::post_create_floating_window` (L246):
5. Build URL `…?floatingPaneId=<id>&windowLabel=<lbl>&workspaceId=<ws>` (L269) — frontend routes to the chromeless `<FloatingPaneWorkspace>`.
6. `CreateFloatingWindowTask` creates a **native owned-popup HWND** (`create_owned_popup`, `CreateWindowExW`, `WS_POPUP|WS_THICKFRAME`) and **`ShowWindow(hwnd, SW_SHOWNOACTIVATE)` immediately (L629)** — *before any CEF content exists*.
7. A CEF browser is then created **into** that HWND, loads the URL, and paints.
8. Frontend bootstraps: `initApp` → `floatingPaneId` branch → `initHostNewWindow` (3 RPCs + mount).

**Flash timeline (pane):**
`blank/dark native HWND` (step 6, ~150–300 ms until CEF paints) → CEF first paint shows `index.html` (brain) → re-bootstrap RPCs (brain visible) → mount; brain removed mid-mount → bare chrome/empty pane → content reveal. Several uncovered stages — matches the report.

(macOS/Linux pane path uses `post_create_window(frameless=true)` — also a fresh, non-pooled window; secondary windows are already frameless there.)

---

## 4. Tab tear-off flow (pooled — for contrast)

`window_pool.rs`: `POOL_TARGET_SIZE = 2` windows parked off-screen at `(-32000,-32000)`, `1200×800`, URL `?pool=1`, hidden from taskbar. On tear-off, `promote_pool_window` (L484) repositions and `ShowWindow(SW_SHOW)` (~L789) a window that is **already painted** (no blank flash), emits `pool:promote { workspaceId }` (~L796), and refills the pool. The window appears instantly; the only flashes are the shared re-bootstrap + brain-ordering ones below.

---

## 5. Root cause of the flashes

### 5a. Pane-only: blank window shown before paint
`floating_pane.rs:629` shows the native HWND (`SW_SHOWNOACTIVATE`) before the CEF browser has painted. Unlike the main window (shown in `on_load_end`, i.e. *after* first paint — `client/mod.rs:1274`) and unlike pool windows (pre-painted), the floater guarantees a blank/dark window frame for the CEF cold-start duration. This is the first and most jarring flash, and it's unique to the pane path.

### 5b. Shared: brain removed mid-mount, decoupled from the reveal gate
The reveal gate `frontend/app/store/tab-reveal.ts::scheduleRevealLift()` holds the **tab content** hidden (a `tabSwitching` CSS class) until "settled" (80 ms no long-tasks, or 800 ms cap). But the brain is not tied to it:
- `initWave` removes `#startup-loading` (brain) at **`app-init.ts:922`** — *mid-mount*.
- `initWaveWrap` `finally` un-hides the **body immediately** (`app-init.ts:623`) while the gate still hides only the tab content.
- Gap between brain-removal and gate-lift: brain gone, body visible, content hidden/settling → user sees bare chrome → empty pane → piecemeal mount, **uncovered**.

So even after the cold-window flash, the brain doesn't cover the bootstrap. Both paths hit this; the floating pane shell (`<FloatingPaneWorkspace>`) shares the same `index.html` brain and the same `initHostNewWindow`/`initWaveWrap` reveal path.

### 5c. Secondary: resize/relayout under no cover
Pool windows resize from `1200×800` to the source size before show; the pane floater is created at the source size (good) but still relays out as the workspace mounts. Invisible under a persistent brain; visible today because the brain dies early.

---

## 6. Root cause of the slowness

Both paths re-bootstrap per window — the latency VSCode avoids:
- `initHostNewWindow` runs **3 serial RPC round-trips** — `GetClientData` (L371), `CreateWindow` (L383), `GetWorkspace` (L388), each `withTimeout` — before any mount.
- Then a full **mount cascade** in `initWave` (global state, layout model, per-pane init: Agent/Terminal/Browser).
- **Pane additionally** pays cold window creation (no pool): native HWND + CEF browser spawn + first paint (~150–300 ms).
- Tab pays neither window-creation nor first-paint (pool), only the re-bootstrap.

VSCode: 0 RPCs, 0 re-mount, 0 window-creation cost (reuses process) — DOM re-parent only.

---

## 7. Fix proposals (prioritized — pane first)

### P0 — Stop the pane's blank-window flash (biggest, pane-specific)
Don't let the floating HWND be visible empty. Two options:
- **(a) Show-after-paint (cheapest):** create the floating HWND **hidden**, and only `ShowWindow` it on the CEF `on_load_end` / first-paint for that browser — mirroring the main window (`client/mod.rs:1274`). Eliminates the blank-frame flash with no pooling. Requires threading the floater's label into the `on_load_end` show path so only floaters wait.
- **(b) Pre-warm a floating-pane pool (best, mirrors tabs):** keep 1–2 pre-painted frameless `?floatingPaneId=…&pool=1` windows off-screen; on tear-off, promote + reposition + show (already-painted) and send a `pool:promote`-style event carrying the workspace + pane id. This gives the pane path the same "instant, no first-paint flash" the tab path already has. More work, but it's the common action — worth it.

Recommendation: ship (a) immediately (kills the worst flash now), then build (b) for true instant.

### P0-shared — Keep the brain covering until content is settled (the explicit ask)
Applies to **both** paths. Tie the brain to the reveal gate instead of `initWave`:
- Remove the `#startup-loading` removal at `app-init.ts:922`.
- Fade/remove the brain only when `tabSwitching` flips to `false` (the gate's "settled" moment). Keep the brain on top (`z-index: 99999`) over the now-visible body until then.
- Don't un-hide the body before content is ready on the new-window path — reveal body + content together under the brain, then cross-fade the brain out.
- Safety cap via the existing `MAX_GATE_MS` / 30 s body fallback so the brain can never stick.

Risk: low (CSS/TS only). This is what makes the transition read as **brain → content** with nothing in between, for both pane and tab.

### P1 — Shrink the re-bootstrap (latency, both paths)
- Carry the workspace payload to the new window (in the floater URL / `pool:promote` event / one combined RPC) so `initHostNewWindow` does **one** round-trip instead of three serial ones. `GetClientData` is session-constant and can be cached at spawn; `CreateWindow`+`GetWorkspace` can be combined.
- Pre-import heavy modules (layout model, xterm) in the idle pool/floater renderer so promote only binds data. Measure with the existing `[startup-perf]` `tlog` lines in `initHostNewWindow`.

### P2 — Size/relayout polish
Already creating the floater at the source-pane size (good). Once P0-shared hides the bootstrap, any residual relayout is covered. No action unless visible after P0.

### P3 — Aspirational: same-process DOM transfer (VSCode model)
True "instant" needs a shared-renderer multi-window model to re-parent DOM instead of re-bootstrapping. With CEF process-per-window (and iframe/agent panes VSCode itself can't move) this is a large re-architecture. **Not recommended now** — P0 + P1 capture the perceived-speed win cheaply.

---

## 8. Recommended sequence
1. **P0(a)** — gate the floating window's `ShowWindow` on first paint (pane blank-flash gone). Verify via CDP frame capture across a pane tear-off (no blank/dark frame).
2. **P0-shared** — brain covers bootstrap until the reveal gate lifts; cross-fade. Fixes the "flashes not behind the brain" for both paths.
3. **P1** — collapse the 3 RPCs to 1 + pre-warm modules (real latency; quantify with `[startup-perf]`).
4. **P0(b)** — floating-pane pool, for true instant on the common path.
5. Revisit P3 only if still not instant.

---

## 9. Key file references
**Pane path (prioritize):**
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` — `performTearOff` `dragType:"pane"` → `TearOffBlock` + `open_floating_pane_window` (L241-320); `.darwin.tsx` / `.linux.tsx` mirror it.
- `frontend/app/drag/tear-off-pool-helper.ts` — `openTearOffWindow`, `measureSourcePaneSize`.
- `agentmux-cef/src/commands/floating_pane.rs` — `open_floating_pane_window` IPC (L81); non-Windows `post_create_window(frameless)` (L218).
- `agentmux-cef/src/floating_pane.rs` — `post_create_floating_window` (L246); `create_owned_popup` + **`ShowWindow(SW_SHOWNOACTIVATE)` L629** (blank-before-paint).
- `frontend/app/workspace/floating-pane-workspace.tsx` + App.tsx `floatingPaneId` branch — chromeless shell.

**Tab path:**
- `frontend/app/tab/tabbar.tsx` — `performTabTearOff`.
- `agentmux-cef/src/commands/window_pool.rs` — `promote_pool_window` (L484), show (~L789), `pool:promote` (~L796), refill (L806).
- `agentmux-cef/src/commands/drag.rs` — `tear_off_pool_promote` (L314), cold fallback `open_window_at_position` (L374), `tear_off_sc_move_handshake` (L509).

**Shared bootstrap / reveal / brain:**
- `frontend/app-init.ts` — body hidden (L483); pool short-circuit (L526-538); `initHostNewWindow` 3 RPCs (L371/383/388) + `initWaveWrap` (L417); `initWaveWrap` finally `scheduleRevealLift` (L622) + body un-hide (L623); **`#startup-loading` removal (L922)**.
- `frontend/app/init/pool.ts` — `isPoolMode`, `awaitPoolPromote`.
- `frontend/app/store/tab-reveal.ts` — reveal gate (SETTLE_MS 80, MAX_GATE_MS 800).
- `index.html` — `#startup-loading` brain (`visibility: visible !important; z-index: 99999`) + `startup-pulse`.
- `agentmux-cef/src/client/mod.rs` — main window shown in `on_load_end` (L1274) — the show-after-paint pattern P0(a) should reuse for floaters.

*Written 2026-06-13 by AgentX. Analysis only — no code changed.*
