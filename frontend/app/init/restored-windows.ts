// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { WindowService } from "@/app/store/services";
import { getApi } from "@/store/global";

/**
 * Open the windows a restore rebuilt beside this one: the rest of the last
 * session's windows, each in a window of its own where it was. Best-effort:
 * one that fails to open leaves the others, and this window, as they are.
 */
export async function reopenRestoredWindows(): Promise<void> {
    try {
        const restored = await WindowService.TakeRestoredWindows();
        for (const w of restored ?? []) {
            await getApi().openRestoredWindow(w);
        }
    } catch (e) {
        console.warn("[session-restore] couldn't reopen the last session's other windows", e);
    }
}
