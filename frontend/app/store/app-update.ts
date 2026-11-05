// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a newer AgentMux release exists, as srv learns it from GitHub
 * Releases (crates/srv/src/backend/update/check.rs), which srv asks in the
 * background (`update/auto.rs`) and when the user clicks Check for update or an
 * agent asks: `app.update-status` when a window opens,
 * kept current by the `app:updatestatus` event. The status-bar chip and the
 * version panel's Maintenance section read it.
 * docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §3.
 */

import { pushNotification } from "@/app/store/flash-notifications";
import { createEffect, createSignal, onCleanup } from "solid-js";

import { WpsEvent } from "@/app/store/mps-events";
import { getApi } from "@/util/app-api";
import { formatTimeOrDate } from "@/util/format-time";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AppUpdateStatus } from "@/types/rpc/AppUpdateStatus";
import type { DownloadStatus } from "@/types/rpc/DownloadStatus";

export type { AppUpdateStatus } from "@/types/rpc/AppUpdateStatus";

/** The `app:updatestatus` WebSocket event. */
export const APP_UPDATE_EVENT = WpsEvent.AppUpdateStatus;

const STATES = new Set(["up-to-date", "available", "local-build", "unknown"]);

const [appUpdate, setAppUpdate] = createSignal<AppUpdateStatus | null>(null);

/** What srv last said, or null before it has. */
export { appUpdate };

/** Take a status from `app.update-status` or the event; ignores anything malformed. */
export function acceptAppUpdate(next: unknown): void {
    const s = next as Partial<AppUpdateStatus> | null | undefined;
    if (typeof s?.state !== "string" || !STATES.has(s.state) || typeof s.current !== "string") return;
    setAppUpdate(s as AppUpdateStatus);
}

/**
 * Check for update: srv asks GitHub now (at most every 30 seconds) and every
 * window gets the answer. Throws when GitHub can't be reached.
 */
export async function checkForUpdate(): Promise<void> {
    acceptAppUpdate(await RpcApi.AppUpdateCheckCommand(TabRpcClient));
    // A Store install is updated by the Store, which lags GitHub: ask it again too.
    const release = availableRelease();
    if (viaStore() && release) await askStore(release.version);
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

/** What the Store itself said about `version`: `asking` until it answers; `no_answer` when it never did; `unknown` carries why it couldn't be asked. */
export type StoreOffer = { version: string; state: "asking" | "available" | "none" | "no_answer" | "unknown"; message: string };

const [storeOffer, setStoreOffer] = createSignal<StoreOffer | null>(null);

/** Ask the Store (through the host) whether it lists an update, instead of guessing from GitHub. `quiet`: keep showing the last answer until the new one comes. */
export async function askStore(version: string, quiet = false): Promise<void> {
    if (!quiet) setStoreOffer({ version, state: "asking", message: "" });
    let next: StoreOffer;
    try {
        const r = await getApi()?.approvals?.storeUpdateCheck?.();
        next = r ? { version, state: r.state, message: r.message } : { version, state: "unknown", message: "this window can't ask the Store" };
    } catch (e: any) {
        next = { version, state: "unknown", message: String(e?.message ?? e) };
    }
    setStoreOffer(next);
}

/** Forget the Store's answer: the next look at a release on offer asks again. */
export function forgetStoreOffer(): void {
    setStoreOffer(null);
}

/** Record what the Store said when asked to install (it had nothing to install). */
export function recordStoreNone(version: string): void {
    setStoreOffer({ version, state: "none", message: "" });
}

/** The Store's answer about the release on offer, or null when none was asked about it. */
export function storeAnswer(s: AppUpdateStatus | null = appUpdate()): StoreOffer | null {
    const release = availableRelease(s);
    const offer = storeOffer();
    return viaStore(s) && release && offer?.version === release.version ? offer : null;
}

/**
 * Whether an update can be had now: always, for an install AgentMux updates
 * itself; for a Store install, only when the Store lists it.
 */
export function updateOffered(s: AppUpdateStatus | null = appUpdate()): boolean {
    return !viaStore(s) || storeAnswer(s)?.state === "available";
}

/** What the panel says in place of "v1.2 is out" while the Store has not listed it; null once it has. */
export function storeOfferLine(version: string | null | undefined, offer: StoreOffer | null): string | null {
    if (offer?.state === "available") return null;
    const released = `v${version} is released.`;
    if (!offer || offer.state === "asking") return `${released} Asking the Microsoft Store whether it has it…`;
    if (offer.state === "none") return `${released} The Microsoft Store is still reviewing it.`;
    if (offer.state === "no_answer") return `${released} The Microsoft Store didn't answer; try again later.`;
    return `${released} Couldn't ask the Microsoft Store whether it has it${offer.message ? ` (${offer.message})` : ""}.`;
}

/** What the Store's last install left (host: store_restart): the app came back `updated`, or `stayed` on the version it was closed on. */
export type StoreCameBack = { state: "none" | "updated" | "stayed"; from: string; target: string };
const [storeStayed, setStoreStayed] = createSignal<StoreCameBack | null>(null);
export { storeStayed, setStoreStayed };

/** Said when the install fell back to the Store page: `why` is the host's reason, when it gave one. */
export const storeFallbackLine = (why?: string): string => `${why ? `${why} ` : ""}Opened the Microsoft Store page to finish the update.`;

/** Said before the Store is asked to install: it closes the app to swap the package. */
export const storeClosesLine = (version: string): string => `The Store closes AgentMux while it installs v${version}, then opens it again.`;

/** Said when the app is back on the version the Store closed it on: the install did not happen. */
export const storeStayedLine = (s: StoreCameBack): string =>
    `The Store closed AgentMux to install v${s.target}, but this is still v${s.from}. Try Update in the Store again, or install it from the Microsoft Store app.`;

/** At start: say how the Store's last install ended, when one was under way. */
export async function loadStoreCameBack(): Promise<void> {
    const r = await getApi()?.approvals?.storeUpdateCameBack?.().catch(() => undefined);
    if (!r || r.state === "none") return;
    const note = { id: "store-update-came-back", timestamp: new Date().toISOString() };
    if (r.state === "updated") {
        pushNotification({ ...note, icon: "fa-circle-check", title: "AgentMux updated", message: `Updated to v${r.target} through the Microsoft Store.`, type: "update", expiration: Date.now() + 10000 });
        return;
    }
    setStoreStayed(r);
    pushNotification({ ...note, icon: "fa-triangle-exclamation", title: "The update did not install", message: storeStayedLine(r), type: "warning", persistent: true });
}

/** The Store certifies a release hours after GitHub publishes it: how often to ask again while it hasn't listed it. */
const STORE_REASK_MS = 30 * 60 * 1000;

/** In a component: ask the Store about each release on offer to a Store install, and again every half hour until it lists it. */
export function useStoreOffer(): void {
    createEffect(() => {
        const release = availableRelease();
        if (viaStore() && release && storeOffer()?.version !== release.version) void askStore(release.version);
    });
    const timer = setInterval(() => {
        const release = availableRelease();
        const state = storeAnswer()?.state;
        if (viaStore() && release && (state === "none" || state === "unknown")) void askStore(release.version, true);
    }, STORE_REASK_MS);
    onCleanup(() => clearInterval(timer));
}

/** The release srv says is newer than this build, or null. */
export function availableRelease(s: AppUpdateStatus | null = appUpdate()): { version: string; url: string } | null {
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
): { version: string; state: "ready" | "waiting" | "restarting"; waitingFor: number } | null {
    const d = availableRelease(s) ? releaseDownload(s) : null;
    if (!d || (d.state !== "ready" && d.state !== "waiting" && d.state !== "restarting")) return null;
    return { version: d.version, state: d.state, waitingFor: d.waiting_for ?? 0 };
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
