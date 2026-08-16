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
// synthetic mousedown + pointerdown on `document` whenever it fires — every
// existing listener treats that exactly like a real outside click, with no
// changes needed on their end.

import { listenEvent } from "@/app/platform/ipc";
import { onCleanup, onMount } from "solid-js";

export const BrowserPaneOutsideClickBridge = () => {
    onMount(() => {
        const unsubPromise = listenEvent<{ block_id: string }>("browser-pane-clicked", () => {
            // No block_id filtering — ANY pane click counts as "outside" for
            // every currently open dismissible menu, the same way clicking
            // anywhere else non-menu in the app already does.
            //
            // Dispatched ON `document` (not `document.body`) so `e.target`
            // is `document` itself: never `.contains()`-matched by any
            // menu/button ref, and `instanceof Element` is false, so
            // existing handlers' `el?.closest(".popover-menu")` fallback
            // also safely no-ops instead of throwing. Both event types are
            // dispatched because existing listeners are a mix of the two.
            for (const type of ["mousedown", "pointerdown"] as const) {
                document.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true }));
            }
        });
        onCleanup(() => {
            void unsubPromise.then((unsub) => unsub());
        });
    });
    return null;
};
