// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Browser-pane hibernation: a hidden browser pane closes its page to free its
 * renderer, keeps what it needs to come back (address, title, icon, cookie
 * jar), and wakes when it is about to be seen, showing a card that says it is
 * restoring, about how long that takes, and why it slept
 * (docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md).
 *
 * A hidden pane sleeps after `browser:hibernateafter` minutes (sooner under
 * memory pressure), a pane in a tab that isn't shown at launch starts asleep,
 * and the person can put hidden panes to sleep by command. A pane wakes on
 * show, on a hover or press on its window tab's pill, or when an agent's
 * Browser call reaches it.
 */

import { getApi } from "@/app/store/app-api";
import { getSettingsKeyAtom } from "@/store/global";
import { createSignal } from "solid-js";

/** Block meta: present while the pane is asleep or waking (spec §4). */
export const SLEEP_META_KEY = "browser:sleep";
/** Block meta: how long the pane's page took to load the last time (ms). */
export const LOADMS_META_KEY = "browser:loadms";
/** Block meta srv writes to ask the pane's view to wake it, for an agent's Browser call (spec §8). */
export const WAKE_META_KEY = "browser:wake";
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
    /** What its renderers held when it went to sleep, in bytes, for the card; none where the host can't tell. */
    memoryBytes?: number;
    /** Discarded in place (the CEF Discard API): its history, scroll position
     *  and form contents come back with it. Otherwise its page was closed. */
    kept?: boolean;
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
        memoryBytes: typeof s.memoryBytes === "number" && s.memoryBytes > 0 ? s.memoryBytes : undefined,
        kept: s.kept === true,
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

/** "180 MB", "1.2 GB": a measured size, rounded for a sentence (10 MB steps from 100 MB, whole MB below). */
export function memoryLabel(bytes: number): string {
    const mb = bytes / (1024 * 1024);
    if (mb >= 1000) return `${(mb / 1024).toFixed(1)} GB`;
    if (mb >= 100) return `${Math.round(mb / 10) * 10} MB`;
    return `${Math.max(1, Math.round(mb))} MB`;
}

/** Why it slept, in one sentence (spec §7.1). */
export function sleepReasonText(reason: SleepReason, memoryBytes?: number): string {
    const saved = memoryBytes != null && memoryBytes > 0 ? `about ${memoryLabel(memoryBytes)}` : "usually 100 to 200 MB";
    const why = `A sleeping pane frees its page's memory, ${saved}, and reloads when you come back to it.`;
    if (reason === "pressure")
        return `Your computer was low on memory, so AgentMux put this browser pane to sleep. ${why}`;
    if (reason === "launch")
        return `AgentMux started this browser pane asleep, since it wasn't on screen, to save memory. ${why}`;
    return `AgentMux put this browser pane to sleep to save memory. ${why}`;
}

/** What a wake doesn't bring back when the page was closed rather than
 *  discarded in place (a browser engine without the Discard API). */
export const NOT_KEPT_TEXT =
    "Back and forward history, scroll position and anything typed into the page aren't kept.";

/** What to do once the host answered a discard of a pane going to sleep:
 *  `kept` (discarded in place), `close` (no Discard API, or it refused), or,
 *  when the pane was woken meanwhile, `load` (it was discarded: load it back)
 *  or `nothing`. */
export function afterDiscard(
    answer: { supported: boolean; ok: boolean } | null | undefined,
    stillAsleep: boolean
): "kept" | "close" | "load" | "nothing" {
    if (!stillAsleep) return answer?.ok ? "load" : "nothing";
    return answer?.supported && answer.ok ? "kept" : "close";
}

/** Whether a wake that has finished (its page loaded, or needed no load) ends the sleep now:
 *  the pane is shown, or the wake was asked for (a command, an agent's call). A hover's
 *  speculative wake that finished while the pane is still hidden is held instead, and goes
 *  back to sleep if no switch follows. */
export function wakeEndsNow(visible: boolean, why: string): boolean {
    return visible || why === "command" || why === "agent";
}

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

// ── When a hidden pane sleeps (spec §5) ────────────────────────────────────

export type Pressure = "normal" | "warn" | "critical";

/** Minutes hidden before a pane sleeps, unless `browser:hibernateafter` says otherwise. */
export const HIBERNATE_AFTER_MIN = 30;
/** Minutes hidden before a pane sleeps when memory is short (spec §5.2). */
export const PRESSURE_AFTER_MIN = { warn: 5, critical: 1 } as const;
/** How many panes go to sleep per policy tick, oldest-hidden first (spec §5.2). */
export const PER_TICK: Record<Pressure, number> = { normal: 1, warn: 3, critical: Number.POSITIVE_INFINITY };
/** How often the window looks for panes due to sleep. */
export const POLICY_TICK_MS = 30_000;
/** Minutes hidden before an awake pane is frozen (no script, no timers) until shown. */
export const FREEZE_AFTER_MIN = 5;
/** How many panes are frozen per policy tick: cheap, but each takes a page check. */
export const FREEZE_PER_TICK = 4;
/** An agent's Browser call this recent keeps its pane awake (spec §5.3). */
export const AGENT_RECENT_MS = 10 * 60_000;
/** A pane that mounts hidden this soon after the window's first load starts asleep (spec §5.4). */
export const LAUNCH_WINDOW_MS = 20_000;

/** `browser:hibernate`: on unless false. */
export function hibernationEnabled(): boolean {
    return getSettingsKeyAtom("browser:hibernate")() !== false;
}

/** How long a pane must have been hidden to sleep, at this pressure. */
export function hibernateAfterMs(pressure: Pressure, settingMin?: unknown): number {
    const normal = typeof settingMin === "number" && settingMin >= 1 ? settingMin : HIBERNATE_AFTER_MIN;
    const min = pressure === "normal" ? normal : Math.min(normal, PRESSURE_AFTER_MIN[pressure]);
    return min * 60_000;
}

/** A local dev server's page: reloading it can rerun the developer's app state. */
export function isLocalAddress(url: string): boolean {
    try {
        const host = new URL(url).hostname.replace(/^\[|\]$/g, "");
        return (
            host === "localhost" ||
            host.endsWith(".localhost") ||
            host === "::1" ||
            host === "0.0.0.0" ||
            /^127\.\d+\.\d+\.\d+$/.test(host)
        );
    } catch {
        return false;
    }
}

/** `url`'s site is in `browser:keepawakesites` (a host, its subdomains, or an origin). */
export function siteKeptAwake(url: string, sites: unknown): boolean {
    if (!Array.isArray(sites) || sites.length === 0) return false;
    let u: URL;
    try {
        u = new URL(url);
    } catch {
        return false;
    }
    return sites.some((raw) => {
        if (typeof raw !== "string" || !raw.trim()) return false;
        const site = raw.trim().toLowerCase();
        if (site.includes("://")) return u.origin.toLowerCase() === site.replace(/\/+$/, "");
        return u.hostname === site || u.hostname.endsWith(`.${site}`);
    });
}

/** A typed keep-awake site as `siteKeptAwake` matches it: an origin
 *  (`https://app.example.com:8443`) when a scheme was given, else a host
 *  (`example.com`, which covers its subdomains). A pasted address is cut to
 *  its host. Null when it names no site. */
export function normalizeKeepAwakeSite(input: string): string | null {
    const s = input.trim().toLowerCase();
    if (!s) return null;
    try {
        if (s.includes("://")) {
            const origin = new URL(s).origin;
            return origin === "null" ? null : origin;
        }
        const host = new URL(`https://${s}`).hostname;
        return host.includes(".") || host === "localhost" ? host : null;
    } catch {
        return null;
    }
}

/** What the window knows about a pane that can keep it awake (spec §5.3). */
export interface PaneFacts {
    url: string;
    keepAwake: boolean;
    /** An agent drives it (`browser:owner_agent`). */
    driverAgent: boolean;
    /** Auto-approve is on for its agent. */
    bypass: boolean;
    /** A banner is waiting on the person (a hand-off, an approval, a page dialog). */
    attention: boolean;
    /** Its page has popup windows open. */
    popupWindows: boolean;
    /** When an agent last acted on it (ms), if ever. */
    lastAgentActionAt?: number;
    /** Its page is capturing camera, microphone or screen. */
    capturing: boolean;
    /** Its title or icon changed while it was hidden: the page is doing something. */
    activeWhileHidden: boolean;
}

/** Why the window keeps this pane awake, or null when nothing does. */
export function awakeReason(f: PaneFacts, now: number, keepAwakeSites?: unknown): string | null {
    if (f.keepAwake) return "kept awake";
    if (!f.url || f.url === "about:blank") return "no page";
    if (isLocalAddress(f.url)) return "local address";
    if (siteKeptAwake(f.url, keepAwakeSites)) return "site kept awake";
    if (f.driverAgent) return "an agent drives it";
    if (f.bypass) return "auto-approve is on";
    if (f.attention) return "waiting on the person";
    if (f.popupWindows) return "popup windows open";
    if (f.lastAgentActionAt != null && now - f.lastAgentActionAt < AGENT_RECENT_MS) return "an agent used it recently";
    if (f.capturing) return "capturing media";
    if (f.activeWhileHidden) return "active while hidden";
    return null;
}

/** Whether this browser engine can freeze a page and show it again. CDP's
 *  freeze marks the page hidden and its thaw doesn't mark it shown, so only an
 *  engine with the AgentMux CEF fork's windowed WasHidden, which the pane's
 *  show calls, brings a thawed page back to full speed. That fork build is the
 *  one with the Discard API, so its presence answers. A yes is kept for the window.
 *  A no is not final: the host also answers "unsupported" when the probed pane has no
 *  live page or the UI thread was slow, so it is asked again after a while. */
export const ENGINE_NO_RECHECK_MS = 5 * 60_000;
let engineCanFreeze: Promise<boolean> | undefined;
let engineNoAt = 0;
export function resetEngineSupportsFreeze(): void {
    engineCanFreeze = undefined;
    engineNoAt = 0;
}
export function engineSupportsFreeze(
    ask: () => Promise<{ supported: boolean } | null | undefined>,
    now: number = Date.now(),
): Promise<boolean> {
    if (engineCanFreeze && engineNoAt > 0 && now - engineNoAt >= ENGINE_NO_RECHECK_MS) resetEngineSupportsFreeze();
    engineCanFreeze ??= ask()
        .then((r) => {
            if (r?.supported === true) return true;
            engineNoAt = now;
            return false;
        })
        .catch(() => {
            resetEngineSupportsFreeze(); // ask again next time
            return false;
        });
    return engineCanFreeze;
}

/** Whether the window believes the host has this pane's page frozen. The host
 *  thaws a page only when told to or on a CDP call, so the flag clears only once
 *  the host's thaw has been answered; a thaw that failed leaves it set, and the
 *  next one tries again. Callers that are about to look at the page `await thaw()`. */
export function createFreezeState(setHostFrozen: (frozen: boolean) => Promise<unknown>) {
    let frozen = false;
    let thawing: Promise<void> | null = null;
    return {
        isFrozen: () => frozen,
        markFrozen: () => {
            frozen = true;
        },
        thaw(): Promise<void> {
            if (!frozen) return thawing ?? Promise.resolve();
            thawing ??= setHostFrozen(false).then(
                () => {
                    frozen = false;
                    thawing = null;
                },
                () => {
                    thawing = null;
                },
            );
            return thawing;
        },
    };
}

/** What in the page keeps a hidden pane from being frozen: only what a frozen
 *  page would break while it waits (media playing, DevTools attached). A typed
 *  form or a leave-page prompt is fine: freezing keeps the page as it is. */
export function freezeAwakeReason(c: BrowserHibernationCheck | null | undefined): string | null {
    if (!c) return "page not checked";
    if (!c.live) return "no live page";
    if (c.media) return "playing media";
    if (c.devtools) return "DevTools open";
    return null;
}

/** Why the page keeps the pane awake, from the host's check; a check that
 *  couldn't run keeps it awake too. */
export function pageAwakeReason(c: BrowserHibernationCheck | null | undefined): string | null {
    if (!c) return "page not checked";
    if (!c.live) return "no live page";
    if (c.media) return "playing media";
    if (c.edited) return "something typed into it";
    if (c.beforeunload) return "the page asks before leaving";
    if (c.devtools) return "DevTools open";
    return null;
}

/** This window's page loaded (not reloaded) less than LAUNCH_WINDOW_MS ago. */
export function isLaunchMount(): boolean {
    try {
        const nav = performance.getEntriesByType?.("navigation")?.[0] as PerformanceNavigationTiming | undefined;
        if (nav?.type === "reload") return false;
        return performance.now() < LAUNCH_WINDOW_MS;
    } catch {
        return false;
    }
}

// ── Media capture, from the host's events ──────────────────────────────────

const capturingPanes = new Set<string>();
let captureListening = false;

function listenCapture(): void {
    if (captureListening) return;
    captureListening = true;
    try {
        void getApi()
            ?.listen<{ blockId?: string; hasVideo?: boolean; hasAudio?: boolean }>(
                "pane-media-capture-changed",
                (p) => {
                    if (!p || typeof p.blockId !== "string") return;
                    if (p.hasVideo || p.hasAudio) capturingPanes.add(p.blockId);
                    else capturingPanes.delete(p.blockId);
                }
            )
            ?.catch(() => {});
    } catch {
        // No host API (a test): nothing captures.
    }
}

/** The pane's page is capturing camera, microphone or screen. */
export function isCapturing(blockId: string): boolean {
    listenCapture();
    return capturingPanes.has(blockId);
}

// ── The window's browser panes ─────────────────────────────────────────────

export interface HibernatablePane {
    blockId: string;
    /** The pane can be seen now (its window tab shown, its pane tab active). */
    isVisible(): boolean;
    isAsleep(): boolean;
    /** When it was last hidden (ms), or null while visible. */
    hiddenSince(): number | null;
    /** Put it to sleep unless something keeps it awake; resolves to that reason, or null when it slept. */
    tryHibernate(reason: SleepReason): Promise<string | null>;
    wake(why: string): void;
    /** Frozen: awake, but its page runs nothing until it is shown again. */
    isFrozen?(): boolean;
    /** Freeze it unless something keeps it running; resolves to that reason, or null when frozen. */
    tryFreeze?(): Promise<string | null>;
}

const panes = new Set<HibernatablePane>();
let policyTimer: ReturnType<typeof setInterval> | undefined;

export function registerHibernatablePane(p: HibernatablePane): () => void {
    panes.add(p);
    if (!policyTimer && typeof setInterval === "function") {
        policyTimer = setInterval(() => void policyTick().catch(() => {}), POLICY_TICK_MS);
    }
    return () => {
        panes.delete(p);
        if (panes.size === 0 && policyTimer) {
            clearInterval(policyTimer);
            policyTimer = undefined;
        }
    };
}

/** The memory each of these panes holds, by block id; empty when the host can't tell. */
async function hostPaneMemory(blockIds: string[]): Promise<Record<string, number>> {
    try {
        return (await getApi()?.browserPanes?.paneMemory?.(blockIds)) ?? {};
    } catch {
        return {};
    }
}

/** `due` with the biggest pages first, then the ones with no figure, each group longest-hidden first. */
export function bySizeThenAge<T extends { blockId: string; hiddenSince(): number | null }>(
    due: T[],
    memory: Record<string, number>
): T[] {
    const size = (p: T) => (typeof memory[p.blockId] === "number" ? memory[p.blockId] : -1);
    return [...due].sort((a, b) => size(b) - size(a) || (a.hiddenSince() ?? 0) - (b.hiddenSince() ?? 0));
}

async function hostPressure(): Promise<Pressure> {
    try {
        return (await getApi()?.browserPanes?.memoryPressure?.()) ?? "normal";
    } catch {
        return "normal";
    }
}

/**
 * One pass of the policy: the hidden panes due to sleep at the host's memory
 * pressure, at most PER_TICK of them: oldest-hidden first, or under pressure
 * (when the few that sleep matter) the biggest pages first. Each still checks
 * what keeps it awake (`tryHibernate`). Resolves to the panes that slept.
 */
export async function policyTick(
    now = Date.now(),
    pressureOf: () => Promise<Pressure> = hostPressure,
    memoryOf: (blockIds: string[]) => Promise<Record<string, number>> = hostPaneMemory
): Promise<string[]> {
    if (!hibernationEnabled()) return [];
    const candidates = [...panes].filter((p) => !p.isAsleep() && !p.isVisible() && p.hiddenSince() != null);
    if (candidates.length === 0) return [];
    const pressure = await pressureOf();
    const after = hibernateAfterMs(pressure, getSettingsKeyAtom("browser:hibernateafter")());
    let due = candidates
        .filter((p) => now - (p.hiddenSince() ?? now) >= after)
        .sort((a, b) => (a.hiddenSince() ?? 0) - (b.hiddenSince() ?? 0));
    // Only more due panes than may sleep this tick make the order matter.
    if (pressure !== "normal" && due.length > PER_TICK[pressure]) {
        due = bySizeThenAge(due, await memoryOf(due.map((p) => p.blockId)));
    }
    const slept: string[] = [];
    for (const p of due) {
        if (slept.length >= PER_TICK[pressure]) break;
        const declined = await p.tryHibernate(pressure === "normal" ? "hidden" : "pressure");
        if (declined == null) slept.push(p.blockId);
    }
    // Hidden long enough to freeze, not yet due to sleep (or kept awake): freeze.
    const freezable = candidates
        .filter((p) => !slept.includes(p.blockId) && p.tryFreeze && !p.isFrozen?.() && !p.isAsleep())
        .filter((p) => now - (p.hiddenSince() ?? now) >= FREEZE_AFTER_MIN * 60_000)
        .sort((a, b) => (a.hiddenSince() ?? 0) - (b.hiddenSince() ?? 0));
    let frozen = 0;
    for (const p of freezable) {
        if (frozen >= FREEZE_PER_TICK) break;
        if ((await p.tryFreeze!()) == null) frozen++;
    }
    return slept;
}

/** Hibernate every browser pane in this window that can't be seen. */
export async function hibernateHiddenBrowserPanes(): Promise<number> {
    let n = 0;
    for (const p of panes) {
        if (p.isVisible() || p.isAsleep()) continue;
        if ((await p.tryHibernate("manual")) == null) n++;
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
