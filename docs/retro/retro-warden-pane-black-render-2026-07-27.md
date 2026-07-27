# Retro: Warden pane renders as a solid black area

**Date:** 2026-07-27
**Severity:** Medium (one widget unusable while the state persists — no data loss, no crash)
**Area:** Renderer/compositor state, NOT Warden's own code (see §4 — Warden's DOM was proven healthy while the pane showed black)
**Status:** Symptom resolved live (pane renders again, screenshot-verified). Root cause narrowed to a sticky renderer/rasterization condition, cleared by a full page reload; the precise trigger remains unproven. Two genuinely-broken things found and fixed/flagged along the way.

---

## 1. What the user saw

> "figure out why the warden pane is rendering as just a black pane .. it used to work"

Reproduced live in a `task dev` instance (branch `main` @ `38978e6b`, channel `dev-main-a68420d74efcd632`). The black state **persisted across closing and re-creating the Warden pane** (two different block ids, `a30c3719…` then `5be72106…`, both black) but **was cleared by a full renderer page reload** (triggered incidentally by Vite full-reloads during this investigation). After the reloads, the pane rendered correctly under BOTH the old and new CSS (see §5), and could no longer be re-broken on demand.

---

## 2. Investigation method

This was diagnosed live against the running instance, in escalating steps, each eliminating a layer:

1. **Host-log sweep** — no `block-error-boundary` catches anywhere in the session: Warden never crashed. (`BlockErrorBoundary` forwards every pane crash to the host log; zero entries.)
2. **Settings/transparency audit** — the instance runs opaque (`window_transparent=0` in the renderer URL; `window:transparent` unset in its `settings.json`), ruling out the known first-paint-alpha/tile-bake issue documented in `theme.scss`.
3. **In-app DOM instrumentation** — temporary `[warden-diag]` logging added to `WardenView` (geometry, computed styles up 12 ancestors, `elementsFromPoint` at pane center). Result, while the user was seeing black: **the DOM was perfectly healthy** — `.warden-container` laid out at 1060×1042, every ancestor `visible`/`opacity: 1`/no transform/no filter, and `elementsFromPoint` at the pane's center returned `.warden-pane` itself as the topmost DOM element. Nothing in the DOM was covering it, and the content genuinely existed.
4. **Native HWND enumeration** (PowerShell `EnumWindows`/`EnumChildWindows` against the host PID) — the visible window contains only its own `Chrome_RenderWidgetHostHWND` + `Intermediate D3D Window`; all pooled windows (`floating-pool-*`, `window-pool-*`) are parked far off-screen at (-25600,…)/(-26214,…) and invisible. **No native surface covers the pane** — ruling out the browser-pane/pane-pool airspace theory.
5. **Pixel capture** (`PrintWindow` with `PW_RENDERFULLCONTENT`) — after the investigation's incidental page reloads, the pane **renders correctly**: header, Host section, LAN section, Internet section all painted. Screenshot-verified twice (with and without the diagnostics code).

## 3. Falsification test — the first hypothesis was WRONG

The initially-promising lead: at the moment the first Warden block was created (03:59:07), the host log recorded a burst of **18 consecutive** `ResizeObserver loop completed with undelivered notifications` errors within ~300ms — the only uncaught error type in the whole session, appearing nowhere else. `.warden-pane` was also the only pane in the codebase combining `container-type: inline-size` with `overflow: auto` on the same element (every sibling — Armory, Settings, agent setup modal — uses a dedicated non-scrolling wrapper, a convention `armory-view.scss` explicitly documents; `.agent-view` uses `overflow: hidden`). Scrollbar↔container-query feedback is a textbook resize-loop trigger.

A wrapper fix was applied and the pane rendered — but to prove causality rather than assume it, the fix was then **reverted to the original broken CSS and the window reloaded: the pane still rendered correctly.** The CSS combination is therefore NOT the root cause of the black pane. (It IS a real defect: with the wrapper in place, the ResizeObserver bursts drop from 18-per-creation to zero. The wrapper fix is kept as convention/hygiene hardening, with comments that are explicit about what it does and doesn't fix.)

## 4. Where the evidence actually points

Assembled facts:

- The black state **survives pane destruction and re-creation** (fresh block id, fresh SolidJS component tree, fresh DOM subtree — still black), so it is not per-component state.
- While black, the pane's DOM is **fully laid out, visible, and topmost** — so the failure is strictly in turning that DOM into pixels: rasterization/compositing.
- A **full renderer page reload clears it** — consistent with discarding the renderer's compositor/tile state wholesale.
- No native window covers the area; no GPU-process crash entries in the host log.
- Context at the time it broke: the machine was under **severe commit pressure** — `mem_attribution` shows 68–75GB used of a 78.6GB commit limit through the session, and the in-app memory-pressure banner (`memory-pressure-banner--warn`) was actively mounted. This repo has an **open GPU-memory investigation** (#2218, `SPEC_GPU_MEMORY_TRACING_SCAFFOLDING_2026_07_24.md` landed dev-only GPU trace commands for exactly this class of problem), and `SPEC_CEF_SANDBOX_2026_06_20.md` already documents "browser panes go black" as a known failure mode when GPU resources misbehave.

The best-supported conclusion: **a Chromium compositor/raster failure — most plausibly GPU-memory-pressure-induced — left the tile(s) for that layout region un-rasterized (black), and the condition was sticky at the renderer level until a full reload rebuilt the compositor tree.** Warden was the victim, not the culprit: it happened to be the newly-created layer when resources were exhausted. This also cleanly explains "it used to work" — nothing in Warden changed since June 24; what changed was the machine/GPU-memory conditions (and possibly ambient GPU memory cost from the recent feature growth that #2218 is already tracking).

Not proven: a deterministic reproduction. The state could not be re-triggered on demand once cleared, so this stops at "strongly indicated" rather than "reproduced under controlled conditions."

## 5. Fixes and findings shipped from this investigation

1. **`warden.tsx`/`warden.scss` — container-query context moved to a dedicated non-scrolling `.warden-container` wrapper** (matching the documented Armory/Settings convention). Not the black-pane cause (§3), but it eliminates a real, measured ResizeObserver loop burst (18→0 per pane creation) and removes the codebase's only remaining `container-type`+`overflow: auto` same-element combination.
2. **Found in passing and fixed — Warden's LAN section has 401'd since the day it shipped.** Visible in the working pane's LAN section: `⚠ Error: warden: GET /api/lan-instances → 401`. `warden.tsx`'s comment claimed the route is public ("no auth required") and sent no auth header — but `/api/lan-instances` sits in `authed_routes` (`server/mod.rs`), and the 2026-05-11 security audit (which removed all unauthenticated localhost routes, enforced by `auth_middleware`'s header-only check) predates the LAN section's ship date (~May 25). Not a recent server regression: the comment was wrong from birth and the fetch never once succeeded. Fixed by sending `authedHeaders()` from `fetchLanPeers`, matching what the Host section always did.

## 6. Follow-ups

- **Feed this incident to the #2218 GPU-memory investigation** — a concrete, user-visible "pane rasterizes black under commit pressure, sticky until reload" data point, with the DOM-healthy/native-clear/reload-fixes evidence chain above. The new dev-only GPU trace commands (`gpu_trace.rs`) are the right tool if it recurs; capture a trace BEFORE reloading next time.
- If a pane goes black again: don't reload immediately. Run the checklist from §2 (it's fast): error-boundary grep → `elementsFromPoint` via a diag build or DevTools → native HWND enum → `PrintWindow` capture → GPU trace. The reload destroys the evidence.
