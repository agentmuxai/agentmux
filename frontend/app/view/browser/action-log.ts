// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How the header's Log shows one entry of a driven pane's action log
// (PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md §5).

/** The entry's time of day, local, to the second. */
export function actionTime(ts: number): string {
    const d = new Date(ts);
    const two = (n: number) => String(n).padStart(2, "0");
    return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** `auto-approve-on` → `auto-approve on`. */
export function actionKind(kind: string): string {
    return kind.replace(/-(on|off)$/, " $1");
}

/** A page address shortened for the list: its host and path, without the
 *  scheme, query or fragment, cut to `max` characters. */
export function shortUrl(url: string | undefined, max = 48): string {
    if (!url) return "";
    let out = url;
    try {
        const u = new URL(url);
        out = u.host ? u.host + (u.pathname === "/" ? "" : u.pathname) : url;
    } catch {
        // Not a URL the parser takes: show it as it is, cut.
    }
    return out.length > max ? out.slice(0, max - 1) + "…" : out;
}

/** An entry that didn't happen: refused, declined or timed out. */
export function actionFailed(how: string): boolean {
    return how.startsWith("refused") || how === "declined" || how === "timed out";
}
