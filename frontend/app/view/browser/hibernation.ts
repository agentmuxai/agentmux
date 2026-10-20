// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Browser-pane hibernation: a hidden browser pane closes its page to free its
 * renderer, keeps what it needs to come back (address, title, icon, cookie
 * jar), and wakes when it is about to be seen, showing a card that says it is
 * restoring, about how long that takes, and why it slept
 * (docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md).
 *
 * Phase 0 (the spike): panes sleep only on the Hibernate command; they wake on
 * show and on a hover over their window tab's pill.
 */

import { createSignal } from "solid-js";

/** Block meta: present while the pane is asleep or waking (spec §4). */
export const SLEEP_META_KEY = "browser:sleep";
/** Block meta: how long the pane's page took to load the last time (ms). */
export const LOADMS_META_KEY = "browser:loadms";
/** Block meta: the person asked to keep this pane awake. */
export const KEEPAWAKE_META_KEY = "browser:keepawake";

export type SleepReason = "hidden" | "pressure" | "launch" | "manual";

export interface BrowserSleep {
    since: number;
    reason: SleepReason;
    url: string;
    title: string;
    favicon: string;
    /** The page's last load time, for the card's estimate. */
    loadMs?: number;
}

/** The meta value, or undefined when it isn't a sleep record. */
export function readSleep(v: unknown): BrowserSleep | undefined {
    if (!v || typeof v !== "object") return undefined;
    const s = v as Partial<BrowserSleep>;
    if (typeof s.since !== "number" || typeof s.url !== "string") return undefined;
    return {
        since: s.since,
        reason: (["hidden", "pressure", "launch", "manual"] as const).includes(s.reason as SleepReason)
            ? (s.reason as SleepReason)
            : "manual",
        url: s.url,
        title: typeof s.title === "string" ? s.title : "",
        favicon: typeof s.favicon === "string" ? s.favicon : "",
        loadMs: typeof s.loadMs === "number" && s.loadMs > 0 ? s.loadMs : undefined,
    };
}

/** The estimate when a page has no load time of its own yet (spec §7.2). */
export const DEFAULT_WAKE_MS = 2000;

/** "About 2 seconds" (spec §7.2). */
export function etaLabel(ms: number | undefined): string {
    const s = (ms ?? DEFAULT_WAKE_MS) / 1000;
    if (s > 3.5) return "A few seconds";
    const n = Math.max(1, Math.round(s));
    return n === 1 ? "About 1 second" : `About ${n} seconds`;
}

/** Past the estimate by more than half: say so instead of a stale number. */
export function isOverdue(ms: number | undefined, elapsed: number): boolean {
    return elapsed > (ms ?? DEFAULT_WAKE_MS) * 1.5;
}

/** Why it slept, in one sentence (spec §7.1). */
export function sleepReasonText(reason: SleepReason): string {
    return reason === "pressure"
        ? "Your computer was low on memory, so AgentMux put this browser pane to sleep. A sleeping pane frees its page's memory, usually 100 to 200 MB, and reloads when you come back to it."
        : "AgentMux put this browser pane to sleep to save memory. A sleeping pane frees its page's memory, usually 100 to 200 MB, and reloads when you come back to it.";
}

/** What a wake doesn't bring back yet (until the in-place discard, spec §10 Phase 3). */
export const NOT_KEPT_TEXT =
    "Back and forward history, scroll position and anything typed into the page aren't kept yet.";

/** The name the card gives the page: its saved title, else its address. */
export function sleepName(s: BrowserSleep): string {
    const title = s.title.trim();
    if (title && title !== "Browser") return title;
    try {
        const u = new URL(s.url);
        return `${u.host}${u.pathname === "/" ? "" : u.pathname}`;
    } catch {
        return s.url;
    }
}

// ── Wake on intent (spec §6) ───────────────────────────────────────────────

/** How long the pointer rests on a tab pill before its panes start waking. */
export const HOVER_WAKE_DWELL_MS = 100;
/** A wake on hover with no switch goes back to sleep after this, unless loaded. */
export const HOVER_WAKE_GIVE_UP_MS = 10_000;

const [prewakeTab, setPrewakeTab] = createSignal<string | null>(null);
/** The window tab whose sleeping browser panes should start waking. */
export const prewakeTabId = prewakeTab;

let dwell: ReturnType<typeof setTimeout> | undefined;

/** The pointer entered a window tab's pill. */
export function hoverTabPill(tabId: string): void {
    clearTimeout(dwell);
    dwell = setTimeout(() => setPrewakeTab(tabId), HOVER_WAKE_DWELL_MS);
}

/** The pointer pressed a window tab's pill: wake at once. */
export function pressTabPill(tabId: string): void {
    clearTimeout(dwell);
    setPrewakeTab(tabId);
}

/** The pointer left a window tab's pill. */
export function leaveTabPill(tabId: string): void {
    clearTimeout(dwell);
    if (prewakeTab() === tabId) setPrewakeTab(null);
}

// ── The window's browser panes, for the commands ───────────────────────────

export interface HibernatablePane {
    blockId: string;
    /** The pane can be seen now (its window tab shown, its pane tab active). */
    isVisible(): boolean;
    isAsleep(): boolean;
    hibernate(reason: SleepReason): void;
    wake(why: string): void;
}

const panes = new Set<HibernatablePane>();

export function registerHibernatablePane(p: HibernatablePane): () => void {
    panes.add(p);
    return () => panes.delete(p);
}

/** Hibernate every browser pane in this window that can't be seen. */
export function hibernateHiddenBrowserPanes(): number {
    let n = 0;
    for (const p of panes) {
        if (p.isVisible() || p.isAsleep()) continue;
        p.hibernate("manual");
        n++;
    }
    console.log(`[hibernate] command hibernate-hidden count=${n}`);
    return n;
}

/** Wake every sleeping browser pane in this window. */
export function wakeAllBrowserPanes(): number {
    let n = 0;
    for (const p of panes) {
        if (!p.isAsleep()) continue;
        p.wake("command");
        n++;
    }
    console.log(`[hibernate] command wake-all count=${n}`);
    return n;
}
