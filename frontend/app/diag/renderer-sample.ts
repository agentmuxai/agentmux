// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Small reads of this renderer's own size, shared by the periodic lines that
 * a long session is read from (renderer-state-diagnostic.ts, the atom-cache
 * line, the tab-switch line). Each read is one property or one live-collection
 * length; none walks the page's data.
 */

/** The window this renderer shows, as the host knows it. */
export function windowLabel(): string {
    // A line that reports on a tab switch must never be the thing that throws, so no page is not an error.
    if (typeof window === "undefined") return "main";
    return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
}

export interface HeapMb {
    used: number;
    total: number;
    limit: number;
}

interface ChromiumMemory {
    usedJSHeapSize: number;
    totalJSHeapSize: number;
    jsHeapSizeLimit: number;
}

const MB = 1024 * 1024;

/** The JS heap in whole MB, or null where the engine doesn't report it. */
export function heapMb(): HeapMb | null {
    if (typeof performance === "undefined") return null;
    const m = (performance as unknown as { memory?: ChromiumMemory }).memory;
    if (!m || typeof m.usedJSHeapSize !== "number") return null;
    return {
        used: Math.round(m.usedJSHeapSize / MB),
        total: Math.round(m.totalJSHeapSize / MB),
        limit: Math.round(m.jsHeapSizeLimit / MB),
    };
}

/** Elements in the document. A live collection's length: no tree copy, a walk of the page once. */
export function domNodes(): number {
    if (typeof document === "undefined") return 0;
    return document.getElementsByTagName("*").length;
}
