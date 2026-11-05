// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Keeping the panel placed: the pointer's Y over the row, the panel's autoUpdate, and the first placement. */

import { autoUpdate } from "@floating-ui/dom";
import { createEffect, onCleanup } from "solid-js";
import type { PeekBridge } from "./peek-overlay-bridge";
import type { PeekPlacement } from "./peek-overlay-placement";
import type { PeekState } from "./peek-overlay-state";

/** Creates two effects: the mouse tracking, then the placement when shown. Returns the panel's ref callback. */
export function usePeekTracking(s: PeekState, p: PeekPlacement, b: PeekBridge) {
    // Track the mouse continuously while the row exists, independent of
    // `show` (the 50ms enter-delay in useNodePeek means `show` flips true
    // slightly after hover starts — this way the first visible frame
    // already has a real Y instead of one captured from a stale/absent
    // mousemove). rAF-coalesced so fast mouse movement doesn't write
    // styles once per raw event — same pattern `registerFloating` below
    // already uses for its own rAF-gated setup.
    createEffect(() => {
        const row = s.props.rowEl();
        if (!row) return;
        const onMouseMove = (e: MouseEvent) => {
            s.lastMouseY = e.clientY;
            if (s.mouseMoveRaf != null) return;
            s.mouseMoveRaf = requestAnimationFrame(() => {
                s.mouseMoveRaf = null;
                // A pinned panel does not follow the pointer (see pinnedOffsetY).
                if (s.show() && s.pinnedOffsetY == null) p.update();
            });
        };
        row.addEventListener("mousemove", onMouseMove);
        onCleanup(() => {
            row.removeEventListener("mousemove", onMouseMove);
            if (s.mouseMoveRaf != null) {
                cancelAnimationFrame(s.mouseMoveRaf);
                s.mouseMoveRaf = null;
            }
        });
    });

    // The RAF below must be cancellable and `floatingEl` reset (#2392): a rapid
    // hover→leave (very reachable given the 150ms enter-delay that mounts this
    // Portal, followed by a mouseleave within the same animation frame) could
    // otherwise let a stale RAF fire AFTER the `<Show>` had already unmounted
    // this div, and `autoUpdate(row, floatingEl, update)` would run anyway,
    // registering scroll/resize listeners nothing would ever clean up.
    // Registering `onCleanup` HERE, synchronously inside the ref callback,
    // attaches it to THIS specific `<Show>` branch's own reactive scope — it
    // fires on every unmount of this div (whether from `show` flipping false or
    // the whole component unmounting), 1:1 with `registerFloating`'s own
    // mounts, so there's no toggle-without-matching-cleanup gap left.
    const registerFloating = (el: HTMLElement) => {
        s.floatingEl = el;
        const rafId = requestAnimationFrame(() => {
            const row = s.props.rowEl();
            if (!row || !s.floatingEl) return;
            s.cleanupAutoUpdate?.();
            s.cleanupAutoUpdate = autoUpdate(row, s.floatingEl, p.update);
        });
        onCleanup(() => {
            cancelAnimationFrame(rafId);
            s.cleanupAutoUpdate?.();
            s.cleanupAutoUpdate = null;
            s.floatingEl = undefined;
            // Closed: the next opening places itself afresh, and nothing is
            // being crossed to any more.
            s.pinnedOffsetY = null;
            s.lastEnterable = false;
            b.clearBridgeTimer();
            b.releaseBridge();
        });
    };

    createEffect(() => {
        if (s.show()) {
            p.update();
        }
    });

    return { registerFloating };
}
