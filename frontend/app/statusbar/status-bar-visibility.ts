// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the status bar is drawn (`workspace.tsx`).
 *
 * `window:showstatusbar` (on unless set to `false`) hides it. A hidden bar
 * comes back while the backend is down, because the offline dot and "Open
 * Maintenance" live there and are the way to see and recover from that.
 */
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { backendStatusAtom, getSettingsKeyAtom } from "@/store/global";

export const SHOW_STATUS_BAR_KEY = "window:showstatusbar";

/** The pure rule, for the tests. */
export function statusBarShown(setting: boolean | undefined, backendStatus: string): boolean {
    return setting !== false || backendStatus === "crashed";
}

export function statusBarVisible(): boolean {
    return statusBarShown(getSettingsKeyAtom(SHOW_STATUS_BAR_KEY)(), backendStatusAtom());
}

/** Palette command: flip the setting for every window. */
export function toggleStatusBar(): void {
    const on = getSettingsKeyAtom(SHOW_STATUS_BAR_KEY)() !== false;
    void RpcApi.SetConfigCommand(TabRpcClient, { "window:showstatusbar": !on });
}
