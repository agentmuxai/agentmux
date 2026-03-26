// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { PLATFORM, PlatformMacOS } from "@/util/platformutil";
import { computeBgStyleFromMeta } from "@/util/waveutil";
import type { JSX } from "solid-js";
import { createMemo, onCleanup, onMount } from "solid-js";
import { debounce } from "throttle-debounce";
import { atoms, getApi, WOS } from "./store/global";

export function AppBackground(): JSX.Element {
    let bgRef: HTMLDivElement;
    const tabData = createMemo(() => {
        const tabId = atoms.activeTabId();
        return WOS.getObjectValue<Tab>(WOS.makeORef("tab", tabId));
    });
    const style = createMemo(() => computeBgStyleFromMeta(tabData()?.meta, 0.5) ?? {});

    const getAvgColor = debounce(30, () => {
        if (
            bgRef &&
            PLATFORM !== PlatformMacOS &&
            "windowControlsOverlay" in window.navigator
        ) {
            const titlebarRect: Dimensions = (window.navigator.windowControlsOverlay as any).getTitlebarAreaRect();
            const bgRect = bgRef.getBoundingClientRect();
            if (titlebarRect && bgRect) {
                const windowControlsLeft = titlebarRect.width - titlebarRect.height;
                const windowControlsRect: Dimensions = {
                    top: titlebarRect.top,
                    left: windowControlsLeft,
                    height: titlebarRect.height,
                    width: bgRect.width - bgRect.left - windowControlsLeft,
                };
                getApi().updateWindowControlsOverlay(windowControlsRect);
            }
        }
    });

    onMount(() => {
        getAvgColor();
        const rszObs = new ResizeObserver(() => getAvgColor());
        if (bgRef) rszObs.observe(bgRef);
        onCleanup(() => rszObs.disconnect());
    });

    return <div ref={bgRef!} class="pointer-events-none absolute top-0 left-0 w-full h-full z-[var(--zindex-app-background)]" style={style() as any} />;
}
