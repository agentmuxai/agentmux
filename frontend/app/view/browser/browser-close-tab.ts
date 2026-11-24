// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { pushNotification } from "@/app/store/global";
import { closeOwnTab } from "@/app/util/close-own-tab";
import type { BrowserViewModel } from "./browser-model";

/** Close `model`'s tab of its pane (Ctrl/Cmd+W). The pane's last tab stays: the pane's own × closes the pane. */
export async function closeBrowserTab(model: BrowserViewModel): Promise<void> {
    const note = (message: string, type: "info" | "error") =>
        pushNotification({
            icon: type === "error" ? "fa-triangle-exclamation" : "fa-circle-info",
            title: type === "error" ? "Couldn't close the tab" : "This is the pane's only tab",
            message,
            timestamp: new Date().toISOString(),
            type,
            expiration: Date.now() + 5000,
        });
    try {
        if (!(await closeOwnTab(model.blockId))) note("Use the pane's × to close it.", "info");
    } catch (e) {
        note(e instanceof Error ? e.message : String(e), "error");
    }
}
