# Spec: Browser pane default URL + in-pane popup redirect

**Date:** 2026-04-21
**Status:** Draft (implementation-ready)
**Scope:** Two small, independent changes to the embedded browser pane.

---

## Problem

1. **Default URL.** A freshly-created browser pane loads about:blank because the widget definition ships `url: ""`. First-time users see a blank page with no signposting.
2. **Popup explosion.** Clicking a link with `target="_blank"` (or any `window.open()` from the page) spawns a CEF-level top-level window outside the AgentMux host. The new window is orphaned — it has its own chrome, can't be embedded, and survives after the agent/workspace is gone. Every "Read more" link in the wild opens a rogue window.

Both are low-friction to fix and together make the browser pane usable without surprises.

---

## Change 1 — Default URL

### Desired behaviour

Blank-spawned browser panes load **`https://agentmux.ai`** unless the widget caller explicitly passes a URL. Callers who want blank still can by passing `"about:blank"` explicitly.

### Files

- `agentmux-srv/src/config/widgets.json` — `defwidget@browser.blockdef.meta.url` flips from `""` to `"https://agentmux.ai"`.

### Frontend fallback

`frontend/app/view/browser/browser-model.ts:78` already reads `meta.url` and calls `navigate()` when truthy. An empty string is falsy, so historically these panes do nothing until the user types a URL. With the default filled in, `navigate()` runs on mount.

One defensive tweak: if a block is created with no `url` meta at all (not just empty string), fall back to the default URL constant rather than leaving the pane blank. Single source of truth in the frontend:

```ts
// frontend/app/view/browser/browser-model.ts
const DEFAULT_BROWSER_URL = "https://agentmux.ai";

// In the constructor, replace:
if (meta?.["url"]) {
    this.navigate(meta["url"] as string);
}
// With:
const initialUrl = (meta?.["url"] as string | undefined) || DEFAULT_BROWSER_URL;
this.navigate(initialUrl);
```

### Non-goals

- Making the default URL user-configurable via settings. Defer until someone asks.
- Branding or onboarding content on the default landing page itself — that's `https://agentmux.ai`'s job, not ours.

### Test plan

- [ ] Fresh portable: widget tray → drag/click browser widget → pane opens to `https://agentmux.ai`.
- [ ] Explicitly passing `{ meta: { view: "browser", url: "about:blank" } }` through the API still lands on `about:blank`.
- [ ] `{ meta: { view: "browser" } }` with no `url` → fallback to `https://agentmux.ai`.

---

## Change 2 — Popup redirect (in-pane)

### Desired behaviour

When code inside the embedded page calls `window.open(url, "_blank")` or a user clicks a link with `target="_blank"`, **navigate the current pane to that URL** instead of spawning a new CEF top-level window. "Open in new tab" in the page's own UI becomes "navigate this pane" — which matches what AgentMux users expect (the agent, not the page, decides how tabs work).

### How CEF hands us the hook

CEF's `LifeSpanHandler::on_before_popup` fires before the new browser is created. Return `true` to cancel; the parent browser's current frame is still valid, so we can call `browser.main_frame().load_url(target_url)` to navigate the current pane to where the popup would have gone.

The signature (cef-rs):

```rust
fn on_before_popup(
    &self,
    browser: Option<&mut Browser>,
    frame: Option<&mut Frame>,
    popup_id: i32,
    target_url: Option<&CefString>,
    target_frame_name: Option<&CefString>,
    target_disposition: WindowOpenDisposition,
    user_gesture: i32,
    popup_features: &PopupFeatures,
    window_info: &mut WindowInfo,
    client: &mut Option<Client>,
    settings: &mut BrowserSettings,
    extra_info: &mut Option<DictionaryValue>,
    no_javascript_access: &mut i32,
) -> i32;  // non-zero = cancel
```

### Files

- `agentmux-cef/src/client.rs`:
  - Add `fn on_before_popup(...)` to `impl AgentMuxHandler`. Body:
    ```rust
    // Intercept target="_blank" / window.open() so embedded browser
    // panes never spawn rogue CEF top-level windows. Instead navigate
    // the current pane to the target URL. Matches the UX expectation
    // that the agent/workspace owns window management, not the page.
    //
    // Returning non-zero cancels the popup creation entirely. Safe to
    // apply to MAIN client too — the main window's frontend owns
    // its own link-routing; any top-level popup it wants would route
    // through `app-api.pane.open` or similar, never through window.open.
    fn on_before_popup(
        &mut self,
        browser: Option<&mut Browser>,
        _frame: Option<&mut Frame>,
        _popup_id: i32,
        target_url: Option<&CefString>,
        _target_frame_name: Option<&CefString>,
        _target_disposition: WindowOpenDisposition,
        _user_gesture: i32,
        _popup_features: &PopupFeatures,
        _window_info: &mut WindowInfo,
        _client: &mut Option<Client>,
        _settings: &mut BrowserSettings,
        _extra_info: &mut Option<DictionaryValue>,
        _no_javascript_access: &mut i32,
    ) -> i32 {
        let url = target_url.map(|s| s.to_string()).unwrap_or_default();
        if url.is_empty() {
            return 1;  // cancel; nothing useful to navigate to
        }
        if let Some(b) = browser {
            if let Some(mut frame) = b.main_frame() {
                frame.load_url(Some(&CefString::from(url.as_str())));
            }
        }
        tracing::info!(
            is_pane = %self.is_pane,
            url = %url,
            "popup intercepted — navigated current frame",
        );
        1  // cancel the popup
    }
    ```
  - Wire it into the `wrap_life_span_handler!` block. The macro currently lists `on_after_created` / `do_close` / `on_before_close`; add `on_before_popup`.

### Devtools exception

The existing DevTools path goes through `on_popup_browser_view_created` (Views-runtime popup) which is a different hook entirely — that stays unchanged. `on_before_popup` fires for content-initiated popups only (page-side `window.open`, `target="_blank"`). DevTools popups go through the Views path and bypass this handler.

### Known special cases

- **`WindowOpenDisposition::NEW_BACKGROUND_TAB` / `NEW_FOREGROUND_TAB`** — we don't have tabs inside a pane, so same behaviour as `NEW_POPUP`: navigate in place.
- **`SAVE_TO_DISK`** — Chromium asks the host for download handling via a different path (`DownloadHandler`). Not affected.
- **User gesture vs automatic popup** — both routed the same. Automatic popups from ads/pop-unders are now effectively blocked (navigated-in-place consumes the action). Acceptable.
- **Router-style SPAs** that rely on `window.open` for new tabs (e.g. GitHub's file-tree "open in new tab") — user sees the target loaded in the same pane. Workaround: `Ctrl+Click` to open in a new AgentMux pane programmatically is a frontend feature we can add later if it becomes a real friction point. Not in this spec.

### Test plan

- [ ] Create browser pane → navigate to a page with `target="_blank"` links (e.g. any GitHub repo) → click such a link → pane navigates to that URL, no new CEF window spawns.
- [ ] Run `window.open("https://example.com")` in devtools console of the pane → pane navigates to example.com.
- [ ] Run `window.open("")` → cancelled (no navigation, no window).
- [ ] DevTools popup (F12) still opens as a separate window — verify the Views popup path isn't broken.
- [ ] Main window: if any UI code uses `target="_blank"` that we missed, verify it now navigates-in-place (may need to audit main-window links; expected count is 0 since the frontend routes link clicks through `openExternal`).

---

## Rollout

Single PR, single bump. Title: `feat(browser-pane): default URL + in-pane popup redirect`. Merge after reagent approval. Build portable and verify both changes.

## Non-goals for this PR

- A "new tab" pane action that clones the current browser pane beside itself — possible future feature but out of scope; users can drag the widget again for a second pane.
- Ctrl+Click to open in a new pane — frontend-layer, deferrable.
- Download handling — separate spec if needed.
