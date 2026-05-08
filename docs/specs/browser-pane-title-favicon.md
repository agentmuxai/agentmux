# Browser pane: dynamic favicon + page title in pane header

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-07
**Branch:** `agenta/browser-pane-title-favicon`

## Problem

The browser widget's pane header is static: a `globe` font icon and the literal text `Browser`, regardless of where the user navigated. Other tab/pane types in AgentMux carry meaningful identity (agent name, file basename, terminal title); the browser pane should match by reflecting the active page's `<title>` and favicon.

Current state in `frontend/app/view/browser/browser-model.ts`:

| Field | Value | Wired up |
|---|---|---|
| `viewIcon: Accessor<string>` | hardcoded `() => "globe"` | no |
| `viewName: Accessor<string>` | reads `titleAtom`, falls back to `"Browser"` | partial — `setTitle` exists, never called from event |
| `_title` signal | initialized to `"Browser"` | never updated post-init |

The host already receives top-level page title changes via CEF's `DisplayHandler::on_title_change` callback (`agentmux-cef/src/client/mod.rs:107`) and uses them to update the OS window title (Win32 `SetWindowTextW` + Views `Window::set_title`). It does **not** forward the title to the renderer or to the `BrowserViewModel`.

## Goals

1. After navigation completes, the browser pane's header **icon** is the page's favicon (16×16, fits the standard header icon slot) instead of the globe.
2. After navigation completes, the header **text** is the page's `<title>` (or empty → falls back to `"Browser"`) instead of the constant `"Browser"`.
3. The previous values stay visible during in-flight navigation; they update once the new page commits, **without** a flash to `"Browser"` mid-navigation.
4. Tabs with no active page (just-created, blank, `about:blank`) keep `globe` + `"Browser"` defaults.
5. No regression in existing OS window title (host's existing `Window::set_title` + `SetWindowTextW` paths stay intact).
6. No new CEF callbacks beyond `on_title_change` if avoidable (keep host-side surface minimal).

## Non-goals

- Persisting favicon across pane reloads — re-fetched on next nav.
- Pre-loading favicons before navigation (no speculative fetch).
- A favicon cache, CDN, or sprite sheet.
- Honoring `<link rel="icon">` declarations that point at non-default paths (`/icons/site.svg`, `/static/favicon.png`, etc.). Default `/favicon.ico` covers the vast majority of sites; non-conforming sites fall back to the globe via `<img onError>`.
- Multi-frame title aggregation (CEF emits per top-level frame already).
- Right-click on the favicon → bookmark / pin.

## Design tradeoffs

### Title: requires IPC plumbing

The renderer cannot read the embedded page's `<title>` directly — the page is in a separate CEF process. The only signal is CEF's `DisplayHandler::on_title_change` callback, which fires on the host's UI thread. **Decision:** extend the existing `on_title_change` handler (already present for OS window titles) to also emit a renderer IPC event when the browser is a pane.

### Favicon: URL-derived, not CEF-callback-driven

CEF exposes `DisplayHandler::on_favicon_urlchange(browser, icon_urls: Option<&mut CefStringList>)` which fires when the page declares a favicon via `<link rel="icon">`. Two design options were considered:

**Option A — wire the CEF callback.**

Pro: honors arbitrary `<link rel="icon" href="...">` declarations including non-standard paths.

Con: significantly more host-side code (new `DisplayHandler` method, `CefStringList` iteration, second IPC event, second renderer subscription, second unsub in `dispose()`).

**Option B — derive `/favicon.ico` from the page URL in the renderer.**

Pro: zero host-side change. Renderer already receives the page URL via the existing `browser-pane-nav-state` event. Set favicon URL to `new URL(navState.url).origin + "/favicon.ico"`. The renderer's `<img>` element handles cross-origin loads transparently. If the site uses a non-default path or no favicon at all, the `<img>` `onError` event fires and the renderer falls back to the globe icon — same UX as if the host had told us "no favicon."

Con: doesn't follow `<link rel="icon">` declarations that point elsewhere (`/icons/site.svg`, `/static/favicon-32.png`, etc.). In practice ≥ 95% of sites still serve at `/favicon.ico` even when they declare additional paths.

**Decision: Option B for the first cut.** It removes an entire CEF callback wiring, removes a second IPC event, and the failure mode (broken icon → globe fallback) is identical to the success-with-callback case for sites where `/favicon.ico` doesn't exist. A future PR can add Option A's callback if real-world sites the team hits frequently force the issue.

### Reset behavior on navigation start

When the user starts a new navigation:

- **Clear favicon** (`setFaviconUrl("")`) — header reverts to `globe` for the loading state. Avoids briefly showing the previous page's favicon over the new page.
- **Keep title** — the new title arrives via `on_title_change` shortly after load completes. Clearing here would flash `"Browser"` for ~50–500 ms during every navigation, which is jarring.

## Architecture

### Host (`agentmux-cef`)

One change, in `client/mod.rs::on_title_change`. After the existing OS-window-title path, add a tail:

```rust
// Forward to the renderer for browser panes so their pane header
// shows the live page title instead of the static "Browser".
if self.is_browser_pane {
    if let (Some(b), Some(t)) = (browser.as_ref(), title) {
        if let Some(block_id) =
            crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
        {
            crate::events::emit_event_from_state(
                &self.state,
                "browser-pane-title-change",
                &serde_json::json!({
                    "block_id": block_id,
                    "title": t.to_string(),
                }),
            );
        }
    }
}
```

`resolve_pane_block_id` is currently `fn` (private to the module); promote to `pub`.

No new CEF callback impls. No new files in `agentmux-cef`.

### Renderer (`frontend/app/view/browser`)

#### `browser-model.ts`

1. Add a favicon signal:

```ts
private _faviconUrl = createSignal<string>("");
faviconUrlAtom: Accessor<string> = this._faviconUrl[0];
setFaviconUrl = this._faviconUrl[1];
```

2. Convert `viewIcon` from a string accessor to a memo:

```ts
viewIcon: Accessor<string | IconButtonDecl>;
// ... in constructor:
this.viewIcon = createMemo<string | IconButtonDecl>(() => {
    const fav = this.faviconUrlAtom();
    if (fav) return buildBrowserHeaderIcon(fav, this.titleAtom());
    return "globe";
});
```

3. Add a new IPC subscription for `browser-pane-title-change` that calls `setTitle(payload.title || "Browser")`. Track the unsub on `_titleUnsub`; release in `dispose()`.

4. In the existing `browser-pane-nav-state` handler, after `setUrl(payload.url)`, derive and set the favicon:

```ts
try {
    const origin = new URL(payload.url).origin;
    if (origin && origin !== "null") {
        this.setFaviconUrl(`${origin}/favicon.ico`);
    } else {
        this.setFaviconUrl("");
    }
} catch {
    this.setFaviconUrl("");
}
```

5. In `navigate()`, after `setLoading(true)`, add `this.setFaviconUrl("")` so the loading state shows the globe.

#### New file: `components/FaviconImg.tsx`

```tsx
import { createSignal, Show, type JSX } from "solid-js";

export interface FaviconImgProps { src: string; size?: number; }

export const FaviconImg = (props: FaviconImgProps): JSX.Element => {
    const [errored, setErrored] = createSignal(false);
    const size = () => props.size ?? 16;
    return (
        <span class="browser-favicon" aria-hidden="true"
              style={{ display: "inline-flex", "align-items": "center", "line-height": 0 }}>
            <Show when={!errored() && props.src}
                  fallback={<i class="fa-sharp fa-solid fa-globe" style={{ "font-size": `${size()}px` }} />}>
                <img src={props.src} width={size()} height={size()}
                     alt="" aria-hidden="true" onError={() => setErrored(true)} />
            </Show>
        </span>
    );
};
```

#### New file: `components/BrowserHeaderIcon.tsx`

```tsx
import { FaviconImg } from "@/app/view/browser/components/FaviconImg";

export function buildBrowserHeaderIcon(faviconUrl: string, title: string): IconButtonDecl {
    return {
        elemtype: "iconbutton",
        icon: <FaviconImg src={faviconUrl} size={16} />,
        noAction: true,
        title: title || "Browser",
    };
}
```

The `.tsx` separation keeps `browser-model.ts` JSX-free (matches the `AgentPaneIcon.tsx` / `agent-model.ts` split).

### Pane header rendering

`frontend/app/block/blockframe.tsx::getViewIconElem` already handles both `string` and `IconButtonDecl` viewIcon shapes (added in PR #717 for the agent picker). When `viewIcon()` returns the `IconButtonDecl` from `buildBrowserHeaderIcon`, blockframe renders an `IconButton` whose `icon` prop is the `<FaviconImg>` JSX element. No blockframe changes needed.

The `.block-frame-view-icon`'s `opacity: 0.5` was moved off the container in PR #736 (full opacity for SVG/img children, dim only for `<i>` font icons). The favicon will render at full opacity against the dark pane header — matches the brand-icon work.

## File-by-file impact

| File | Change |
|---|---|
| `agentmux-cef/src/browser_pane/callbacks.rs` | `pub fn resolve_pane_block_id` (visibility bump only) |
| `agentmux-cef/src/client/mod.rs` | extend `on_title_change` to emit IPC event for browser panes |
| `frontend/app/view/browser/browser-model.ts` | favicon signal, viewIcon memo, IPC subscriptions, navigate-time clear |
| `frontend/app/view/browser/components/FaviconImg.tsx` | new |
| `frontend/app/view/browser/components/BrowserHeaderIcon.tsx` | new |
| `frontend/app/view/browser/browser-model.test.ts` | extend coverage |

Total: 1 new Rust visibility change, 1 new Rust function body extension, 2 new tsx files, 1 ts file modified, 1 test file extended.

## Test plan

### Unit (`browser-model.test.ts`)

- `setTitle("Foo")` → `viewName()` returns `"Foo"`.
- `setTitle("")` → `viewName()` returns `"Browser"` (fallback).
- `setFaviconUrl("https://x/favicon.ico")` → `viewIcon()` returns an `IconButtonDecl` whose `title` matches the current title.
- `setFaviconUrl("")` after a non-empty value → `viewIcon()` returns `"globe"`.
- `navigate("https://example.com")` → `faviconUrlAtom()` is cleared (loading state). Title atom unchanged.
- `browser-pane-nav-state` event with `url: "https://example.com/page"` → `faviconUrlAtom()` becomes `"https://example.com/favicon.ico"`.
- `browser-pane-nav-state` event with malformed URL → `faviconUrlAtom()` is cleared, no throw.

### Integration (manual smoke in dev build)

1. Open a browser pane to `https://github.com`. Header icon swaps to GitHub's favicon, title becomes `"GitHub · Build and ship software on a single, collaborative platform"` or similar.
2. Navigate to `https://google.com`. Globe flashes briefly (loading state), then favicon swaps to Google's.
3. Navigate to `about:blank`. Header reverts to globe + `"Browser"`.
4. Navigate to a site with no `/favicon.ico` (e.g. a private dev server). Header icon stays globe (onError fallback fires); title still updates.
5. Open multiple browser panes. Each maintains its own favicon and title independently (block_id filter in subscription works).
6. Close a pane mid-navigation. No console errors from late-arriving events (`_closed` flag and unsubs work).

## Edge cases

- **`about:blank`, `chrome://newtab`**: title is empty string → `viewName` falls back to `"Browser"`. `new URL("about:blank").origin === "null"` → favicon cleared; header shows globe.
- **`file:///` URLs**: `URL.origin` is `"null"` → cleared. Globe + filename in title (CEF-provided).
- **Title change before nav-state**: independent event streams. `on_title_change` may fire before `on_load_end`. Already handled — `setTitle` runs immediately.
- **Cross-origin favicon load**: `<img>` doesn't enforce CORS for display (only canvas readback). Should always work for display.
- **Malformed favicon URL**: `<img onError>` fires → `FaviconImg` swaps to globe.
- **Site sets `<link rel="icon">` to non-default path**: our derived `/favicon.ico` may 404. `onError` → globe. We accept this trade-off.
- **Site that 302-redirects `/favicon.ico` to a CDN**: works fine, browser follows the redirect.
- **Late-arriving event after pane close**: `_closed` flag short-circuits the handler. Unsubs in `dispose()` prevent leaks.
- **Multiple `on_title_change` events per page** (SPAs that mutate `document.title`): each event re-runs `setTitle`; latest wins. No debounce needed.
- **Subdomains / multi-tenant sites**: `new URL("https://docs.example.com/foo").origin === "https://docs.example.com"`, so each subdomain gets its own favicon URL. Correct behavior.

## Out-of-scope follow-ups

- **Honor `<link rel="icon">`**: add the host-side `on_favicon_urlchange` callback (Option A from the favicon tradeoff section). Suggested when (and only when) we hit specific real-world sites in the team's daily browsing where `/favicon.ico` doesn't exist.
- **Persistence layer cache**: store last-known favicon per URL so brand-new tabs can show the previous session's icon instantly without waiting for nav.
- **Favicon hover tooltip**: surface the page URL via `IconButtonDecl.title` (currently shows the page title). Could augment.
- **Theme-aware favicon**: most favicons are designed for both light and dark; no fix needed unless a specific site shows poorly.

## Rollout

Single PR off `agenta/browser-pane-title-favicon`. Bump patch. Standard reagent/codex review pass. No feature flag — the change is purely additive behavior for an existing pane type, with the failure mode (broken favicon URL) gracefully handled in the renderer.
