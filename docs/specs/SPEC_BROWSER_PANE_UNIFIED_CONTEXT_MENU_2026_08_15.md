# SPEC — Browser pane: replace Chromium's native right-click menu with the app's own

**Date:** 2026-08-15
**Type:** Feature (design proposal — not yet implemented)
**Status:** Draft
**Scope:** `agentmux-cef` (new `ContextMenuHandler` impl, browser-pane only) +
`frontend/app/view/browser/browser-model.ts` (new `getBodyContextMenuItems`
implementation) + `frontend/app/block/blockframe.tsx` (no change expected —
existing extension point should just work, see below).

## Problem

Right-clicking inside a browser pane's content shows Chromium's own native
context menu (Back / Forward / Reload / Print / Save As / View Page Source /
Inspect, laid out Chrome-style) instead of AgentMux's own pane context menu
(the split-up/down/left/right + replace + color + close menu every other
pane type shows). This is inconsistent with the rest of the app and doesn't
compose with pane-level actions (split, replace, etc.) while on a web page.

Goal: right-clicking a browser pane's content shows the SAME app-style menu
every other pane uses (`buildPaneContextMenu` — split, replace, color, close,
etc.), extended with the browser-specific actions users still need: Back,
Forward, Reload, Print, View Page Source, Copy Link Address (when over a
link), Inspect Element / DevTools. Not a redesign of the menu system — an
extension, using the extension point that already exists for this purpose.

## Root cause (why the native menu wins today)

Same root cause family as the click-to-select and click-dismisses-menus
issues (see the two sibling specs). Per
`docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md`, a browser pane's content
is a native, sibling `CefBrowserView` (CEF Views `AddOverlayView`), not DOM.
`blockframe.tsx`'s body already has an `onContextMenu` handler
(`onBodyContextMenu`, `blockframe.tsx:866`) that builds exactly the menu we
want — `buildPaneContextMenu` plus any `viewModel.getBodyContextMenuItems()`
— but it's a **DOM** `contextmenu` event handler, so it never fires for a
right-click landing on the pane's native overlay content. CEF's own default
`ContextMenuHandler` behavior (used because AgentMux doesn't implement the
interface at all today — confirmed, zero references to
`ContextMenuHandler`/`on_before_context_menu`/`run_context_menu` anywhere in
`agentmux-cef/src`) takes over instead and shows the native OS-style menu.

## Existing extension point (reuse, don't invent a new one)

`ViewModel` already supports pane-type-specific context menu items via two
optional methods (`frontend/types/custom.d.ts:517-518`):

```ts
getSettingsMenuItems?: () => ContextMenuItem[];      // header right-click, appended
getBodyContextMenuItems?: () => ContextMenuItem[];   // body right-click, PREPENDED
```

`getBodyContextMenuItems` is exactly what we need — `blockframe.tsx`'s
`onBodyContextMenu` already prepends its result (with a trailing separator)
before the shared `buildPaneContextMenu` items:

```ts
// blockframe.tsx:866-883
const onBodyContextMenu = (e: MouseEvent) => {
    ...
    const bodyItems = props.viewModel?.getBodyContextMenuItems?.();
    if (bodyItems && bodyItems.length > 0) menu.push(...bodyItems, { type: "separator" });
    menu.push(...buildPaneContextMenu(blockData(), {...}, props.viewModel));
    ContextMenuModel.showContextMenu(menu, e);
};
```

`termViewModel.ts` and `sysinfo-model.ts` already implement this pattern —
`termViewModel.ts` is the closest precedent (adds actions specific to its own
content type). `browser-model.ts` implementing the same method is the
"retain these functions, add them to the general menu" the user asked for —
it is literally what this extension point is for.

**The gap is only that nothing ever calls `onBodyContextMenu` for a browser
pane**, because CEF intercepts the right-click natively before it becomes a
DOM `contextmenu` event. Closing that gap is the actual work here.

## Proposed design

### 1. Suppress CEF's native menu and capture its params (backend)

CEF's `ContextMenuHandler` trait is confirmed available in the vendored
bindings (`cef` crate v148, `ImplContextMenuHandler`) with everything needed:

```rust
fn on_before_context_menu(&self, browser, frame, params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>);
fn run_context_menu(&self, browser, frame, params, model, callback: Option<&mut RunContextMenuCallback>) -> c_int;
fn on_context_menu_command(&self, browser, frame, params, command_id: c_int, event_flags: EventFlags) -> c_int;
```

`ContextMenuParams` (`ImplContextMenuParams`) exposes everything the app-side
menu needs to build browser-specific items and position itself correctly:

| Getter | Use |
|---|---|
| `xcoord()` / `ycoord()` | Where to render the app's own popover menu |
| `link_url()` | Populates "Copy Link Address" (only shown when non-empty) |
| `page_url()` | For "View Page Source" / "Print" targeting |
| `selection_text()` | Could gate a "Copy" item, mirrors other panes' copy handling in `pane-actions.ts` |
| `is_editable()` / `edit_state_flags()` | Whether to show Cut/Copy/Paste for an editable field (e.g. a web form's `<input>`) |
| `media_type()` / `has_image_contents()` | Could gate "Copy Image" / "Save Image As" (stretch — not required for v1) |

Add a new browser-pane-scoped `ContextMenuHandler` impl (mirrors how
`RequestHandler`/`LoadHandler` are already scoped per-handler in
`client/handlers.rs`). Wire it the same way `on_before_browse` already gates
on `self.is_browser_pane` — this must NOT change context-menu behavior for
the main app UI (right-clicking the app chrome itself must keep working
exactly as it does today; this is browser-pane-only).

In `run_context_menu`:
1. Resolve `block_id` via the existing
   `browser_pane::callbacks::resolve_pane_block_id` helper (same one
   `on_before_browse` now uses for the load watchdog).
2. Translate `params.xcoord()`/`ycoord()` (frame-relative) into
   window/screen coordinates the frontend can use to position its popover —
   check how the click-to-select flow or the pane's own screen-position
   tracking (`ui_tasks/pane_geometry.rs`) already does this translation, to
   reuse rather than re-derive it.
3. Emit a new event, e.g. `browser-pane-context-menu`, carrying `block_id`,
   the translated coordinates, and the subset of `ContextMenuParams` fields
   above (link URL, selection text, editable state, can-go-back/forward —
   the latter two already available via `browser.can_go_back()`/
   `can_go_forward()`, no new CEF surface needed).
4. Call `callback.cancel()` (never `.cont()` — that would still show CEF's
   native menu) and return `1` (handled) so CEF suppresses its own menu
   entirely. `on_before_context_menu`'s `model.clear()` is a documented
   alternative/belt-and-suspenders (clear the model before CEF would show
   it), but suppressing in `run_context_menu` + `cancel()` is the more
   direct mechanism and avoids relying on the empty-model side effect.

This is the same "native event in, IPC event out, frontend renders its own
UI" shape as `browser-pane-clicked` (click-to-select) — no new architectural
pattern, just a second application of the existing one.

### 2. Render the app's menu (frontend)

- `browser-model.ts` implements `getBodyContextMenuItems()`-equivalent
  content: Back (disabled if `!canGoBack`), Forward (disabled if
  `!canGoForward`), Reload — all three call the model's own existing
  `goBack()`/`goForward()`/`reload()` (already implemented, see
  `browser-model.ts:517-543`, no new IPC needed for these three), then Print,
  View Page Source, Copy Link Address (conditional on `link_url` being
  present), Inspect Element (new — see below).
- A new always-mounted listener (same shape as the click-dismiss-menus fix in
  the sibling spec) receives `browser-pane-context-menu`, resolves the
  target pane's `BrowserModel`/`ViewModel`, and calls
  `ContextMenuModel.showContextMenu(menu, {clientX, clientY})` directly at
  the translated coordinates — `blockframe.tsx`'s `onBodyContextMenu` itself
  is never invoked (there's no real DOM event to intercept), but it can stay
  exactly as-is; this is a second caller of `ContextMenuModel.showContextMenu`
  and `buildPaneContextMenu`, not a change to the existing one.
- Menu ordering should match `onBodyContextMenu`'s existing convention:
  browser-specific items first, separator, then the shared
  `buildPaneContextMenu` (split/replace/color/close) — i.e. build the exact
  same `menu` array `onBodyContextMenu` would have built, had the DOM event
  fired.

### 3. New actions needing backend support

Print, View Page Source, and Inspect Element have no existing IPC path
(unlike Back/Forward/Reload, which reuse `browser-model.ts`'s existing
methods):

- **Print**: CEF exposes `CefBrowserHost::Print()` (triggers the native/CEF
  print UI for that browser) — check `ImplBrowserHost` in the vendored
  bindings for the Rust method name; if unavailable, `frame.execute_java_script("window.print()")`
  is a viable fallback (same injection mechanism already used elsewhere in
  `on_load_end`).
- **View Page Source**: Chromium supports a `view-source:` URL prefix
  (`view-source:<page_url>`) that can be loaded via the pane's existing
  `navigate()`/`frame.load_url()` path — likely doesn't need any new backend
  command, just a frontend-side URL transform before calling the pane's
  existing navigate flow. Needs verification that CEF's `view-source:` scheme
  isn't blocked by `on_before_browse`'s `is_disallowed_pane_nav_scheme` guard
  (added in this same session for the OS-handoff/UAC fix) — if it is, that
  guard needs a `view-source:` carve-out.
- **Inspect Element**: CEF exposes `CefBrowserHost::ShowDevTools()` (with an
  optional inspect-element point). Needs a new IPC command (e.g.
  `browser.inspectElement`) since nothing in the app currently opens
  DevTools for an arbitrary browser pane.

## Scope / caveats

- Windows + macOS only initially, following the same platform split as the
  two sibling native-overlay specs (`resolve_pane_block_id` and
  `can_go_back`/`can_go_forward` are platform-agnostic, but this whole
  feature depends on `ContextMenuHandler` being wired into the browser-pane
  CEF client the same way `RequestHandler`/`LoadHandler` are — need to
  confirm CEF's context-menu callback fires identically cross-platform before
  assuming Linux gets this for free; unlike the click events, context-menu
  handling is NOT part of the Windows-HWND-subclass/macOS-swizzle machinery,
  so this may not have the same Linux gap as the other two specs — needs
  verification during implementation, not assumed here).
- "Save As" / "Save Image As" / spell-check suggestions / "Cast" are out of
  scope for v1 — not mentioned by the user, and each adds its own file-dialog
  or media-specific complexity. Can be added later as more
  `getBodyContextMenuItems()` entries without touching the suppression
  mechanism.
- Must not regress right-click behavior on the main app UI or on
  non-browser-pane native overlays if any exist — scope the new
  `ContextMenuHandler` impl to `is_browser_pane` exactly like
  `on_before_browse` already does.

## Verification plan (once implemented)

- Right-click a loaded browser pane's body → app-style menu appears (not
  Chromium's native one), positioned at the cursor.
- Back/Forward disabled state matches the pane's actual history state.
- Reload, Back, Forward all work from the new menu.
- Copy Link Address appears only when right-clicking an actual link, and the
  clipboard content is correct.
- Print opens print output/dialog for the pane's current page.
- View Page Source shows the page's HTML source.
- Inspect Element opens DevTools, ideally scrolled/highlighted to the
  clicked element if CEF's inspect-element-at-point is wired through.
- Split/Replace/Color/Close items from the shared menu all still work
  identically to how they work from a normal pane's context menu.
- Right-clicking the pane HEADER (already real DOM) is unaffected — confirm
  no regression there, since this change is body-only.
