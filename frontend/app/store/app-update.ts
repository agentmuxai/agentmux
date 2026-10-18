// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a newer AgentMux release exists, as srv learns it from GitHub
 * Releases (crates/srv/src/backend/update_check.rs), only when the user clicks
 * Check for update or an agent asks: `app.update-status` when a window opens,
 * kept current by the `app:updatestatus` event. The status-bar chip and the
 * version panel's Maintenance section read it.
 * docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §3.
 */

import { createSignal } from "solid-js";

import { WpsEvent } from "@/app/store/mps-events";
import { getApi } from "@/app/store/app-api";
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
}

/** Ask srv for the status now (a window opened after the last event). */
export async function loadAppUpdate(): Promise<void> {
    try {
        acceptAppUpdate(await RpcApi.AppUpdateStatusCommand(TabRpcClient));
    } catch {
        // An older srv without the command: no update news.
    }
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
    const at = new Date(ms);
    return at.toDateString() === now.toDateString()
        ? at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : at.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

/** srv's download of the newer release, or null before one was asked for. */
export function releaseDownload(s: AppUpdateStatus | null = appUpdate()): DownloadStatus | null {
    const d = s?.download;
    return d && s?.latest && d.version === s.latest ? d : null;
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
