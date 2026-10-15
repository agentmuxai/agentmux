// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a newer AgentMux release exists, as srv learns it from GitHub
 * Releases (crates/srv/src/backend/update_check.rs): `app.update-status`
 * when a window opens, kept current by the `app:updatestatus` event. The
 * status-bar chip and the version panel's Maintenance section read it.
 * docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §3.
 */

import { createSignal } from "solid-js";

import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AppUpdateStatus } from "@/types/rpc/AppUpdateStatus";

export type { AppUpdateStatus } from "@/types/rpc/AppUpdateStatus";

/** The `app:updatestatus` WebSocket event. */
export const APP_UPDATE_EVENT = WpsEvent.AppUpdateStatus;

const STATES = new Set(["up-to-date", "available", "local-build", "off", "unknown"]);

const [appUpdate, setAppUpdate] = createSignal<AppUpdateStatus | null>(null);

/** What srv last said, or null before it has. */
export { appUpdate };

/** Take a status from `app.update-status` or the event; ignores anything malformed. */
export function acceptAppUpdate(next: unknown): void {
    const s = next as Partial<AppUpdateStatus> | null | undefined;
    if (typeof s?.state !== "string" || !STATES.has(s.state) || typeof s.current !== "string") return;
    setAppUpdate(s as AppUpdateStatus);
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

/** Open the version panel, where the Maintenance section shows the release. */
export function openVersionPanel(): void {
    window.dispatchEvent(new CustomEvent("agentmux:open-version-panel"));
}
