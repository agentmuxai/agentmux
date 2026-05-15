# Browser Pane: Live Favicon + Page Title in Pane Header

**Date:** 2026-05-15  
**Status:** Proposed  
**Author:** AgentY  
**Depends on:** browser-pane-reducer-roadmap.md Phase 6 (this spec IS Phase 6, broken into two independent sub-features)

---

## Summary

When the embedded browser pane navigates to a page, the pane header shows:
- **Icon:** `globe` (FontAwesome) — static, never changes regardless of the loaded page
- **Title:** `"Browser"` — static fallback, never updates from the actual page title

This spec wires both to live data from CEF. After this change the pane header shows the real page favicon as its icon, and the real `<title>` as its name — exactly like a browser tab.

---

## Current State Analysis

### What exists (and works)

| Component | Status | File |
|-----------|--------|------|
| `TitleChanged` reducer command | ✅ Fully implemented | `frontend/app/store/browser-pane-state/reducer.ts:172` |
| `title` cell in `BrowserPaneState` | ✅ Fully implemented | `frontend/app/store/browser-pane-state/types.ts:73` |
| `title` projection → `setTitle(next)` | ✅ Wired in `BrowserViewModel` | `browser-model.ts:234` |
| `viewName` returns `titleAtom()` | ✅ Plumbed to header | `browser-model.ts:250` |
| `faviconUrl` cell in state | ✅ Exists, derived from URL | `types.ts:91` |
| `faviconUrlAtom` signal on model | ✅ Exists | `browser-model.ts:110` |
| Rust `on_title_change` handler | ✅ Fires on page load | `agentmux-cef/src/client/mod.rs:107` |
| CEF `on_favicon_urlchange` trait method | ✅ Available in cef-146 binding | `ImplDisplayHandler::on_favicon_urlchange` |
| `resolve_pane_block_id()` | ✅ Already works | `browser_pane/callbacks.rs:241` |

### What is missing (the gap)

**Title gap:** `on_title_change` (line 107 in `client/mod.rs`) only calls `SetWindowTextW`. It does NOT check `is_browser_pane` and does NOT emit `browser-pane-title-change` IPC. The frontend has no subscription to any title event.

**Favicon gap:** `on_favicon_urlchange` is not overridden anywhere in `AgentMuxHandler`. The current `deriveFaviconUrl(url)` in the reducer constructs `origin/favicon.ico` — a guess that fails for sites using `/wp-content/...`, `/assets/...`, or Apple touch icons.

**Icon rendering gap:** `ViewModel.viewIcon` type is `Accessor<string | IconButtonDecl>`. `getViewIconElem()` in `blockframe.tsx:154` only renders FontAwesome icon strings or `IconButtonDecl` buttons. It has no code path for image URLs. Showing a real favicon requires either extending the type or rendering an `<img>` from inside the view model.

---

## Design

Two independent sub-features. Each is a small, self-contained PR.

### Sub-feature 1: Live page title (simpler — do first)

No new UI changes. The title already flows to the header — the only gap is the IPC bridge.

**Rust: emit title change for panes**

In `agentmux-cef/src/client/mod.rs`, `on_title_change` (line 107), add a pane branch after the existing `SetWindowTextW` block:

```rust
fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
    // ... existing window title code ...

    // Emit title to frontend for browser panes.
    if self.is_browser_pane {
        if let Some(browser) = browser.as_deref() {
            if let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(
                &self.state, browser,
            ) {
                let title_str = title.map(|t| t.to_string()).unwrap_or_default();
                let block_id_short: String = block_id.chars().take(7).collect();
                tracing::info!(
                    "[browser-pane:diag][{}] emit-title-change title={:?}",
                    block_id_short, title_str,
                );
                crate::events::emit_event_from_state(
                    &self.state,
                    "browser-pane-title-change",
                    &serde_json::json!({ "block_id": block_id, "title": title_str }),
                );
            }
        }
    }
}
```

Note: `resolve_pane_block_id` is currently a `fn` (not `pub fn`) in `callbacks.rs`. It needs to be made `pub(crate)` for this call site.

**Frontend: subscribe in `BrowserViewModel`**

In `browser-model.ts`, alongside the existing `browser-pane-nav-state` subscription (around line 260), add:

```typescript
void listenEvent<{ block_id: string; title: string }>(
    "browser-pane-title-change",
    (payload) => {
        if (this.closed) {
            this.diag(`post-close-event-dropped name=browser-pane-title-change`);
            return;
        }
        if (payload.block_id !== this.blockId) return;
        this.diag(`title-change recv title=${JSON.stringify(payload.title)}`);
        this._dispatch({ type: "TitleChanged", title: payload.title }, "title-change");
    }
).then((unsub) => {
    this.diag(`sub-registered name=browser-pane-title-change`);
    if (this.closed) unsub();
    else this._titleUnsub = unsub;
});
```

Add `private _titleUnsub: (() => void) | null = null;` with the other unsub fields, and null+call it in `dispose()`.

**Result:** The pane header title updates on every page navigation. `TITLE_FALLBACK = "Browser"` remains the initial value and the fallback for pages that emit an empty title.

---

### Sub-feature 2: Live favicon in pane header (more involved)

This requires changes in three layers: Rust (emit real URLs), frontend state (override derived URL), and blockframe (render image).

#### Layer A: Rust — emit real favicon URLs

`AgentMuxHandler` implements `ImplDisplayHandler`. The `on_favicon_urlchange` method already has a no-op default in the trait. Override it in `agentmux-cef/src/client/mod.rs` (alongside `on_title_change`):

```rust
fn on_favicon_urlchange(
    &mut self,
    browser: Option<&mut Browser>,
    icon_urls: Option<&mut CefStringList>,
) {
    if !self.is_browser_pane {
        return;
    }
    let Some(browser) = browser.as_deref() else { return };
    let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(
        &self.state, browser,
    ) else { return };

    let urls: Vec<String> = icon_urls
        .map(|list| {
            (0..list.len())
                .filter_map(|i| list.get(i).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let block_id_short: String = block_id.chars().take(7).collect();
    tracing::info!(
        "[browser-pane:diag][{}] emit-favicon-urls count={} urls={:?}",
        block_id_short,
        urls.len(),
        urls,
    );

    crate::events::emit_event_from_state(
        &self.state,
        "browser-pane-favicon-urls",
        &serde_json::json!({ "block_id": block_id, "urls": urls }),
    );
}
```

CEF fires `on_favicon_urlchange` for each navigation once the page's `<link rel="icon">` elements are parsed. `icon_urls` contains the full URL list in document order; the first one is usually the primary favicon.

#### Layer B: Frontend state — `FaviconUrlConfirmed` command

The current `faviconUrl` cell is a **derived projection** of `url` — no independent input command exists. For real favicon URLs from CEF we need an override command.

**In `types.ts`**, add to `BrowserPaneCommand`:

```typescript
/**
 * Host emitted the page's actual favicon URLs from on_favicon_urlchange.
 * Overrides the guessed `origin/favicon.ico` derivation. An empty `urls`
 * array clears back to the derived value. Idempotent on identical url.
 */
| { type: "FaviconUrlsReceived"; urls: string[] }
```

And to `BrowserPaneState`, add a flag to track whether favicon was overridden:

```typescript
/** When true, faviconUrl came from an explicit on_favicon_urlchange event
 *  and should not be overwritten by URL derivation. Cleared on Navigate
 *  so each new page starts from the derived heuristic until CEF reports. */
faviconOverridden: boolean;
```

**In `reducer.ts`**, handle `FaviconUrlsReceived`:

```typescript
case "FaviconUrlsReceived": {
    // Pick first URL; caller provides the list in document order.
    const next = command.urls[0] ?? "";
    if (next === state.faviconUrl && state.faviconOverridden) {
        return { state, events: [] };
    }
    return {
        state: { ...state, faviconUrl: next, faviconOverridden: next !== "" },
        events: [{ type: "favicon-urls-received", url: next }],
    };
}
```

And clear `faviconOverridden` in the `Navigate` case:

```typescript
case "Navigate": {
    return {
        state: {
            ...state,
            loading: true,
            error: null,
            url: command.url,
            faviconUrl: deriveFaviconUrl(command.url),  // heuristic while real one loads
            faviconOverridden: false,
        },
        events: [{ type: "navigate", url: command.url }],
    };
}
```

**In `browser-model.ts`**, subscribe to the new event:

```typescript
void listenEvent<{ block_id: string; urls: string[] }>(
    "browser-pane-favicon-urls",
    (payload) => {
        if (this.closed) return;
        if (payload.block_id !== this.blockId) return;
        this.diag(`favicon-urls recv count=${payload.urls.length}`);
        this._dispatch({ type: "FaviconUrlsReceived", urls: payload.urls }, "favicon-urls");
    }
).then((unsub) => {
    if (this.closed) unsub();
    else this._faviconUnsub = unsub;
});
```

#### Layer C: Render favicon image in pane header

**The constraint:** `ViewModel.viewIcon` is typed `Accessor<string | IconButtonDecl>`. `getBlockHeaderIcon()` in `blockutil.tsx:120` takes a `string`, converts it to a FontAwesome class, and returns `<i class="fa-solid fa-..."/>`. It has no image path for `favicon.ico` URLs.

**Chosen approach: extend `ViewModel.viewIcon` to accept `JSX.Element`**

This is the minimal-change path that avoids wrapping a favicon image inside a button affordance.

**Step 1: Update `custom.d.ts`**

```typescript
interface ViewModel {
    viewType?: string;
    viewIcon?: Accessor<string | IconButtonDecl | JSX.Element>;  // add JSX.Element
    viewName?: Accessor<string>;
    // ...
}
```

**Step 2: Update `getViewIconElem` in `blockframe.tsx:154`**

```typescript
function getViewIconElem(viewIconUnion: string | IconButtonDecl | JSX.Element, blockData: Block): JSX.Element {
    if (viewIconUnion == null || typeof viewIconUnion === "string") {
        const viewIcon = viewIconUnion as string;
        return <div class="block-frame-view-icon">{getBlockHeaderIcon(viewIcon, blockData)}</div>;
    } else if (isJSXElement(viewIconUnion)) {
        // Raw JSX — wrap in the same container so layout is consistent.
        return <div class="block-frame-view-icon">{viewIconUnion as JSX.Element}</div>;
    } else {
        return <IconButton decl={viewIconUnion as IconButtonDecl} className="block-frame-view-icon" />;
    }
}
```

`isJSXElement` is a simple runtime discriminator:

```typescript
function isJSXElement(v: unknown): boolean {
    // SolidJS JSX elements are objects with a $$typeof symbol or a
    // function component — not a plain string, not a number.
    // An IconButtonDecl is a POJO with elemtype:"iconbutton"; a JSX element
    // produced by solid-js is a function or an object without elemtype.
    if (typeof v === "function") return true;
    if (typeof v === "object" && v !== null && !("elemtype" in v)) return true;
    return false;
}
```

**Step 3: `BrowserViewModel.viewIcon` becomes a memo**

In `browser-model.ts`, replace:

```typescript
viewIcon: Accessor<string> = () => "globe";
```

with:

```typescript
viewIcon: Accessor<string | JSX.Element> = createMemo(() => {
    const url = this.faviconUrlAtom();
    if (!url) return "globe";
    return (
        <img
            class="browser-pane-favicon"
            src={url}
            alt=""
            onError={(e) => {
                // Hide broken image; blockframe falls back to globe below.
                (e.currentTarget as HTMLImageElement).style.display = "none";
            }}
            width={14}
            height={14}
        />
    ) as JSX.Element;
});
```

**Step 4: CSS — `browser-pane-favicon`**

In `browser-view.scss` (or a new `browser-model.scss`):

```scss
.browser-pane-favicon {
    width: 14px;
    height: 14px;
    object-fit: contain;
    display: block;
    border-radius: 2px;
}
```

The `.block-frame-view-icon` container is already `14px` wide (matches FontAwesome icon slot). No layout changes needed.

---

## Rendering result

Before:

```
[🌐] Browser
```

After navigating to `https://github.com`:

```
[🐙] github / reagent · Pull Request #156
```

(favicon from `https://github.com/favicon.ico`, title from page `<title>`)

The title truncates via the existing `.block-frame-view-type` ellipsis CSS — no additional work needed.

---

## Error handling

| Scenario | Behavior |
|----------|----------|
| Page has no `<link rel="icon">` | CEF may not fire `on_favicon_urlchange`; `faviconUrl` stays at `origin/favicon.ico` heuristic |
| Favicon 404 or broken | `<img onError>` hides the element; `.block-frame-view-icon` renders empty (acceptable — title still shows) |
| `on_title_change` emits empty string | Reducer folds to `TITLE_FALLBACK = "Browser"` (existing Invariant 6) |
| Pane closed before event arrives | `_dispatch` guard in model no-ops; unsubs called in `dispose()` |
| Multiple favicon URLs from CEF | First URL used; this is the primary favicon per HTML spec |

---

## Files changed

### Sub-feature 1 (title)
- `agentmux-cef/src/client/mod.rs` — add IPC emit in `on_title_change` for panes
- `agentmux-cef/src/browser_pane/callbacks.rs` — make `resolve_pane_block_id` `pub(crate)`
- `frontend/app/view/browser/browser-model.ts` — add `browser-pane-title-change` subscription, `_titleUnsub`

### Sub-feature 2 (favicon)
- `agentmux-cef/src/client/mod.rs` — add `on_favicon_urlchange` override
- `agentmux-cef/src/browser_pane/callbacks.rs` — `resolve_pane_block_id` already `pub(crate)` from sub-feature 1
- `frontend/app/store/browser-pane-state/types.ts` — add `FaviconUrlsReceived` command, `faviconOverridden` field, `favicon-urls-received` event
- `frontend/app/store/browser-pane-state/reducer.ts` — handle `FaviconUrlsReceived`, clear `faviconOverridden` in `Navigate`
- `frontend/types/custom.d.ts` — extend `ViewModel.viewIcon` union to include `JSX.Element`
- `frontend/app/block/blockframe.tsx` — `getViewIconElem` handles JSX.Element, `isJSXElement` discriminator
- `frontend/app/view/browser/browser-model.ts` — `viewIcon` memo, `browser-pane-favicon-urls` subscription, `_faviconUnsub`
- `frontend/app/view/browser/browser-view.scss` — `.browser-pane-favicon` sizing

---

## Testing

```
# Verify title updates on navigation
1. Open browser pane → navigate to https://github.com
2. Header should update from "Browser" to "GitHub · Build and ship software on a single, collaborative platform · GitHub"
   (or truncated version)
3. Navigate to https://www.rust-lang.org
4. Header should update to "Rust Programming Language"

# Verify title fallback
5. Navigate to about:blank → header should remain "Browser" (empty title folds to fallback)

# Verify favicon appears  
6. Open browser pane → navigate to https://github.com
7. Globe icon in header should be replaced by GitHub's favicon image
8. Navigate to https://news.ycombinator.com
9. Orange Y favicon should appear

# Verify favicon error fallback
10. Navigate to a page with a 404 favicon
11. Icon slot should render empty (no broken image icon)

# Verify pane lifecycle safety
12. Close a pane while it's still loading — no errors in console
    grep: muxlog host '[browser-pane:diag]' should show post-close-event-dropped, not an exception
```

---

## Relation to browser-pane-reducer-roadmap.md

This spec is what the roadmap calls "Phase 6" — the actual product features. The roadmap's Phase 5 (slot audit / diag panel) is not a prerequisite for these changes; both sub-features are additive and don't restructure any existing state transitions. They can ship independently of Phase 5.

The roadmap's caution about PR #737 (regression from trying to migrate + feature in one PR) is addressed here by splitting into two tiny PRs that are purely additive.

---

## PR order

1. **`agenty/browser-pane-live-title`** — Sub-feature 1 only (~60 lines changed)
2. **`agenty/browser-pane-live-favicon`** — Sub-feature 2 only (~150 lines changed)

No blocking dependency between the two; they touch non-overlapping code except both use `resolve_pane_block_id` (sub-feature 1 makes it `pub(crate)`, sub-feature 2 uses it). Sub-feature 2 PR should rebase on sub-feature 1.
