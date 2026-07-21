# Report — Armory Ctrl+Wheel Zoom (missing) + Browser Pane Zoom (not per-instance)

**Status:** Analysis only — no code written yet (per explicit request: "lets analyze and write a
report to file").
**Trigger:** user request — "we need zoom (ctrl-wheel) inside the armory, and it also needs to be
decoupled from instances of the browser pane. if I have a couple browser panes open, the zoom is
affecting them all, they must be individual."
**Verify before acting:** all file:line citations checked against `main` @ current HEAD on
2026-07-20.

---

## 1. Two genuinely separate bugs, not one root cause

These read like one feature request ("zoom needs fixing in two places") but they're structurally
unrelated:

- **Armory zoom** is a DOM/JS problem — Armory content is regular HTML rendered by SolidJS, and the
  existing per-pane zoom pipeline (block metadata → font/CSS) already reaches every pane's DOM tree.
  It's simply not wired up for this one view type.
- **Browser pane zoom** is a native-CEF problem — browser pane content is a **child HWND**, not a
  DOM element. AgentMux's JS-level wheel handler literally cannot see wheel events over it; zoom
  there is handled entirely by Chromium's own built-in page zoom, which this codebase never
  intercepts or scopes.

Fixing one does not fix the other, and (per §5) they don't share an implementation mechanism either
— they need two separate changes.

---

## 2. Issue 1 — Armory has no Ctrl+Wheel zoom at all

### 2.1 Current behavior, traced exactly

`AppZoomHandler` (`frontend/app/app.tsx:261-299`) is the single global Ctrl+Wheel listener,
attached to `window` with `{ passive: false }`. On every Ctrl/Cmd+wheel event it:

1. Calls `e.preventDefault()` unconditionally (line 270) — this is what suppresses the
   browser-native/OS zoom fallback, for every pane type, always.
2. If the target is under `.window-header` / `.status-bar` / `.block-frame-default-header`, zooms
   chrome (lines 276-279).
3. Otherwise walks up to the nearest `[data-blockid]` ancestor (line 283) and calls
   `zoomBlockIn`/`zoomBlockOut(blockId, WHEEL_STEP)` (lines 287-288).

`data-blockid` is set on **every** block frame regardless of view type
(`frontend/app/block/blockframe.tsx:880`), so Armory panes have the attribute and the handler does
fire — this part works.

The actual dead end is one level deeper, in `getBlockZoom()`
(`frontend/app/store/zoom.win32.ts:64`):

```ts
if (vt !== "term" && vt !== "agent" && vt !== "swarm" && vt !== "editor") return null;
```

`"armory"` isn't in this list. `zoomBlockIn`/`zoomBlockOut` (`zoom.win32.ts:99-109`) call
`getBlockZoom` first and return immediately when it's `null` — `setBlockZoom` (which would actually
write `term:zoom` metadata and trigger any visual change) is never reached.

**Net effect today:** Ctrl+Wheel over Armory does *nothing at all* — not zoom, not OS/browser-native
zoom either, because step 1's `preventDefault()` already suppressed that fallback before the
allow-list check ever runs. It's a true no-op, not a fallback to something else.

Armory's own SCSS (`frontend/app/view/armory/armory-view.scss:36,98`) only has plain
`overflow-y/x: auto` scroll containers. `armory-view.tsx` has zero wheel/zoom-related code. Nothing
here conflicts with adding zoom — there's simply nothing there yet.

### 2.2 This gap has a documented history

`docs/specs/per-pane-zoom-hover.md` (the original spec for the hover-target wheel-zoom design)
explicitly flagged this exact scenario as unresolved at design time (§"Edge cases"):

> **Mouse over non-terminal pane:** Ctrl+Scroll is a no-op (or could fall through to focused pane —
> TBD).

The current allow-list (`term`, `agent`, `swarm`, `editor`) reflects view types that were added to
the mechanism individually over time, one at a time, as each needed it — not a closed design
decision that deliberately excludes Armory. Armory (added later — see
`docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`) simply hasn't had this follow-up done yet. This is
the same shape of gap, not a new one.

### 2.3 What "zoom" should mean for Armory

Unlike a terminal pane, Armory has no `fontSizeAtom`-driven single scaling knob today — it's a
form/list UI (bundle editor, skill list, MCP server list, account cards), not a monospace grid.
`docs/specs/zoom-architecture.md` §3 already surveyed exactly this class of problem for **chrome**
zoom (scaling a mixed UI of text, icons, fixed-size buttons, and gaps) and recommended **CSS `zoom`
on a container** (Option A) over `calc()`-propagation (Option C, "tedious... 50+ individual property
changes") or `transform: scale()` (Option B, needs manual layout compensation). The same tradeoff
analysis applies directly to Armory's content: one `zoom: var(--armory-zoomfactor)`-equivalent rule
on Armory's root container would scale text, icons, padding, and gaps together with a single line,
versus hand-touching every hardcoded px value in `armory-view.scss` and its child components.

**Open question (flagging, not deciding):** should Armory zoom reuse the *exact same* per-block
`term:zoom` metadata mechanism (persisted, keyed by blockId, clamped 0.5–2.0 per
`zoom-architecture.md` §5) so it behaves identically to terminal/agent/swarm/editor zoom — or is a
CSS-`zoom`-based scale factor different enough in kind (scaling a form UI vs. a monospace font size)
that it warrants its own metadata key (e.g. `armory:zoom`) even while reusing the *same* wheel
dispatch/persistence plumbing? Recommend the latter: add `"armory"` to `getBlockZoom`'s allow-list
and a parallel `armory:zoom` metadata key, applied via a CSS custom property + `zoom:` rule on
Armory's root container — reuses 100% of the existing dispatch/persistence pipeline
(`AppZoomHandler` → `zoomBlockIn/Out` → `setBlockZoom` → `SetMetaCommand` RPC → blockAtom → derived
atom), only the "what does the number actually do to the DOM" step differs from terminal's
font-size path.

---

## 3. Issue 2 — Browser pane zoom is global, not per-instance

### 3.1 Why the DOM-level zoom system can't see this at all

Browser pane content renders as a **native CEF child HWND** overlaid on the SolidJS DOM (the
"airspace problem" — see the `data-pane-overlay` handling in `browser-view.tsx` and
`agentmux-cef/src/browser_pane/hwnd.rs`), not as a DOM element. `AppZoomHandler`'s `window`-level
`wheel` listener is pure DOM/JS — it cannot observe input that Windows delivers directly to a
different HWND. Confirmed at `agentmux-cef/src/browser_pane/hwnd.rs:377-395`: `WM_MOUSEWHEEL` /
`WM_MOUSEHWHEEL` messages reaching the pane's own wndproc are only **logged for diagnostics**, never
intercepted, redirected, or suppressed.

So today, Ctrl+Wheel over actual browser page content is handled **entirely by Chromium's own
built-in page-zoom** — zero AgentMux code is involved. This is a materially different bug shape than
Issue 1: there's no "allow-list to extend," there's no app-level zoom code path touching this pane
type at all yet.

### 3.2 Root cause: every browser pane's CEF browser is distinct, but they share one profile

It's tempting to assume "shared browser instance," but that's not it — each browser pane genuinely
gets its own `CefBrowser`. The actual cause is **shared `RequestContext`** (CEF's per-profile
container — cookies, cache, and critically, `HostZoomMap`, which is where Chromium stores per-host
zoom levels). Two separate pane-creation code paths both share context, for different reasons:

- **Views-mode (primary path):** `agentmux-cef/src/browser_pane/creation_views.rs:140-164`
  explicitly resolves and reuses the **parent window's own** `RequestContext`:
  ```rust
  let parent_request_context = state
      .get_browser(&window_label)
      ...
      .host()
      ...
      .request_context()
  ```
  This is a deliberate, documented tradeoff (see
  `docs/specs/pane-shares-window-request-context-linux-2026-05-13.md`) made to avoid a
  `ThemeService` observer-list crash when creating isolated per-pane contexts — **not** anything to
  do with zoom. It's a side effect: every browser pane in one window shares one profile, therefore
  one `HostZoomMap`, therefore one zoom level per host/domain across all of them.
- **Legacy child-HWND path:** `agentmux-cef/src/browser_pane/creation.rs:192` passes
  `None // request_context` — even broader, falling through to the single app-wide default context.

Chromium's `HostZoomMap` keys zoom by **host** (domain), not by browser/tab instance, when browsers
share a profile — this is standard multi-tab-browser behavior (all tabs on `github.com` in one
Chrome profile share a zoom level too) and is *working as Chromium intends*. The bug, from AgentMux's
perspective, is that **AgentMux browser panes are conceptually independent tabs/windows a user
expects to zoom independently**, not tabs within one browser's tab strip — the shared-profile
architecture (adopted for an unrelated crash-avoidance reason) accidentally imported normal-browser
tab-zoom-sharing semantics into a product where that's surprising.

### 3.3 A red herring, ruled out

`set_zoom_factor`/`get_zoom_factor` (`agentmux-cef/src/commands/window/meta.rs:19-49`, backed by
`state.zoom_factor: Mutex<f64>` — a single global scalar, `agentmux-cef/src/state.rs:545,1289`) is
**unrelated** to this bug, despite the name. It only drives the **chrome** `--zoomfactor` CSS
variable (title bar/status bar — see `zoom-architecture.md` §1) and explicitly does *not* call
`host.set_zoom_level()` — the code comment at `meta.rs:41-43` notes that doing so "deadlocks from
the IPC thread." `frontend/util/cef-api.ts:408-409` is its only caller. Worth ruling out early since
the name is misleading, but this global scalar is not why browser-pane page zoom is shared.

### 3.4 Fix directions (tradeoffs, not a decision)

**Option A — Isolated `RequestContext` per browser pane.** Directly undoes the sharing that causes
the bug. **Real risk:** this is exactly the change that was reverted/avoided to prevent the
`ThemeService` crash documented in `pane-shares-window-request-context-linux-2026-05-13.md` — that
crash needs to be understood and either fixed at its root or confirmed not to reproduce anymore
before this option is viable. Also has real cost: isolated contexts mean each browser pane gets its
own cookie jar/cache — likely **undesirable** for most uses (a user probably wants their login
session shared across browser panes, just not zoom).

**Option B — Explicit per-pane zoom applied after every navigation, bypassing `HostZoomMap`'s
host-scoping.** CEF exposes `CefBrowserHost::SetZoomLevel()`/`GetZoomLevel()` per-`CefBrowser`. If
called with a **temporary, non-persisted** zoom level (CEF distinguishes "temporary" zoom, which
does NOT get written into the shared `HostZoomMap`, from the default persisted-per-host behavior),
each pane could carry its own effective zoom independent of the shared profile. This keeps cookies/
cache/session sharing intact (avoids Option A's downside) and doesn't touch the crash-prone context
isolation at all. Needs: (a) intercepting `WM_MOUSEWHEEL`/`WM_MOUSEHWHEEL` in `hwnd.rs` instead of
only logging them (§3.1), translating Ctrl+Wheel into a zoom-level delta; (b) storing that delta
per-pane (likely in `AppState`, keyed by block/pane id, mirroring the block-metadata pattern Issue
1's terminal zoom already uses) so it survives navigation within that pane; (c) re-applying it via
`SetZoomLevel` on every `OnLoadStart`/navigation-committed callback, since CEF's temporary zoom
doesn't persist across a full navigation on its own.

**Recommendation (not decided — flagging for discussion):** Option B is very likely the right shape
— it fixes the actual reported bug (independent zoom per pane) without touching the
crash-avoidance tradeoff Option A would reopen, and without changing session-sharing behavior users
likely rely on. But it needs an implementer to confirm CEF's "temporary zoom" semantics precisely
(exact API surface, whether `cef-rs`'s binding exposes the temporary/non-persisted variant
distinctly) before committing to it — flagged rather than assumed.

**Update 2026-07-21 — Option B investigated, then superseded by Option C. Recorded here rather than
silently rewriting the history above, since the investigation itself is useful evidence for future
zoom/CEF-patch work.**

Chased Option B far enough to learn two real things, both worth recording:

1. **CEF's public API has no distinct "temporary, non-`HostZoomMap`" zoom method** — verified
   against upstream Chromium source (`chromium.googlesource.com/chromium/src/+/main/components/zoom/zoom_controller.h`).
   What it *does* have is `zoom::ZoomController::SetZoomMode(ZOOM_MODE_ISOLATED)` — a per-`WebContents`
   mode switch, after which the *existing* `SetZoomLevel`/`GetZoomLevel` (already reachable from the
   Rust binding, no patch needed for those two) become per-browser-scoped automatically. `SetZoomMode`
   itself isn't exposed through CEF's C API at all, so implementing this genuinely required a real,
   small CEF source patch (`CefBrowserHost::SetZoomIsolated()`) plus a matching Rust binding-fork
   patch. Both were written and opened for review — `agentmuxai/cef#6` (C++, includes a self-caught
   fix: the new struct field was initially inserted mid-struct, which would have silently shifted
   every subsequent field's hard-coded offset assertion in the Rust binding; moved to an append-only
   position at the end of the struct instead, matching the existing `BeginWindowDrag` precedent) and
   `AgentU-asaf/cef-rs#2` (Rust `cef-dll-sys` binding, same fix applied). **Both closed, superseded —
   left open on GitHub as a record of the investigation, not merged.**
2. **Building a patched libcef is per-platform, and Windows has no precedent at all.** The C++ source
   patch itself is plain, `#ifdef`-free Chromium code — identical on every platform as written. But
   each OS ships its own separately-compiled `libcef` binary, and `docs/cef-build/build-patched-libcef.md`
   (Linux, tested, ~3-6h/~99GB) and `docs/cef-build/build-patched-framework-macos.md` (macOS) are the
   *only* documented build paths in this repo — there is no Windows equivalent, and nothing suggests
   one has ever been produced for this project (the existing `BeginWindowDrag`/transparency patches
   are Linux+macOS only; Windows apparently ships the vanilla published `cef-dll-sys` crate because
   Windows's native title-bar drag doesn't need a CEF-level patch the way Wayland/X11 did). Attempting
   a Windows build here would have been a from-scratch, unprecedented undertaking — including
   devising a Windows-equivalent of the Linux runbook's `systemd-run`-based OOM-safety mechanism,
   which has no Windows analog — on a machine simultaneously hosting a live, in-use AgentMux instance.
   Flagged as a real safety concern before attempting it; not pursued.

**Option C — CSS-injection zoom via `ExecuteJavaScript`, entirely bypassing Chromium's native zoom
system.** AgentMux browser panes already have both pieces this needs, already exercised elsewhere in
this exact codebase: `ExecuteJavaScript` (used in `commands/drag.rs`, `events.rs`, `ipc.rs`,
`launcher_event_bridge.rs`) and a per-pane post-navigation hook, `on_load_end_browser_pane`
(`browser_pane/callbacks.rs`), which the code's own comment says fires "after every navigation."
Injecting `document.documentElement.style.zoom = "<factor>"` on a pane's own `CefFrame` — Chromium's
own CSS `zoom` property, not a `transform: scale()` approximation — never touches `HostZoomMap` or
`RequestContext` at all, so it sidesteps the cookie/session-sharing tradeoff entirely rather than
choosing a side of it, needs no native CEF patch, no libcef build on any platform, and is
identical Windows/Linux/macOS by construction (it's JS, not native code).

**Real tradeoff, stated plainly:** not pixel-identical to Chromium's own native page zoom in every
edge case, and needs re-injection handling for client-side (SPA-style) navigation that doesn't
trigger a fresh `on_load_end` — a case native `HostZoomMap` zoom doesn't have to think about, since
it persists per-origin automatically regardless of navigation type. Judged an acceptable, minor cost
against Option B's now-demonstrated build-pipeline risk and cross-platform effort. **This is the
implemented approach — see §6 below, updated accordingly.**

---

## 4. Interaction between the two fixes

None. Issue 1's fix lives entirely in `frontend/` (TS/SolidJS + existing RPC). Issue 2's fix lives
entirely in `agentmux-cef/` (Rust, native HWND/CEF APIs) with no DOM/JS involvement at all —
`AppZoomHandler` will never see browser-pane wheel events regardless of what Issue 1 does. They can
ship as fully independent PRs in either order.

---

## 5. Do these share a mechanism? (research question from the trigger, answered)

**No**, and this is worth stating plainly since it might look like it should. The existing per-block
`zoomBlockIn/Out` → `setBlockZoom` → `term:zoom`-style metadata pipeline
(`frontend/app/store/zoom.win32.ts`) is already a genuinely *per-pane-instance* mechanism — adding
Armory to its allow-list (Issue 1) is exactly reusing it as designed. But that whole pipeline is DOM/
JS-side and cannot reach browser-pane content at all (§3.1) — Issue 2 is unfixable through it no
matter how it's extended. Issue 2's implemented fix (Option C, §3.4 update) also doesn't route
through the JS zoom store — it's fully native-Rust-side (`ExecuteJavaScript` + a per-pane state map
in `agentmux-cef`), for the same reason: browser-pane content is unreachable from the frontend DOM
entirely, regardless of which native-side fix is chosen.

---

## 6. Implementation scope

1. **PR A — Armory zoom.** Add `"armory"` to `getBlockZoom`'s allow-list, add `armory:zoom` block
   metadata + a CSS `zoom` rule on Armory's root container (`armory-view.scss`), following
   `zoom-architecture.md`'s already-recommended Option A pattern. Frontend-only, low risk.
   **Shipped:** `agentmuxai/agentmux#2251`.
2. **PR B — Per-pane browser zoom, via CSS injection (Option C).** Entirely within
   `agentmux-cef/`, no frontend changes (browser-pane content is unreachable from the DOM regardless
   of native-side approach — see §5). Intercept Ctrl+`WM_MOUSEWHEEL` in `hwnd.rs` (currently
   logged-only, §3.1), maintain a per-pane zoom-factor map, inject `document.documentElement.style.zoom`
   via `ExecuteJavaScript` on wheel and re-inject on `on_load_end_browser_pane` so it survives
   navigation. No CEF patch, no libcef build, no platform-specific work.
   **Superseded/abandoned CEF-patch attempt, left open as a record:** `agentmuxai/cef#6`,
   `AgentU-asaf/cef-rs#2` — see the Option B investigation write-up above for why this path was
   dropped.

---

## 7. Sources

- `frontend/app/app.tsx:261-299` (`AppZoomHandler`)
- `frontend/app/store/zoom.win32.ts:64,99-109` (`getBlockZoom`, `zoomBlockIn/Out` allow-list gate)
- `frontend/app/block/blockframe.tsx:880` (`data-blockid` attribute)
- `frontend/app/view/armory/armory-view.tsx`, `armory-view.scss:36,98`
- `agentmux-cef/src/browser_pane/hwnd.rs:377-395` (`WM_MOUSEWHEEL`/`WM_MOUSEHWHEEL` logged-only)
- `agentmux-cef/src/browser_pane/creation_views.rs:140-164` (shared parent `RequestContext`)
- `agentmux-cef/src/browser_pane/creation.rs:192` (legacy path, `None` request_context)
- `agentmux-cef/src/commands/window/meta.rs:19-49`, `agentmux-cef/src/state.rs:545,1289`
  (`zoom_factor` — chrome-only, ruled out as unrelated)
- `frontend/util/cef-api.ts:408-409`
- `docs/specs/zoom-architecture.md` (chrome-zoom options analysis; §5 per-pane architecture reference)
- `docs/specs/per-pane-zoom-hover.md` (original hover-wheel-zoom spec; documents the
  non-terminal-pane gap as unresolved "TBD" at design time)
- `docs/specs/pane-shares-window-request-context-linux-2026-05-13.md` (why browser panes share a
  `RequestContext` — the crash-avoidance tradeoff Issue 2's Option A would need to revisit)
- `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md` (Armory pane structure, confirms it's a recent
  addition relative to the zoom allow-list's original scope)
