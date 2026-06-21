# SPEC: Tab UI Refinements

**Date:** 2026-06-20  
**Status:** Approved for implementation  
**Scope:** `frontend/app/tab/`

---

## Overview

Three targeted changes to the tab bar UI:

1. **Larger close button (X)** — increase the SVG icon and hit target
2. **Context-menu "Close" dismisses the menu only** — not the tab
3. **Close-confirmation modal** — window-wide confirm before closing a tab, with a persistent "don't remind me" preference

---

## 1. Larger Close Button

### Current state

- **`frontend/app/tab/tab.tsx:294-307`** — SVG `width="14" height="14"`, `viewBox="0 0 16 16"`
- **`frontend/app/tab/tab.scss:121-135`** — `.wave-button` container is 16×16px

### Proposed change

Increase the close icon to **16×16** and its container to **20×20px**, giving a larger click target without crowding the tab.

#### `tab.tsx` (lines 294-296)
```diff
- width="14"
- height="14"
+ width="16"
+ height="16"
```
`viewBox` stays `"0 0 16 16"` — no change needed, the icon path already fills a 16-unit grid.

#### `tab.scss` (lines 121-135)
```diff
  .wave-button {
    flex-shrink: 0;
-   width: 16px;
-   height: 16px;
+   width: 20px;
+   height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
```

No other sizing changes. The tab's inner flex layout already centers the button; the 4px increase is absorbed by the tab-inner padding.

---

## 2. Context-Menu "Close" Dismisses the Menu Only

### Current state

**`frontend/app/tab/tab.tsx:32-104`** — `TabContextPanel` component.  
The "✕ Close" button at lines 97-99 calls `props.onClose`, which closes the **tab** (deletes it from the workspace).

### Proposed change

"Close" in the context menu should **dismiss the popup** — not close the tab. Tab closing is now guarded by the confirmation modal (section 3) and is intentionally only reachable via the X button. The context menu "Close" becomes an escape hatch: "I right-clicked by mistake, dismiss this menu."

#### `tab.tsx` — `TabContextPanel` (lines 97-99)
```diff
- <button class="tab-context-btn tab-context-btn-close" onClick={() => { props.onClose?.(); setShowPanel(false); }}>
-     ✕ Close
- </button>
+ <button class="tab-context-btn tab-context-btn-close" onClick={() => setShowPanel(false)}>
+     ✕ Close
+ </button>
```

> Note: exact signal/setter name for the panel's open state needs to be confirmed at implementation time — the Explore agent found the panel is dismissed via `setShowPanel` or equivalent in the `TabContextPanel` closure. The key invariant is: clicking "Close" in the menu calls only the panel-dismiss callback, with no `onClose` call.

**Rename the button label** from "✕ Close" to "✕ Close menu" for clarity, so users understand it closes the popup, not the tab.

---

## 3. Close-Confirmation Modal

### Behaviour

When the user clicks the X button on a tab:

1. **Check** `settingsAtom()["tab:skipcloseconfirm"]`.
2. If `true` → skip modal, close tab immediately (existing `handleClose` flow).
3. If falsy (default) → show a window-scoped `ConfirmModal`:
   - **Title:** "Close tab?"
   - **Body:** name of the tab being closed (e.g. "Close 'My tab'?")
   - **Checkbox:** "Don't ask again" (unchecked by default)
   - **Buttons:** Cancel | Close tab (destructive)
4. **On confirm:**
   - If checkbox is checked → `RpcApi.SetConfigCommand(TabRpcClient, { "tab:skipcloseconfirm": true } as any)` then close tab.
   - If checkbox is unchecked → close tab immediately.
5. **On cancel:** do nothing, tab stays open.

### Settings schema addition

**`schema/settings.json`** — add under the `tab:` block:

```json
"tab:skipcloseconfirm": {
    "type": "boolean",
    "default": false,
    "description": "Skip the close-tab confirmation modal. Set automatically by the 'Don't ask again' checkbox."
}
```

### Implementation

#### A. `tabbar.tsx` — wrap `handleClose` in a modal gate

The existing `handleClose` (lines 70-82) directly calls `WorkspaceService.CloseTab`. Introduce an intermediate function `requestClose(tabId)` that:

1. Reads the skip-setting.
2. If set: calls `handleClose(tabId)` directly.
3. If not set: opens the confirmation modal.

Pass `requestClose` as the `onClose` prop to each tab (currently the prop is `handleClose`).

```typescript
// tabbar.tsx — new signal for pending close
const [pendingCloseTabId, setPendingCloseTabId] = createSignal<string | null>(null);

function requestClose(tabId: string) {
    const skip = settingsAtom()["tab:skipcloseconfirm"];
    if (skip) {
        handleClose(tabId);
    } else {
        setPendingCloseTabId(tabId);
    }
}
```

#### B. Modal JSX in `tabbar.tsx`

Render a `ConfirmModal` driven by `pendingCloseTabId()`. The modal is window-scoped so it overlays the full window (not just a tab pane).

```tsx
<Show when={pendingCloseTabId() !== null}>
    <TabCloseConfirmModal
        tabId={pendingCloseTabId()!}
        onConfirm={(skipFuture) => {
            if (skipFuture) {
                RpcApi.SetConfigCommand(TabRpcClient, { "tab:skipcloseconfirm": true } as any);
            }
            handleClose(pendingCloseTabId()!);
            setPendingCloseTabId(null);
        }}
        onCancel={() => setPendingCloseTabId(null)}
    />
</Show>
```

#### C. `TabCloseConfirmModal` component

New small component, can live in `tabbar.tsx` or a separate `tab-close-confirm-modal.tsx`.

```tsx
function TabCloseConfirmModal(props: {
    tabId: string;
    onConfirm: (skipFuture: boolean) => void;
    onCancel: () => void;
}) {
    const [skipFuture, setSkipFuture] = createSignal(false);
    const [tabData] = useWaveObjectValue<Tab>(makeORef("tab", props.tabId));
    const tabName = () => tabData()?.name ?? "this tab";

    return (
        <Modal scope="window" onClose={props.onCancel}>
            <ModalHeader title={`Close "${tabName()}"?`} />
            <ModalBody>
                <p>This tab and all its panes will be closed.</p>
                <label style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "12px", cursor: "pointer" }}>
                    <input
                        type="checkbox"
                        checked={skipFuture()}
                        onChange={(e) => setSkipFuture(e.currentTarget.checked)}
                    />
                    Don't ask again
                </label>
            </ModalBody>
            <ModalFooter>
                <Button className="ghost grey" onClick={props.onCancel}>Cancel</Button>
                <Button className="solid red" onClick={() => props.onConfirm(skipFuture())}>
                    Close tab
                </Button>
            </ModalFooter>
        </Modal>
    );
}
```

#### D. `Modal` scope note

The existing modal system (`frontend/app/element/modal.tsx`) supports `scope="window"`. This renders the modal via the window-level Portal, overlaying the full app content — correct for a destructive confirmation that blocks all tab interactions. The `inert` + scroll-lock machinery is already handled by the Modal component for this scope.

---

## Files to Modify

| File | Change |
|------|--------|
| `frontend/app/tab/tab.tsx` | SVG size 14→16; context-menu "Close" → dismiss only |
| `frontend/app/tab/tab.scss` | `.wave-button` container 16px→20px |
| `frontend/app/tab/tabbar.tsx` | `requestClose` gate, `pendingCloseTabId` signal, `TabCloseConfirmModal` usage |
| `schema/settings.json` | Add `tab:skipcloseconfirm` boolean key |

Optional (if modal is extracted):
| `frontend/app/tab/tab-close-confirm-modal.tsx` | New component |

---

## Edge Cases

- **Last tab:** `handleClose` already guards against closing the last tab (`allTabs.length <= 1`). `requestClose` should apply the same guard before even showing the modal — no point confirming a close that won't happen.
- **Keyboard close:** If there's a keyboard shortcut for closing a tab (Cmd/Ctrl+W equivalent), it should also route through `requestClose`.
- **Multiple windows:** `tab:skipcloseconfirm` is a global setting (shared across all windows, written to `settings.json`). This is intentional — the user's preference should be consistent.
- **Re-enabling the prompt:** Users can set `"tab:skipcloseconfirm": false` in `settings.json` to get the modal back. No UI needed for this edge case.
