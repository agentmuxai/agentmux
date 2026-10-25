// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How long each site's pages took to load the last few times a sleeping pane
 * woke, for the sleep card's estimate when the pane has no load time of its own
 * (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §7.2). Kept per origin in the
 * window's local storage, which every window of the instance shares and which
 * outlives a restart. A guess for a card, so a store that can't be read or
 * written just means no history.
 */

const STORE_KEY = "agentmux:browser-load-history";
/** Loads kept per origin; the estimate is their median. */
export const LOADS_PER_ORIGIN = 5;
/** Origins kept, the most recently loaded. */
export const MAX_ORIGINS = 200;
/** A load time outside this is a mistake in the clock, not a page. */
const MIN_PLAUSIBLE_MS = 20;
const MAX_PLAUSIBLE_MS = 120_000;

export type LoadHistory = Record<string, number[]>;

/** `scheme://host[:port]` of a web address, or undefined for anything else. */
export function originOf(url: string): string | undefined {
    try {
        const u = new URL(url);
        return u.protocol === "http:" || u.protocol === "https:" ? u.origin : undefined;
    } catch {
        return undefined;
    }
}

/** `history` with one more load for `origin`: the newest `LOADS_PER_ORIGIN` kept, the least recently loaded origins dropped. */
export function withLoad(history: LoadHistory, origin: string, ms: number): LoadHistory {
    if (!(ms >= MIN_PLAUSIBLE_MS && ms <= MAX_PLAUSIBLE_MS)) return history;
    const { [origin]: previous = [], ...others } = history;
    const next: LoadHistory = { ...others, [origin]: [...previous, Math.round(ms)].slice(-LOADS_PER_ORIGIN) };
    const origins = Object.keys(next);
    // Object keys keep insertion order, and `origin` was just put last.
    for (const old of origins.slice(0, Math.max(0, origins.length - MAX_ORIGINS))) delete next[old];
    return next;
}

/** The median of the loads kept for `origin`, or undefined with none. */
export function medianLoadMs(history: LoadHistory, origin: string): number | undefined {
    const loads = history[origin];
    if (!Array.isArray(loads) || loads.length === 0) return undefined;
    const sorted = loads.filter((n) => typeof n === "number" && n > 0).sort((a, b) => a - b);
    if (sorted.length === 0) return undefined;
    const mid = Math.floor(sorted.length / 2);
    return sorted.length % 2 === 1 ? sorted[mid] : Math.round((sorted[mid - 1] + sorted[mid]) / 2);
}

function read(): LoadHistory {
    try {
        const parsed = JSON.parse(globalThis.localStorage?.getItem(STORE_KEY) ?? "null");
        return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as LoadHistory) : {};
    } catch {
        return {};
    }
}

/** Note that the page at `url` took `ms` to load. */
export function recordLoad(url: string, ms: number): void {
    const origin = originOf(url);
    if (!origin) return;
    try {
        globalThis.localStorage?.setItem(STORE_KEY, JSON.stringify(withLoad(read(), origin, ms)));
    } catch {
        // Storage full or unavailable: the estimate falls back to its default.
    }
}

/** About how long the page at `url` takes to load, from its site's recent loads. */
export function estimatedLoadMs(url: string): number | undefined {
    const origin = originOf(url);
    return origin ? medianLoadMs(read(), origin) : undefined;
}
