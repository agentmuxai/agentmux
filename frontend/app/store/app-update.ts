// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a newer AgentMux release exists, as srv learns it from GitHub
 * Releases (crates/srv/src/backend/update/check.rs), which srv asks in the
 * background (`update/auto.rs`) and when the user clicks Check for update or an
 * agent asks: `app.update-status` when a window opens,
 * kept current by the `app:updatestatus` event. The status-bar chip and the
 * version panel's Maintenance section read it.
 *
 * A Microsoft Store install is the exception: the Store, not GitHub, says
 * whether there is an update and which version it is (the host's
 * `store_update_check`, asked from here on srv's own cadence and on click).
 * GitHub is then only where that version's release notes are linked from.
 * docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §3.
 */

import { pushNotification } from "@/app/store/flash-notifications";
import { createEffect, createSignal, onCleanup } from "solid-js";

import { WpsEvent } from "@/app/store/mps-events";
import { getSettingsKeyAtom } from "@/store/global";
import { getApi } from "@/util/app-api";
import { formatTimeOrDate } from "@/util/format-time";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AppUpdateStatus } from "@/types/rpc/AppUpdateStatus";
import type { DownloadStatus } from "@/types/rpc/DownloadStatus";
import type { WaitingOn } from "@/types/rpc/WaitingOn";

export type { AppUpdateStatus } from "@/types/rpc/AppUpdateStatus";

/** The `app:updatestatus` WebSocket event. */
export const APP_UPDATE_EVENT = WpsEvent.AppUpdateStatus;

const STATES = new Set(["up-to-date", "available", "local-build", "unknown"]);

const [appUpdate, setAppUpdate] = createSignal<AppUpdateStatus | null>(null);

/** What srv last said, or null before it has. */
export { appUpdate };

/**
 * How far back a status can have been overtaken. The answer to a click and the event for the change after it
 * cross by milliseconds; a status older than this is another run of srv, or a clock that was set back, and is taken.
 */
const OVERTAKEN_WITHIN_MS = 10 * 60 * 1000;

/**
 * Whether a status numbered `next` is older than the one numbered `have` (srv's `rev`, which rises with every
 * change). 0 is a status that doesn't say, which is taken as it comes.
 */
export function overtaken(next: number, have: number): boolean {
    return next > 0 && have > 0 && next < have && have - next < OVERTAKEN_WITHIN_MS;
}

/**
 * Take a status from `app.update-status`, a click's answer or the event; ignores anything malformed, and a status
 * srv has already replaced: a click's answer ("waiting") can arrive after the event for what came next
 * ("restarting"), and must not put the window back.
 */
export function acceptAppUpdate(next: unknown): void {
    const s = next as Partial<AppUpdateStatus> | null | undefined;
    if (typeof s?.state !== "string" || !STATES.has(s.state) || typeof s.current !== "string") return;
    if (overtaken(s.rev ?? 0, appUpdate()?.rev ?? 0)) return;
    setAppUpdate(s as AppUpdateStatus);
}

/**
 * Check for update: srv asks GitHub now (at most every 30 seconds) and every
 * window gets the answer. Throws when GitHub can't be reached. A Store install
 * asks the Store instead, whose answer (never an error) is shown as it is.
 */
export async function checkForUpdate(): Promise<void> {
    if (!viaStore()) acceptAppUpdate(await RpcApi.AppUpdateCheckCommand(TabRpcClient));
    // A Store install is updated by the Store, which has its own idea of the latest version.
    if (viaStore()) await askStore();
}

/** Ask srv for the status now (a window opened after the last event). */
export async function loadAppUpdate(): Promise<void> {
    try {
        acceptAppUpdate(await RpcApi.AppUpdateStatusCommand(TabRpcClient));
    } catch {
        // An older srv without the command: no update news.
    }
}

/** A Microsoft Store install: the Store, not AgentMux, updates it. */
export function viaStore(s: AppUpdateStatus | null = appUpdate()): boolean {
    return s?.install === "windows-store";
}

/** Whether dotted numeric version `a` is newer than `b` (numbers, not text: 0.60.100 is newer than 0.60.17); a missing part counts as 0. */
function isNewer(a: string, b: string): boolean {
    const pa = a.split(".").map((n) => Number.parseInt(n, 10) || 0);
    const pb = b.split(".").map((n) => Number.parseInt(n, 10) || 0);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
        const d = (pa[i] ?? 0) - (pb[i] ?? 0);
        if (d !== 0) return d > 0;
    }
    return false;
}

/** What the Store itself said: `asking` until it answers; `no_answer` when it never did; `unknown` carries why it couldn't be asked. `version` is the update's version when `available` and known (empty when the Store gave none, or gave this build's own); `checkedMs` is when it last answered. */
export type StoreOffer = { state: "asking" | "available" | "none" | "no_answer" | "unknown"; version: string; message: string; checkedMs: number };

const [storeOffer, setStoreOffer] = createSignal<StoreOffer | null>(null);
let storeAsk: Promise<void> | null = null;

/**
 * Ask the Store (through the host) whether it lists an update, instead of guessing from GitHub. `quiet`: keep
 * showing the last answer until the new one comes. Asks made while one is out share it.
 */
export function askStore(quiet = false): Promise<void> {
    storeAsk ??= (async () => {
        if (!quiet) setStoreOffer({ state: "asking", version: "", message: "", checkedMs: storeOffer()?.checkedMs ?? 0 });
        let next: Omit<StoreOffer, "checkedMs">;
        try {
            const r = await getApi()?.approvals?.storeUpdateCheck?.();
            next = r
                ? { state: r.state, version: r.version ?? "", message: r.message }
                : { state: "unknown", version: "", message: "this window can't ask the Store" };
        } catch (e: any) {
            next = { state: "unknown", version: "", message: String(e?.message ?? e) };
        }
        // Listed means there is an update. The version comes from the listed item's package, which Windows
        // documents as "the package that has an update available": it can be the installed one. So a version
        // that isn't newer than this build is not the update's version; the update is offered without one.
        const current = appUpdate()?.current;
        if (next.state === "available" && next.version && current && !isNewer(next.version, current)) {
            next = { ...next, version: "" };
        }
        setStoreOffer({ ...next, checkedMs: Date.now() });
    })().finally(() => {
        storeAsk = null;
    });
    return storeAsk;
}

/** Forget the Store's answer: the next look asks again. */
export function forgetStoreOffer(): void {
    setStoreOffer(null);
}

/** Record what the Store said when asked to install (it had nothing to install). */
export function recordStoreNone(): void {
    setStoreOffer({ state: "none", version: "", message: "", checkedMs: Date.now() });
}

/** The Store's last answer for a Store install, or null when none was asked yet (or this isn't a Store install). */
export function storeAnswer(s: AppUpdateStatus | null = appUpdate()): StoreOffer | null {
    return viaStore(s) ? storeOffer() : null;
}

/**
 * The release srv found on GitHub that this Store install's Store check does not list yet, or null. The Store
 * stays the only thing that installs; this only lets the panel say why it shows nothing to install.
 */
export function storePublishedNotListed(s: AppUpdateStatus | null = appUpdate()): string | null {
    if (!viaStore(s) || storeOffer()?.state !== "none") return null;
    return s?.state === "available" && s.latest && isNewer(s.latest, s.current) ? s.latest : null;
}

/**
 * What the version panel says where a release would be, for a Store install that has none to offer, in the
 * Store's own terms; null when it has one (the panel then offers it).
 */
export function storeStandingLine(offer: StoreOffer | null, checked: string | null, published: string | null = null): string | null {
    if (!offer) return "Not checked with the Microsoft Store yet";
    switch (offer.state) {
        case "available":
            return null;
        case "asking":
            return "Asking the Microsoft Store for updates…";
        case "none":
            if (published) {
                return `v${published} is published, but the Microsoft Store hasn't listed it for this PC yet${checked ? ` (checked ${checked})` : ""}. This can take a while after the Store certifies a release.`;
            }
            return `The Microsoft Store has no newer version${checked ? ` (checked ${checked})` : ""}`;
        case "no_answer":
            return "The Microsoft Store didn't answer; try again later.";
        default:
            return `Couldn't ask the Microsoft Store${offer.message ? ` (${offer.message})` : ""}.`;
    }
}

/** What the Store's last install left (host: store_restart): the app came back `updated`, or `stayed` on the version it was closed on. */
export type StoreCameBack = { state: "none" | "updated" | "stayed"; from: string; target: string; reopen_failed?: boolean };
const [storeStayed, setStoreStayed] = createSignal<StoreCameBack | null>(null);
export { storeStayed, setStoreStayed };

/**
 * What the Store reports while it downloads and installs (the host's `store-update-progress` event). `percent` is
 * null when the Store gave no numbers.
 */
export type StoreProgress = { phase: "pending" | "downloading" | "installing" | "asking"; percent: number | null; doneBytes: number; totalBytes: number };
const [storeProgress, setProgress] = createSignal<StoreProgress | null>(null);
export { storeProgress };

const PHASES = new Set(["pending", "downloading", "installing", "asking"]);

/**
 * Whether the install call is open: the person pressed Restart now, confirmed, and the host is asking the Store. It
 * lives here, not in the Maintenance section, so closing the panel and opening it again (or switching to another
 * window and back) still finds the row saying what is going on.
 */
const [storeRestarting, setStoreRestarting] = createSignal(false);
export { storeRestarting, setStoreRestarting };

/** The Store's own window belongs to ours: coming back to ours while it asks brings that window forward again. */
let raiseOnFocus: (() => void) | null = null;
function keepAskingInFront(asking: boolean): void {
    if (asking === (raiseOnFocus !== null) || typeof window === "undefined") return;
    if (asking) {
        raiseOnFocus = () => void getApi()?.approvals?.storeUpdateRaise?.().catch(() => undefined);
        window.addEventListener("focus", raiseOnFocus);
    } else if (raiseOnFocus) {
        window.removeEventListener("focus", raiseOnFocus);
        raiseOnFocus = null;
    }
}

export function setStoreProgress(next: StoreProgress | null): void {
    setProgress(next);
    keepAskingInFront(next?.phase === "asking");
}

/** Take a report from the host's event; anything malformed is ignored. */
export function acceptStoreProgress(next: unknown): void {
    const p = next as Partial<StoreProgress> | null | undefined;
    if (typeof p?.phase !== "string" || !PHASES.has(p.phase)) return;
    // While the Store asks, its own earlier states (pending, a finished download) don't take the question off the row.
    if (storeProgress()?.phase === "asking" && (p.phase === "pending" || p.phase === "downloading")) return;
    const percent = typeof p.percent === "number" && Number.isFinite(p.percent) ? Math.max(0, Math.min(100, Math.round(p.percent))) : null;
    setStoreProgress({ phase: p.phase as StoreProgress["phase"], percent, doneBytes: Number(p.doneBytes) || 0, totalBytes: Number(p.totalBytes) || 0 });
}

/** The row while the install call is open and the Store asks the person to confirm, in its own window. */
export const STORE_ASKING_WINDOW_LINE = "The Microsoft Store is asking you to confirm: look for its window.";

/** The row while the Store installs without asking. */
export const STORE_INSTALLING_LINE = "Installing: AgentMux closes and opens again.";

const megabytes = (bytes: number): number => Math.round(bytes / (1024 * 1024));

/**
 * The line for the Store's progress, in place of the fixed "Asking the Microsoft Store…": "Downloading 0.60.25: 42%
 * (88 of 210 MB)", then "Installing…". null when there is no report yet or the Store gave no numbers (the caller
 * keeps its plain line); `version` is empty when it isn't known.
 */
export function storeProgressLine(p: StoreProgress | null, version: string): string | null {
    if (!p) return null;
    switch (p.phase) {
        case "pending":
            return null;
        case "downloading": {
            const what = version ? `Downloading ${version}` : "Downloading the update";
            if (p.percent === null) return null;
            const size = p.totalBytes > 0 ? ` (${megabytes(p.doneBytes)} of ${megabytes(p.totalBytes)} MB)` : "";
            return `${what}: ${p.percent}%${size}`;
        }
        case "installing":
            return STORE_INSTALLING_LINE;
        case "asking":
            return STORE_ASKING_WINDOW_LINE;
        default:
            return null;
    }
}

/**
 * The version a Store update will be, for the row's wording: only what the Store itself named (a version newer than
 * this build), else "". GitHub's newest release is not used: the Store installs the version it has, which can be an
 * older one, and no public Windows call names a staged package's version (the Store names its listed package only
 * as the installed one). The row then says "the new version", and the next start reports the real one.
 */
export function storeTargetVersion(offer: StoreOffer | null = storeOffer()): string {
    return offer?.version ?? "";
}

/** The Store has the update downloaded here, ready for Restart now; `version` is "" when no one knows it. */
const [storeDownloaded, setStoreDownloaded] = createSignal<{ version: string } | null>(null);
export { storeDownloaded, setStoreDownloaded };

/**
 * Said once the update is downloaded. Nothing is installed yet, and closing AgentMux by hand does not install it
 * (Windows swaps the package only when the Store is asked to, which Restart now does).
 */
export const storeDownloadedLine = (version: string): string =>
    `Downloaded${version ? ` ${version}` : ""}. Restart now installs it: AgentMux closes and opens again. Closing AgentMux yourself does not install it.`;

/** "Vmer", "Vmer and Deke", "Vmer, Deke and 2 more": the agents a restart interrupts, as the close warning names them. */
function namesLine(names: readonly string[]): string {
    const MAX = 3;
    if (names.length <= 1) return names.join("");
    if (names.length <= MAX) return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
    return `${names.slice(0, MAX).join(", ")} and ${names.length - MAX} more`;
}

/**
 * Said before Restart now closes anything: it is a restart of everything, and it names the agents in the middle of
 * a turn (`working`, from srv's quit question) that it interrupts. This confirm is the only question AgentMux asks;
 * the Store may then ask once more in its own window.
 */
export const storeRestartAsk = (working: readonly string[] = [], agents = 0): string => {
    const stops = agents > 0 ? `stops ${agents === 1 ? "its 1 agent" : `its ${agents} agents`}` : "stops every agent in it";
    const who = working.length > 0 ? ` ${namesLine(working)} ${working.length === 1 ? "is" : "are"} in the middle of a turn and will be interrupted.` : "";
    return `Restart now closes AgentMux, ${stops}, installs the update and opens AgentMux again.${who}`;
};

/** Said when the install fell back to the Store page: `why` is the host's reason, when it gave one. */
export const storeFallbackLine = (why?: string): string => `${why ? `${why} ` : ""}Opened the Microsoft Store page to finish the update.`;

/** Said before the Store is asked to download: Restart now installs it afterwards. */
export const storeClosesLine = (version: string): string =>
    `The Store downloads ${version ? `v${version}` : "the update"}. Restart now then installs it and opens AgentMux again.`;

/** Added when the Scheduled Task that opens AgentMux again after the update could not be registered. */
export const storeReopenFailedNote = "AgentMux could not arrange to open itself after the update, so it was opened by hand.";

/** Said when the app is back on the version the Store closed it on: the install did not happen. */
export const storeStayedLine = (s: StoreCameBack): string =>
    `The Store closed AgentMux to install ${s.target ? `v${s.target}` : "an update"}, but this is still v${s.from}. Try Update in the Store again, or install it from the Microsoft Store app.${s.reopen_failed ? ` ${storeReopenFailedNote}` : ""}`;

/** At start: say how the Store's last install ended, when one was under way. */
export async function loadStoreCameBack(): Promise<void> {
    const r = await getApi()?.approvals?.storeUpdateCameBack?.().catch(() => undefined);
    if (!r || r.state === "none") return;
    const note = { id: "store-update-came-back", timestamp: new Date().toISOString() };
    if (r.state === "updated") {
        pushNotification({ ...note, icon: "fa-circle-check", title: "AgentMux updated", message: `Updated to v${r.target} through the Microsoft Store.${r.reopen_failed ? ` ${storeReopenFailedNote}` : ""}`, type: "update", expiration: Date.now() + (r.reopen_failed ? 30000 : 10000) });
        return;
    }
    setStoreStayed(r);
    pushNotification({ ...note, icon: "fa-triangle-exclamation", title: "The update did not install", message: storeStayedLine(r), type: "warning", persistent: true });
}

/** A window asks the Store this long after it starts, then every `STORE_ASK_EVERY_MS`: srv's own cadence for GitHub (`update/auto.rs`). */
const STORE_FIRST_ASK_MS = 60 * 1000;
const STORE_ASK_EVERY_MS = 6 * 60 * 60 * 1000;

let storeWatchers = 0;
let storeTimer: ReturnType<typeof setInterval> | undefined;

/** The background asks follow the same switch as srv's: `update:auto` off asks nothing by itself. */
const autoChecks = () => getSettingsKeyAtom("update:auto")() !== false;

/**
 * In a component: a Store install asks the Store for updates a minute after the window starts and every six
 * hours after, as srv does for GitHub. `now`: ask as soon as the Store install is known (the version panel is
 * open) when nothing was answered yet, or the Store didn't answer.
 */
export function useStoreOffer(now = false): void {
    let askedNow = false;
    createEffect(() => {
        if (!viaStore()) return;
        const offer = storeOffer();
        if (now) {
            const retry = !offer || offer.state === "no_answer" || offer.state === "unknown";
            if (retry && !askedNow) {
                askedNow = true;
                void askStore();
            }
            return;
        }
        if (offer) return;
        const first = setTimeout(() => {
            if (autoChecks() && !storeOffer()) void askStore();
        }, STORE_FIRST_ASK_MS);
        onCleanup(() => clearTimeout(first));
    });
    if (storeWatchers++ === 0) {
        storeTimer = setInterval(() => {
            if (viaStore() && autoChecks()) void askStore(true);
        }, STORE_ASK_EVERY_MS);
    }
    onCleanup(() => {
        if (--storeWatchers === 0) clearInterval(storeTimer);
    });
}

/**
 * The release newer than this build, or null: what srv says GitHub has; for a Store install, what the Store
 * says it has (its own version, with that version's release notes as the link).
 */
export function availableRelease(s: AppUpdateStatus | null = appUpdate()): { version: string; url: string } | null {
    if (viaStore(s)) {
        const offer = storeOffer();
        return offer?.state === "available" ? { version: offer.version, url: releaseNotesUrl(offer.version) } : null;
    }
    if (s?.state !== "available" || !s.latest || !s.url) return null;
    return { version: s.latest, url: s.url };
}

/**
 * Whether a download of the offered release, or a restart into it, is under
 * way: a new check then can't replace the release on offer, so the panel
 * doesn't offer one.
 */
export function upgradeUnderWay(s: AppUpdateStatus | null = appUpdate()): boolean {
    const state = releaseDownload(s)?.state;
    return state === "downloading" || state === "unpacking" || state === "waiting" || state === "restarting";
}

/** When the last check answered, for "Checked 14:05" (a date when not today). */
export function checkedAt(ms: number, now: Date = new Date()): string {
    return formatTimeOrDate(ms, now);
}

/** srv's download of the newer release, or null before one was asked for. */
export function releaseDownload(s: AppUpdateStatus | null = appUpdate()): DownloadStatus | null {
    const d = s?.download;
    return d && s?.latest && d.version === s.latest ? d : null;
}

/** What's new: the release's own page on GitHub, or the list of releases when the version isn't known. */
export function releaseNotesUrl(version: string | null | undefined): string {
    return version
        ? `https://github.com/agentmuxai/agentmux/releases/tag/v${version}`
        : "https://github.com/agentmuxai/agentmux/releases";
}

/**
 * The newer release once it is downloaded, verified and staged (srv does this by
 * itself, `update/auto.rs`): ready to restart into, or a restart under way.
 * Null while it is still downloading, failed, or has no download.
 */
export function stagedRelease(
    s: AppUpdateStatus | null = appUpdate(),
): { version: string; state: "ready" | "waiting" | "restarting"; waitingFor: number; waitingOn: WaitingOn[] } | null {
    const d = availableRelease(s) ? releaseDownload(s) : null;
    if (!d || (d.state !== "ready" && d.state !== "waiting" && d.state !== "restarting")) return null;
    return { version: d.version, state: d.state, waitingFor: d.waiting_for ?? 0, waitingOn: d.waiting_on ?? [] };
}

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * How long the staged release has gone unclicked, for the chip's colour: the
 * "warm" step after 2 days of the release being out, the "hot" step after 4
 * (Chrome's relaunch icon steps at 2, 4 and 7 days). Nothing nags earlier.
 */
export function stagedUrgency(s: AppUpdateStatus | null = appUpdate(), nowMs: number = Date.now()): "calm" | "warm" | "hot" {
    const published = s?.published_at ? Date.parse(s.published_at) : NaN;
    if (!Number.isFinite(published)) return "calm";
    const age = nowMs - published;
    return age >= 4 * DAY_MS ? "hot" : age >= 2 * DAY_MS ? "warm" : "calm";
}

/**
 * A click in the version panel, through the host: srv takes these only from
 * it, so an agent holding the instance key can't make them.
 */
async function hostAction(action: "download" | "restart" | "cancel", now?: boolean): Promise<void> {
    const relay = getApi()?.approvals?.appUpdate;
    if (typeof relay !== "function") throw new Error("This window can't do that; use the release page.");
    acceptAppUpdate(await relay(action, now));
}

/** Ask srv to download and stage the newer release; progress arrives as events. */
export async function startAppUpdateDownload(): Promise<void> {
    await hostAction("download");
}

/**
 * Restart into the staged release: once agents finish their turn, or `now`.
 * The launcher then asks the user in a dialog of its own.
 */
export async function upgradeAndRestart(now: boolean): Promise<void> {
    await hostAction("restart", now);
}

/** Stop waiting to restart. */
export async function cancelUpgrade(): Promise<void> {
    await hostAction("cancel");
}

/** Open the version panel, where the Maintenance section shows the release. */
export function openVersionPanel(): void {
    window.dispatchEvent(new CustomEvent("agentmux:open-version-panel"));
}
