# Spec: opening a widget in a new window or a floating pane is fast every time

**Date:** 2026-10-13
**Status:** Active. Item 3 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`; it replaces that item's original scope ("shrink the spare-window pool") after the measurements below.
**Reported by the operator:** opening a widget from the widgets bar with "Open in New Window" or "Open in Floating Pane" takes a long time, the first time especially.

## 1. What happens today

Both actions are in the widget's context menu (`frontend/app/window/action-widgets-menu.ts`):

| Action | Path | Warm path | Cold path |
|---|---|---|---|
| Open in New Window | `open_new_window` (`crates/cef/src/commands/window/creation.rs`) | `promote_pool_window_for_new_window` (window pool, `POOL_TARGET_SIZE = 2`) | a new top-level window with its **own in-memory request context** (`create_isolated_request_context`) |
| Open in Floating Pane | `pane.open` with `floating: true` → srv's tear-off saga → `open_floating_pane_window` (`crates/cef/src/commands/floating_pane.rs`) | `promote_pane_pool_window` (pane pool, `PANE_POOL_TARGET_SIZE = 1`) | a new popup window on the **shared** request context |

### 1.1 Measured

Lab: an isolated production portable of `main` (9088493f5) on the operator's machine. Every widget in the widgets bar was opened twice each way, then closed. Each figure is the time from the call until the pane's content stopped changing.

| | Warm (from a pool) | Cold |
|---|---|---|
| New window | 150–650 ms (most 150–300) | 850 ms–3.1 s; Memory's first open never rendered within 20 s |
| Floating pane | **never happened**: 0 of 30 floating opens found a warm window | 500 ms–1.9 s (Toolchain settled at 3.2 s) |

### 1.2 Why the pools are empty

Both pools are evicted when commit pressure reaches **Warn**: less than 15% of the commit limit free (`memory_pressure.rs`, `memory_heartbeat.rs`). They aren't refilled until pressure has been **Normal for 180 s** (`POOL_REFILL_SETTLE`). Spawning is also refused at anything but Normal (`spawn_pool_window`, `spawn_pane_pool_window`).

**On the operator's machine (61.6 GB RAM, 77 GB commit limit) that is most of the day.** Other applications keep commit within about 11 GB of the limit.
- The live 0.59.20 instance logged 58 pressure changes and 22 window-pool evictions on 2026-10-12.
- One stretch: evicted at 09:29, empty until 19:00, refilled, evicted again 40 s later at 19:01. A window opened at 19:09 took the cold path.
- The lab instance reached Warn 20 s after it started.

**The pane pool also has a refill gap.** It respawns only:
- at startup;
- after one of its own windows is promoted (which can't happen while it's empty);
- when a pool window is destroyed while pressure is Normal;
- after the 180 s settle.

In the lab, pressure went back to Warn 2 min 20 s after reaching Normal. The pane pool was spawned once at startup and never again; the window pool recovered within seconds, because window opens and closes refill it.

### 1.3 Why a cold new window is slower than it needs to be

- A cold new window gets a unique off-the-record context with an empty in-memory cache. It downloads and compiles the whole app again: 7 of 7 JS files (4.4 MB) **from the network, none from cache**.
- A cold floating pane uses the shared, disk-backed context: **8 of 8 from cache**, and `DOMContentLoaded` at 240–300 ms after navigation.
- The reason given for the private context (`commands/mod.rs`: "each browser window needs its own renderer process to get an isolated JavaScript context") doesn't hold. Every top-level browser gets its own document, globals and module state whichever process it lands in. Floating panes already run the full app on the shared context without problems.

### 1.4 Under real memory pressure

When commit was Critical, the cold paths took 1–2 s more: a floating pane spent 1.1 s before its JS started. That is the machine paging, not something AgentMux can fix, but it's one more reason to have warm windows ready.

## 2. Changes

**1. Shed warm pools only at Critical.**
- At Warn, keep pools at their target size and stop only the overfill: the demote cap is already `POOL_TARGET_SIZE` under pressure.
- Spawning is refused only at Critical.
- **Why it's safe:** at the Warn threshold the free commit is 15% of the limit (11.5 GB here, 2.4 GB on a 16 GB-commit machine). The pools total about 165 MB (3 renderers at 52–58 MB measured), 1.4% and 7% of that headroom. Critical (5% free) still sheds them, which is the case the pressure work was built for (`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`, the 0xE0000008 crashes).

**2. Refill once pressure is below Critical for 30 s, not Normal for 180 s.**
- The 180 s settle exists because refills on Windows used to take keyboard focus while pressure flapped.
- Focus is now handed back (`[focus-restore] invisible … activated … handing back` in the logs). The gate still waits out a flap; 30 s, not 3 minutes.

**3. A cold floating open refills the pane pool.**
- After `post_create_floating_window`, call `spawn_pane_pool_window`, as a window-pool cold open already leads to a refill.
- The next floating pane is then warm even if the pool was empty.

**4. A cold new window uses the shared request context.**
- `None` instead of `create_isolated_request_context`, as floating panes and browser panes already do.
- That gives cold new windows the disk cache and V8's code cache, and removes the off-the-record profile created on the UI thread for each window.
- **What it shares:** the shared context means same-origin storage is shared with the main window (`localStorage`, cookies for the app's origin). That's exactly the situation floating panes are already in. The frontend's `localStorage` uses (15) are app-wide settings, not per-window state. `sessionStorage` stays per window.

## 3. Out of scope, own items in the plan

- **Window churn leaks memory.** In the lab, about 60 open/close cycles took AgentMux's CEF processes from 189 MB to 1.47 GB. 7 recycled "pre-warmed" windows sat at 115–124 MB each, against 52–58 MB fresh, with 13 documents per renderer. That's more than the cap of `POOL_TARGET_SIZE + 2` allows, and a document per reuse is retained.
- **A floating pane often doesn't find its parent window.** 21 of 42 floating opens logged "could not resolve parent main HWND; floater will not cascade on minimize/destroy (orphan risk)". `pane.open` from the main page doesn't carry `source_window_label`.
- **The Memory widget's first open in a new window never rendered** (nothing within 20 s); its second open took 3.1 s.

## 4. Tests

- **`memory_pressure.rs`:** `PoolRefillGate` refills after 30 s below Critical. It doesn't refill while Critical, and a return to Critical restarts the wait. `effective_pool_demote_cap` is unchanged.
- **Spawn guards:** both spawn functions refuse at Critical and proceed at Warn. The guard is a pure function, `pool_spawn_allowed(level)`, so it can be tested without windows.
- **`create_isolated_request_context`'s caller:** the cold new-window path passes no context. A unit test covers the choice, and its doc comment explains why.
- **Lab:** rerun the widget benchmark from §1.1 on the same machine, while pressure is at Warn as it usually is. Expected:
  - new windows and floating panes come from a pool on every first open (≤ 650 ms);
  - cold new windows load their JS from cache;
  - a second floating pane in a row is warm.
