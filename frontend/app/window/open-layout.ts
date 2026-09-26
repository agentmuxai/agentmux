// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ☰ → Layouts → "Open layout…" (docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md
// §3.5): the host's Open dialog picks the file, `layout.preview` describes
// it, the user confirms in the preview, and `layout.open` adds its tabs to
// this window.

import { getApi, pushNotification } from "@/store/global";
import { openModal } from "@/app/store/modalmodel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { windowId } from "@/app/store/window-identity";
import { LayoutPreviewModal } from "./layout-preview-modal";

function notify(type: "info" | "warning" | "error", title: string, message: string): void {
    pushNotification({
        icon: type === "error" ? "fa-triangle-exclamation" : "fa-table-columns",
        title,
        message,
        timestamp: new Date().toISOString(),
        type,
        expiration: Date.now() + (type === "info" ? 6000 : 15000),
    });
}

/** Open the layout at `path` into this window. Exported for the preview's
 *  "Open" button and for tests. */
export async function applyLayout(path: string, runCommands: boolean): Promise<void> {
    try {
        const result = await RpcApi.OpenLayoutCommand(TabRpcClient, {
            path,
            window_id: windowId(),
            run_commands: runCommands,
        });
        const tabs = `${result.tab_ids.length} ${result.tab_ids.length === 1 ? "tab" : "tabs"} added`;
        if (result.notes.length > 0) {
            notify("warning", `Layout opened — ${tabs}`, result.notes.join("\n"));
        } else {
            notify("info", "Layout opened", tabs);
        }
    } catch (err) {
        notify("error", "Couldn't open the layout", err instanceof Error ? err.message : String(err));
    }
}

/** Ask for a file, preview it, and open it on confirmation. A cancelled
 *  dialog does nothing; an unreadable file is reported, not swallowed. */
export async function openLayoutFromFile(): Promise<void> {
    const path = await getApi()?.showOpenLayoutDialog?.();
    if (!path) {
        return;
    }
    let preview;
    try {
        preview = await RpcApi.PreviewLayoutCommand(TabRpcClient, { path });
    } catch (err) {
        notify("error", "Couldn't read the layout", err instanceof Error ? err.message : String(err));
        return;
    }
    openModal(LayoutPreviewModal, { preview, onOpen: (runCommands: boolean) => applyLayout(path, runCommands) });
}
