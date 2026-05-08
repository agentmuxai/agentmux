# Browser pane state catalog

**Status:** Analysis (informs reducer migration)
**Owner:** AgentA
**Date:** 2026-05-07

Inventory of every state cell that touches a browser pane — data, focus, lifecycle — and every read/write call site. Written as a precondition to any structural refactor (per "lots of prior work on getting the address bar + doc text routing right").

This is **descriptive**, not prescriptive. It captures *what exists today* so the next refactor can preserve every invariant.

---

## State cells

### 1. View-model data state (per-pane, currently in `BrowserViewModel`)

| Cell | Type | Owner | Source of truth | Notes |
|---|---|---|---|---|
| `urlAtom` | `Accessor<string>` | model | `browser-pane-nav-state` IPC; `navigate()` | Committed/loading URL. |
| `titleAtom` | `Accessor<string>` | model | `browser-pane-title-change` IPC | Falls back to `"Browser"` when empty. |
| `faviconUrlAtom` | `Accessor<string>` | model | derived from `urlAtom` (`${origin}/favicon.ico`) | Empty → header globe. |
| `loadingAtom` | `Accessor<boolean>` | model | `navigate()` set true; `nav-state` set false | Mutually exclusive with `error`. |
| `canGoBackAtom` | `Accessor<boolean>` | model | `nav-state` (urlOnly=false only) | History gate. |
| `canGoForwardAtom` | `Accessor<boolean>` | model | same | History gate. |
| `errorAtom` | `Accessor<string \| null>` | model | `onError()` from view | Mutually exclusive with `loading`. |
| `_closed` (state.closed) | bool | model | `dispose()` | Terminal flag; gates all post-close commands. |

### 2. Per-pane component-local state (lives in `browser-view.tsx`, NOT in the model)

| Cell | Type | Why local | Risk if migrated |
|---|---|---|---|
| `addressBar` | `Accessor<string>` (createSignal) | The **unsubmitted, user-typed** URL. Distinct from `urlAtom` (the **committed** URL). | High — the live-typing UX depends on this being a fast, locally-mutable signal that doesn't round-trip through the reducer for every keystroke. |
| `addressBar` ↔ `urlAtom` sync | `createEffect` | When the page navigates (URL changes), the address bar resets to match — but only if the user hasn't started typing into it. | Subtle: a naive sync would clobber in-progress typing. |
| `paneCreated` | `Accessor<boolean>` | First-paint guard; controls placeholder vs. actual pane HWND show. | Must not regress the placeholder flicker fix. |
| Address bar DOM `<input>` ref | implicit (DOM API) | Used for selection, focus, blur, key handling. | DOM-managed; do **not** try to put in a reducer. |

### 3. Focus state (cross-cutting)

The user's typing destination is a function of focus at multiple levels. Each level has its own state cell, and each is owned by a different system.

| Level | State cell | Owner | Where read | Where written |
|---|---|---|---|---|
| OS window | (Win32 active window) | Win32 | host (`wndproc.rs`) | OS / user click |
| AgentMux window | `activeWindowId` | host saga | host | host on focus event |
| Tab | `activeTabId` (`global.ts:70`) | frontend | many places (uiContext, atom keying) | workspace.activetabid via SetMeta |
| Block (pane) | `focusedNode` (`layoutModel`) | layout reducer | `refocusNode()` consumers | `refocusNode(blockId)` from clicks / keyboard |
| Within pane | DOM `document.activeElement` | DOM | `giveFocus()` checks | Browser DOM focus events |
| Browser pane HWND | CEF browser host focus | CEF / Chromium | host's `browser_pane_focus` IPC handler | `invokeCommand("browser_pane_focus")` |

**Critical interaction (from `BrowserViewModel.giveFocus()`):**

```ts
const active = document.activeElement;
const isMainInput =
    active?.tagName === "INPUT" || active?.tagName === "TEXTAREA";
if (isMainInput) {
    invokeCommand("main_window_focus", {});  // pull OS focus back to main
    return true;
}
invokeCommand("browser_pane_focus", { block_id });  // hand OS focus to pane
```

This single check is *the* implementation of "typing goes to address bar OR DOM". If the address bar `<input>` is the active DOM element when the layout asks who has focus, the model says "main window keeps focus, don't steal it for the pane HWND". Otherwise it tells the host to move OS-level focus into the pane HWND so keystrokes route to Chromium.

**This must be preserved verbatim in any refactor.** It's the bridge between three independent focus systems (DOM, OS, CEF).

### 4. Pane HWND state (host-side, mirrored to renderer via events)

| Cell | Owner | Renderer view |
|---|---|---|
| Pane HWND alive/dead | `browser_pane::hwnd` registry | implicit — events stop firing |
| HWND visibility / z-order | host saga | drives placeholder timing |
| HWND geometry | host saga | placeholder positioning |
| Pointer capture during drag | `useWindowDrag` (Win32 hook) | unrelated to typing focus |

### 5. Lifecycle / IPC subscription state

| Cell | Type | Notes |
|---|---|---|
| `_navUnsub` / `_clickUnsub` / `_titleUnsub` | unsub fn or null | Released on `dispose`. |
| `_closed` | bool | Gates dispose-after-late-event. |
| Promise.allSettled([3 subs]) | gate | The race-fix gate before construction-time `navigate()`. |

---

## Multi-instance cardinality

```
N windows (per AgentMux window)
  N tabs (per window)
    N panes (per tab)
      0..1 browser pane (any single pane *can* be a browser, but typically 0 or 1 per tab is browsing)
        1 BrowserViewModel
          1 url, 1 title, 1 favicon, 1 history pair, 1 error, 1 closed
          1 address bar input (DOM, in browser-view.tsx)
        1 pane HWND (host)
        1 CEF Browser instance
```

**Concretely, a single user could have:** 3 windows × 4 tabs × 2 browser panes per tab = 24 simultaneous BrowserViewModels, each with its own state cell. Today this works because:

- View-model state is per-instance (one `BrowserViewModel` per blockId).
- IPC events carry `block_id` and the renderer's listener filters on it.
- Focus is OS-managed at the window level; AgentMux-level focus is per-tab; pane-level focus is `focusedNode` per layout.
- DOM focus is per-render-process — each AgentMux window is one render process, so address bars across windows can never collide for "active element."

**The constraints any refactor must keep:**

1. **No cross-pane state leakage.** A keystroke in pane A must not affect pane B's URL bar.
2. **No cross-tab state leakage.** Switching tabs must not lose the per-pane URL/title/history/loading.
3. **No cross-window state leakage.** Each window has its own renderer; each has its own copy of the slice.
4. **DOM focus is the source of truth for "where does typing go."** Don't try to track this in a reducer — the DOM already does it.
5. **`giveFocus()` is the bridge.** Layout asks "give focus to this pane?", model decides DOM-vs-OS, IPC fans out.

---

## State write call sites (current)

### From IPC events
- `browser-pane-nav-state` → `setUrl`, `setLoading(false)`, `setCanGoBack/Forward` (urlOnly=false), `SetMetaCommand({url})`
- `browser-pane-title-change` → `setTitle`
- `browser-pane-clicked` → `refocusNode(blockId)`

### From model methods
- `navigate(url)` → `setUrl`, `setError(null)`, `setLoading(true)`, `setFaviconUrl("")`, `SetMetaCommand({url})`, `invokeCommand("browser_pane_navigate")`
- `goBack()` → `setLoading(true)`, `invokeCommand("browser_pane_go_back")`
- `goForward()` → `setLoading(true)`, `invokeCommand("browser_pane_go_forward")`
- `reload()` → `setLoading(true)`, `invokeCommand("browser_pane_navigate", state.url)`
- `onError(msg)` → `setError(msg)`, `setLoading(false)`
- `dispose()` → flips closed, unsubs IPC

### From component-local state (browser-view.tsx)
- `setAddressBar(value)` on every keystroke in the URL `<input>`
- `setAddressBar(modelUrl)` reactively when `urlAtom` changes (sync)

### From layout / focus
- `refocusNode(blockId)` — called by `browser-pane-clicked` and many other paths (keyboard nav, click on header, etc.)

---

## Read sites

### Of `urlAtom`
- `browser-view.tsx` URL bar `<input value>` (initial)
- `browser-view.tsx` reload button (so it knows current URL)
- `browser-view.tsx` placeholder `<Show when={!model.urlAtom()}>` (suppresses iframe before first nav)
- `browser-view.tsx` reactive sync to `addressBar`

### Of `titleAtom`, `faviconUrlAtom`
- `BrowserViewModel.viewName` / `viewIcon` memos → blockframe header

### Of `loadingAtom`, `errorAtom`, `canGoBackAtom`, `canGoForwardAtom`
- Toolbar buttons (back / forward / reload) — disabled state
- Error overlay component

### Of `closed`
- All public methods short-circuit
- `_closed` checks inside late .then callbacks for IPC subscriptions

---

## What a future reducer migration must NOT change

1. **`addressBar` stays component-local.** It's an unsubmitted-input concern, not a domain concern. Routing every keystroke through a reducer adds latency for zero benefit.
2. **`giveFocus()` keeps the DOM check.** `document.activeElement.tagName === "INPUT"` is the answer to "is the user typing into the address bar?". A reducer can't see `activeElement`.
3. **The `addressBar ↔ urlAtom` sync rule.** Currently:  on every URL change, address bar resets to match the new URL. This must continue (it's how `browser_pane_navigate` from a link click reflects in the bar).
4. **Per-pane independence.** Each `BrowserViewModel` is its own state cell keyed by `blockId`. Stays.
5. **IPC subscription registration race fix.** Promise.allSettled gate stays.
6. **All current IPC events.** No payload shape changes; the renderer's filter on `block_id` stays.
7. **All current reducer invariants** (closed-terminal, error/loading mutex, urlOnly history-gate masking, favicon derivation, title fallback).

## What a future reducer migration *can* improve

1. **Audit trail / `recordDispatch`** — we lose nothing by adding it (the reverted slot-lifecycle attempt did this correctly).
2. **Slot lifecycle (`registerPane` / `unregisterPane`)** — but only after the catalog of state-write call sites is complete and tested. The previous attempt missed e.g. that `addressBar ↔ urlAtom` is a CONSUMER of state changes (not a producer), and that any race in re-registration could break the typing UX.
3. **Cross-pane visibility for diagnostics** — a single `Map<blockId, BrowserPaneState>` lets the diagnostics panel show all browser panes at once.

## Open questions for the future migration

- Should `addressBar` (the typing buffer) be promoted to a state cell? Trade-offs: reducer audit vs. keystroke latency. Probably no.
- Should the `addressBar ↔ urlAtom` sync rule become a reducer event? E.g., `NavigationCommitted → AddressBarShouldResync`? Only if we want to make the sync rule explicit and testable.
- Should focus-which-element-has-the-cursor be reducer-tracked at all? Probably no — the DOM is fine for this.
- How does this interact with **multi-window**? Each window is a separate renderer process; each renderer has its own `Map<blockId, State>`. Cross-window coordination would require the host to relay events, which is currently NOT how nav-state works (host emits to ALL renderers but each renderer's filter only fires for its own block_ids).

## Summary table — every browser-pane state cell, by ownership

| Cell | Owner | Lives in | Per-instance? | Notes |
|---|---|---|---|---|
| `urlAtom` | model | `BrowserViewModel` | per blockId | core |
| `titleAtom` | model | `BrowserViewModel` | per blockId | core |
| `faviconUrlAtom` | model | derived | per blockId | derived from url |
| `loadingAtom` | model | `BrowserViewModel` | per blockId | mutually exclusive with error |
| `canGoBackAtom` | model | `BrowserViewModel` | per blockId | history |
| `canGoForwardAtom` | model | `BrowserViewModel` | per blockId | history |
| `errorAtom` | model | `BrowserViewModel` | per blockId | mutually exclusive with loading |
| `closed` | model | `BrowserViewModel` | per blockId | terminal |
| `addressBar` | view | `browser-view.tsx` | per pane mount | typing buffer |
| `paneCreated` | view | `browser-view.tsx` | per pane mount | placeholder gate |
| Address bar DOM ref | DOM | DOM tree | per render | focus check target |
| `focusedNode` | layout | `layoutModel` | per tab | drives `refocusNode` |
| `activeTabId` | global | `global.ts` | per window | tab selection |
| Window active state | OS / Win32 | OS | per window | OS focus |
| CEF browser focus | CEF / Chromium | CEF | per pane | OS focus inside pane HWND |
| Pane HWND visible | host saga | host | per blockId | placeholder timing |
| `_navUnsub` / `_clickUnsub` / `_titleUnsub` | model | `BrowserViewModel` | per blockId | sub lifecycle |

---

**Bottom line:** the browser pane is more than a state machine for `{url, title, favicon, history}`. It's the intersection of DOM focus, OS window focus, CEF browser focus, AgentMux layout focus, tab focus, and an unsubmitted-typing buffer that has its own UX rules. A reducer can model the data state cleanly; it can't (and shouldn't try to) model the cross-system focus story. Any future structural refactor needs to keep every cell in this catalog working — and the previous slot-lifecycle attempt missed at least the `addressBar` typing buffer and the `addressBar ↔ urlAtom` sync rule.
