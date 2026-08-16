// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Makes a click inside ANY browser pane dismiss whatever dismissible
// menu/popover is currently open, the same way a click anywhere else in the
// app already does. See docs/specs/SPEC_BROWSER_PANE_CLICK_DISMISSES_MENUS_2026_08_15.md.
//
// Root cause: a browser pane's content is a native, sibling CefBrowserView
// layered on top of the DOM via CEF's Views AddOverlayView (see
// docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md) — a click there never
// becomes a DOM mousedown/pointerdown event, so it can't reach any of the
// app's ~18 independent "outside click" listeners (action-widgets.tsx,
// context-menu.tsx, popover-menu.tsx, the status-bar popovers, etc.), all of
// which close on document-level mousedown/pointerdown.
//
// Fix: `browser-pane-clicked` already exists as a backend->frontend event
// for exactly this native-click gap (added for pane click-to-select, see
// docs/specs/SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07.md). Instead of
// teaching every listener about it individually, this dispatches a
// synthetic mousedown + pointerdown on `document.body` whenever it fires —
// every existing listener treats that exactly like a real outside click,
// with no changes needed on their end.

import { listenEvent } from "@/app/platform/ipc";
import { onCleanup, onMount } from "solid-js";

export const BrowserPaneOutsideClickBridge = () => {
    onMount(() => {
        const unsubPromise = listenEvent<{ block_id: string }>("browser-pane-clicked", () => {
            // No block_id filtering — ANY pane click counts as "outside" for
            // every currently open dismissible menu, the same way clicking
            // anywhere else non-menu in the app already does.
            //
            // Dispatched on `document.body`, NOT `document` itself
            // (reagentx P1 on PR #2597): several existing listeners
            // (useWindowDrag.*.ts's isInDragRegion) call DOM methods like
            // `getAttribute` on `e.target`, which `Document` doesn't have —
            // dispatching on `document` threw an uncaught TypeError on
            // every browser-pane click. `document.body` is a real Element,
            // so those calls are safe, while still reading as "outside" for
            // every existing check: `.contains()` only matches descendants
            // (body is never a descendant of a menu/popover), and
            // `body.closest(".popover-menu")` walks ancestors, which body
            // has none of, so it correctly returns null. Both event types
            // are dispatched because existing listeners are a mix of the
            // two — see sound-service.ts for why `pointerdown` alone needed
            // a companion fix (isTrusted guard on its autoplay-prime
            // listener) rather than being dropped here.
            for (const type of ["mousedown", "pointerdown"] as const) {
                document.body.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true }));
            }
        });
        onCleanup(() => {
            void unsubPromise.then((unsub) => unsub());
        });
    });
    return null;
};
