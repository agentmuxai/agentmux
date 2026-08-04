// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SolidJS-compatible dimension/resize hooks (ported from React version).

import { onCleanup, onMount } from "solid-js";
import { debounce } from "throttle-debounce";

// Ref object shape compatible with SolidJS refs { current: T | null }
type RefObject<T> = { current: T | null };

// Watches a ref element for size changes and calls the callback with the new rect.
// Pass debounceMs of null to not debounce.
export function useOnResize<T extends HTMLElement>(
    ref: RefObject<T> | null | undefined,
    callback: (domRect: DOMRectReadOnly) => void,
    debounceMs: number = null
) {
    onMount(() => {
        if (!ref) return;
        let isFirst = true;
        const cb = debounceMs == null ? callback : debounce(debounceMs, callback);
        const rszObs = new ResizeObserver((entries) => {
            for (const entry of entries) {
                if (isFirst) {
                    isFirst = false;
                    callback(entry.contentRect);
                } else {
                    cb(entry.contentRect);
                }
            }
        });

        if (ref.current) {
            rszObs.observe(ref.current);
        }

        onCleanup(() => {
            rszObs.disconnect();
        });
    });
}
