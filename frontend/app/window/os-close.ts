// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A close of this window that came from the OS (Alt+F4, the taskbar's Close,
// the window manager) is held back by the host and sent here, so it does what
// the window's own close button does: warn first when it stops working agents
// (window-close.ts). Host side: crates/cef/src/commands/window/close_ask.rs.

import { getApi } from "@/store/global";

/** What the host sends for an OS close it held back. */
export const OS_CLOSE_EVENT = "window-close-requested";

/** Answer one held-back OS close: say it arrived, then close as the close
 *  button does. If that can't run, close anyway: the user asked to. */
export async function onOsCloseRequest(seq: number): Promise<void> {
    const api = getApi();
    // Said first: the host closes a window whose page doesn't answer.
    api.closeRequestSeen(seq).catch(() => {});
    try {
        const { requestCloseWindow } = await import("./window-close");
        await requestCloseWindow();
    } catch (e) {
        console.warn("[os-close] the close check failed; closing the window", e);
        await api.closeWindow().catch(console.error);
    }
}

/** Listen for the host's held-back OS closes. Once per page. */
export function installOsCloseRequests(): void {
    void getApi().listen<{ seq?: number }>(OS_CLOSE_EVENT, (payload) => {
        void onOsCloseRequest(payload?.seq ?? 0);
    });
}
