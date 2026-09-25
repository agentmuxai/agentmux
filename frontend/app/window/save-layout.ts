// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ☰ → Layouts → "Save layout…" (docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md
// §6.1): the host's Save dialog picks the file, the srv's `layout.save`
// writes this window into it. The layout's name is the file name the user
// chose, so there is no second prompt.

import { getApi, pushNotification } from "@/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { windowId } from "@/app/store/window-identity";

const LAYOUT_EXTENSION = /\.agentmux-layout\.json$/i;

/** `C:\x\Review setup.agentmux-layout.json` → `Review setup`. */
export function layoutNameFromPath(path: string): string {
    const base = path.split(/[\\/]/).pop() ?? "";
    return base.replace(LAYOUT_EXTENSION, "").trim() || "Layout";
}

function notify(type: "info" | "warning" | "error", title: string, message: string): void {
    pushNotification({
        icon: type === "error" ? "fa-triangle-exclamation" : "fa-floppy-disk",
        title,
        message,
        timestamp: new Date().toISOString(),
        type,
        expiration: Date.now() + (type === "info" ? 6000 : 12000),
    });
}

/** Ask where to save, then save the current window. A cancelled dialog does
 *  nothing; a failure or a warning is shown, never swallowed. */
export async function saveCurrentLayout(): Promise<void> {
    const path = await getApi()?.showSaveLayoutDialog?.("Layout");
    if (!path) {
        return;
    }
    try {
        const result = await RpcApi.SaveLayoutCommand(TabRpcClient, {
            window_id: windowId(),
            name: layoutNameFromPath(path),
            path,
        });
        if (result.warnings.length > 0) {
            notify("warning", "Layout saved — check before sharing", [result.path, ...result.warnings].join("\n"));
        } else {
            notify("info", "Layout saved", result.path);
        }
    } catch (err) {
        notify("error", "Couldn't save the layout", err instanceof Error ? err.message : String(err));
    }
}
