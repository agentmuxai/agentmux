# Spec: Loading-Brain Indicator for Browser Panes (Messenger Widgets)

**Date:** 2026-07-11
**Author:** AgentY
**Type:** Analysis + implementation-ready design. No code shipped yet.
**Purpose:** The 5 messenger widgets (Discord, Slack, Telegram, WhatsApp, Teams) — each just a `browser` pane pointed at a pinned URL — show a blank/white native window while the external site's JS bundle boots, which reads as broken rather than loading. Show the existing pulsating `BrainSpinner` (already used elsewhere in the app for pane-loading states) over that gap instead.

---

## 1. The 5 messengers, confirmed

`agentmux-srv/src/config/widgets.json` — all five are the exact same shape, differing only in URL/branding:

| Widget key | Label | URL | Notes |
|---|---|---|---|
| `defwidget@discord` | Discord | `https://discord.com/app` | "background bridge" |
| `defwidget@slack` | Slack | `https://app.slack.com/` | |
| `defwidget@telegram` | Telegram | `https://web.telegram.org/` | |
| `defwidget@whatsapp` | WhatsApp | `https://web.whatsapp.com/` | |
| `defwidget@teams` | Teams | `https://teams.microsoft.com/` | "bridge Phase 3" |

Each entry is `blockdef.meta = {view: "browser", url: "<...>", "browser:show_controls": false}` — per `CLAUDE.md`'s own "Not widgets" note, this is the existing "browser preset" pattern: no dedicated messenger view type exists, they're all just the shared `browser` pane. **This means the fix below is not messenger-specific code — it lives entirely in the shared Browser pane view/model, and every browser pane (messenger or ad hoc) gets it identically.** Messengers are the flagship motivating case (heavy client-side SPAs — Discord/Slack/Teams all ship large JS bundles that visibly take a beat to boot) but not a special case.

---

## 2. Root cause: two real bugs, not just "no indicator exists"

The pulsating brain (`frontend/app/element/BrainSpinner.tsx`, extracted per `docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md`) already covers **stage-one blank** — the gap before a pane's Solid component/`viewModel` resolves at all (`block.tsx`'s `<Suspense fallback={<BrainSpinner/>}>` and `<Show when={ready()} fallback={<BrainSpinner/>}>`). For a browser pane this resolves fast; it is not the gap the user is describing.

**Stage two — the real gap — is unindicated:** once the `BrowserViewComponent` mounts, `browser-view.tsx`'s `createPane()` calls `browser_pane_create` and CEF creates a native Win32 child HWND (`WindowInfo::default().set_as_child(...)`, `agentmux-cef/src/browser_pane/creation.rs:182`) immediately, visible by default. `.browser-placeholder` (the DOM div the HWND is positioned over) renders **nothing** once a URL exists — its only content is an "enter a URL" empty-state, gated on `!model.urlAtom() && !paneCreated()`, i.e. it never shows for the messenger widgets (they always have a URL). So the moment the HWND exists, Chromium's own default blank page paints through — for however long Discord/Slack/etc. take to actually load and render their app, with zero AgentMux-level indicator.

**A `loading` state already exists in the reducer — but is wired wrong, twice:**

`frontend/app/store/browser-pane-state/reducer.ts` already has full, tested, correct machinery for this: `Navigate` sets `loading: true` (line ~540), `TabLoadingChanged` (line 376) atomically sets `{loading, canGoBack, canGoForward}` from CEF's real values, `LoadFinished`/`LoadFailed` clear it. `BrowserViewModel.loadingAtom` (`browser-model.ts:117-119`) projects it. This is exactly the right foundation — **but two things break it before it ever reaches the UI:**

1. **`browser-view.tsx`'s `createPane()` calls `model.onLoad()` immediately after `browser_pane_create` resolves** (line 212) — i.e. the instant the *HWND exists*, not when the *page has loaded*. `onLoad()` dispatches `LoadFinished`, which clears `loading` back to `false` within the same tick `Navigate` set it `true`.
2. **`browser-model.ts`'s `browser-pane-nav-state` listener unconditionally dispatches `LoadFinished` on every event** (line 398), regardless of which CEF callback produced it. `on_loading_state_change_browser_pane` (`agentmux-cef/src/browser_pane/callbacks.rs:249`) fires on "navigation start, navigation commit, and after back/forward" per its own doc comment — not just completion — so even a mid-navigation event clears `loading`.
3. **`TabLoadingChanged` — the command built for exactly this — is never dispatched from anywhere in production code** (confirmed: only referenced in `reducer.ts` itself, `types.ts`, and tests). CEF's real `is_loading` boolean is available at the FFI boundary (`agentmux-cef/src/client/navigation.rs:24`, `on_loading_state_change`) but is explicitly discarded as `_is_loading` before it ever reaches `on_loading_state_change_browser_pane` (which only forwards `can_go_back`/`can_go_forward`).

Net: `loadingAtom` is real, tested, reducer-correct state that has never actually reflected reality end-to-end, and is not consumed by `browser-view.tsx`'s JSX at all today (grepped — zero references). This isn't a "build a loading indicator from scratch" task; it's "finish wiring machinery that's already half-built."

---

## 3. The z-order piece already has a proven, reusable answer

A native browser-pane HWND paints above DOM regardless of CSS z-index on Windows (the "airspace problem," `docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`). The existing fix for this — used today by modals, and (via auto-discovery) menus/tooltips/popovers — is `frontend/app/platform/pane-overlay-auto.ts`: any DOM element tagged `data-pane-overlay` is automatically measured (`MutationObserver` + per-element `ResizeObserver`) and the union of all such rects is punched as a transparent hole through the relevant pane HWND via `SetWindowRgn` (`pane-overlay.ts` → `browser_panes_set_overlay_clip` IPC). Visibility is already opacity-aware: `isOverlayElementVisible()` treats `opacity: 0` as "not clipping" (`pane-overlay-auto.ts:56-63`).

This means **no new native-window-visibility plumbing is needed.** `BrainSpinner`'s existing `fading` prop already sets `opacity: 0` via a `200ms ease-out` transition (`BrainSpinner.scss:25-29`) — that alone is enough to make `pane-overlay-auto.ts` stop clipping the moment the fade completes, automatically restoring full HWND visibility, no coordination code required between "fade the spinner" and "un-punch the hole." Render `<BrainSpinner data-pane-overlay/>` absolutely positioned to fill `.browser-placeholder` (same rect the HWND itself is sized against, via the same `getBoundingClientRect()`), and the existing systems do the rest.

**One caveat worth flagging, not silently assumed away:** every current `data-pane-overlay` consumer (modals, menus, tooltips, popovers) is a small rect. This would be the first full-pane-size (100% coverage) overlay. Nothing in `pane-overlay.ts`/`pane-overlay-auto.ts` caps overlay size, and the mechanism is just a rect union either way — but it's untested at that scale and should be explicitly verified during implementation (does a full-pane `SetWindowRgn(RGN_DIFF)` behave identically to a partial one performance/correctness-wise, given the pane is also being actively resized/positioned by the same-frequency `syncPosition` 200ms poll during this exact window?).

---

## 4. Design

### 4.1 Backend: thread `is_loading` through, stop the blanket `LoadFinished`

`agentmux-cef/src/client/navigation.rs`, `on_loading_state_change`: stop discarding the parameter.

```rust
pub(crate) fn on_loading_state_change(
    &mut self,
    browser: Option<&mut Browser>,
    is_loading: i32,          // was: _is_loading
    can_go_back: i32,
    can_go_forward: i32,
) {
    if !self.is_browser_pane { return; }
    if let Some(b) = browser.as_deref() {
        crate::browser_pane::callbacks::on_loading_state_change_browser_pane(
            &self.state, b, is_loading != 0, can_go_back != 0, can_go_forward != 0,
        );
    }
}
```

`agentmux-cef/src/browser_pane/callbacks.rs`, `on_loading_state_change_browser_pane`: accept and forward `is_loading` in the existing `browser-pane-nav-state` payload (additive field — the frontend already treats unknown/missing fields as no-op per the existing `url_only` convention, so this can't break the address-bar-sync consumer):

```rust
pub fn on_loading_state_change_browser_pane(
    state: &Arc<AppState>, browser: &Browser,
    is_loading: bool, can_go_back: bool, can_go_forward: bool,
) {
    // ... existing block_id/url resolution unchanged ...
    crate::events::emit_event_from_state(state, "browser-pane-nav-state", &serde_json::json!({
        "block_id": block_id, "url": url,
        "can_go_back": can_go_back, "can_go_forward": can_go_forward,
        "is_loading": is_loading,   // NEW
    }));
}
```

No change needed to `on_load_end_browser_pane`'s emit (`url_only: true`) — it stays the address-bar-redirect-catcher it already is; §4.2 below stops treating it as a loading-finished signal.

### 4.2 Frontend model: fix the two premature-`LoadFinished` sites

`browser-model.ts`'s `browser-pane-nav-state` listener — replace the unconditional `LoadFinished` dispatch:

```ts
// was: this._dispatch({ type: "LoadFinished" }, "nav-state");
if (payload.is_loading !== undefined) {
    this._dispatch(
        { type: "TabLoadingChanged", tabId: <active tab id>, loading: payload.is_loading,
          canGoBack: payload.can_go_back ?? active.canGoBack, canGoForward: payload.can_go_forward ?? active.canGoForward },
        "nav-state",
    );
}
```

(Exact tab-id plumbing depends on Phase 1A's active-tab shape already in the reducer — `TabLoadingChanged` requires a `tabId`, so the dispatch needs the active tab's id, available the same way `HistoryUpdated`'s handling already resolves it a few lines up.)

`browser-view.tsx`'s `createPane()` (line 212): **remove the `model.onLoad()` call.** It was firing at "HWND exists" time under a name that reads as "page loaded" — nothing else in this file needs it once `TabLoadingChanged` is correctly wired, and removing it is what stops `loading` from clearing itself within the same tick it was set.

### 4.3 Frontend view: render the spinner

`browser-view.tsx`, inside `.browser-placeholder`, alongside the existing empty-state `<Show>`:

```tsx
<Show when={model.loadingAtom()}>
    <BrainSpinner
        data-pane-overlay
        class="browser-loading-overlay"
        fading={!model.loadingAtom() /* see below */}
    />
</Show>
```

Actual fade sequencing needs a local signal (mirroring how `block.tsx`'s callers elsewhere handle `fading`+unmount-after-transition) rather than deriving `fading` from the same atom that gates the `<Show>` — the existing `BrainSpinner` contract is "caller owns unmounting after the transition ends" (per its own doc comment), so this needs a short-lived `[stillMounted, setStillMounted]` signal: set `true` when `loadingAtom` flips `true`→ mount; on `loadingAtom` flipping to `false`, keep mounted with `fading={true}` for ~220ms (matching the CSS transition), then unmount. This is the same pattern `block.tsx`'s existing `BrainSpinner` consumers don't need (they use Suspense/Show unmount directly, no fade) but the tab-reveal-gate/startup-splash precedent (`tab-reveal.ts`, `startup-splash.ts`) already establishes for this exact "hold state briefly for the fade" shape.

`.browser-loading-overlay` CSS: `position: absolute; inset: 0;` inside `.browser-placeholder` (which needs `position: relative` if it isn't already) — must exactly match the placeholder's box so the auto-clip rect lines up with the HWND underneath pixel-for-pixel.

### 4.4 Reload / back-forward — re-arm correctly, without SPA-navigation false triggers

`reload()` and `goBack()`/`goForward()` in `browser-model.ts` already dispatch `LoadStarted` (sets `loading: true`) before firing their IPC — that path is unaffected by this change and should correctly show the spinner again on an explicit reload/back/forward, matching §4.2's now-correct clear-on-`TabLoadingChanged(loading:false)`.

**Deliberately not a concern:** in-app SPA navigation (Discord/Slack channel switches via History API `pushState`) does not trigger CEF's `on_loading_state_change`/`on_load_end` the way a real top-level navigation does — those are LoadHandler callbacks tied to the navigation controller's actual document loads, not client-side routing. So the spinner will not flicker on every channel switch inside an already-loaded messenger. **Worth flagging as a real, if minor, UX cost rather than ignoring:** a multi-hop OAuth/login redirect (Slack/Discord/Teams login flows commonly bounce through 2-3 domains) *is* a sequence of real top-level navigations, so the spinner may show/hide/show again briefly during login. Acceptable — arguably more honest than the current blank-white gap — but call it out for whoever implements this, since "flickery during login" is the kind of thing that reads as a regression if unexpected.

### 4.5 Cross-platform note

The native-child-HWND path (`browser_pane/creation.rs`, `SetWindowRgn`) is Windows-specific — per prior research in this codebase (`docs/specs/SPEC_EXTERNAL_APP_DRIVING_BLENDER_2026_07_03.md` §4.3), macOS/Wayland use CEF's Views-overlay compositing instead of a real child window, where the "airspace problem" this spec's z-order piece (§3) works around may not exist in the same form (no separate native window to be clipped-behind in the first place). **This spec's §4.1/§4.2 (the loading-state plumbing) is platform-agnostic — CEF's LoadHandler callbacks fire identically everywhere.** §4.3's `data-pane-overlay` z-order mechanism needs verification on macOS/Linux specifically: either it already works there for the existing modal/menu overlays (in which case this reuses cleanly), or the Views-overlay path never needed clipping to begin with (in which case a full-pane spinner may just render correctly via normal CSS z-index with no special handling needed at all on those platforms). Flagged as an implementation-time check, not resolved here.

---

## 5. Scope / non-goals

- **No new component.** `BrainSpinner` is reused as-is; no new visual asset.
- **No new backend command/RPC.** `is_loading` rides the existing `browser-pane-nav-state` event as one additive field.
- **No messenger-specific code anywhere** — the fix is entirely in the shared `browser` pane view/model/reducer, benefiting every browser pane uniformly (manually-opened panes typing a URL get the same fix for free, since it's the identical `navigate()`/`createPane()` code path).
- **Does not touch `TabLoadingChanged`'s reducer logic** — it's already correct and tested; this spec's only reducer-adjacent change is finally dispatching it from somewhere real.
- **Out of scope:** per-tab loading indicators for Phase 1B's not-yet-shipped tab strip (the reducer already tracks `loading` per-tab; this spec only wires the *active* tab's value into the one-pane-one-spinner view that exists today). If/when the tab strip ships, it's a natural, additive consumer of the same now-correct `loading` field per tab.

## 6. Open questions

1. **§4.3's fade-hold pattern** — worth factoring into a small reusable hook (`useFadingSpinner(showWhen: Accessor<boolean>)`) if the same "hold mounted briefly for CSS fade-out, then unmount" shape recurs, rather than hand-rolling it once here. Not blocking — can ship inline first, extract on the second use site.
2. **§4.5's cross-platform verification** — needs a real macOS/Linux `task dev` check before shipping, not just Windows.
3. **§3's full-pane-size overlay** — first use of `data-pane-overlay` at 100% pane coverage; worth a manual perf/correctness pass (resize-while-loading, multiple messenger panes open simultaneously) before considering this done, not just a code read.
4. Should the spinner also show during `reload()`, or only on true first-load? §4.4 already answers yes by construction (reload already dispatches `LoadStarted`) — flagging only in case product wants reload to feel "instant" via some other treatment instead.

## 7. References

- `agentmux-srv/src/config/widgets.json` (lines 121-195) — the 5 messenger widget definitions.
- `frontend/app/element/BrainSpinner.tsx`/`.scss` — the reusable spinner component.
- `docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md` — the report that produced `BrainSpinner` for the agent-pane case; this spec is its Browser-pane sibling.
- `frontend/app/view/browser/browser-view.tsx`, `browser-model.ts` — the shared Browser pane view/model.
- `frontend/app/store/browser-pane-state/reducer.ts`, `types.ts` — the already-correct, already-tested `loading`/`TabLoadingChanged` reducer machinery this spec finally wires up.
- `agentmux-cef/src/client/navigation.rs`, `agentmux-cef/src/browser_pane/callbacks.rs`, `agentmux-cef/src/browser_pane/creation.rs` — CEF LoadHandler callbacks and native pane creation.
- `frontend/app/platform/pane-overlay.ts`, `pane-overlay-auto.ts`, `docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md` — the existing `SetWindowRgn`-based DOM-over-native-pane mechanism this spec reuses for z-order, unmodified.
