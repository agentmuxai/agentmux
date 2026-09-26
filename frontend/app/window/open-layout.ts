// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ☰ → Layouts → "Open layout…" (docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md
// §3.5): the host's Open dialog picks the file, `layout.preview` describes
// it, the user confirms in the preview, and `layout.open` builds its tabs —
// in a new workspace that a new window then opens onto, or added to this
// window.

import { getApi, pushNotification } from "@/store/global";
import { openModal } from "@/app/store/modalmodel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WorkspaceService } from "@/app/store/services";
import { windowId } from "@/app/store/window-identity";
import { LayoutPreviewModal, type LayoutOpenChoice } from "./layout-preview-modal";

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

function errorText(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
}

/** Open the layout at `path`. Exported for the preview's "Open" button and
 *  for tests. */
export async function applyLayout(path: string, choice: LayoutOpenChoice): Promise<void> {
    let result;
    try {
        result = await RpcApi.OpenLayoutCommand(TabRpcClient, {
            path,
            window_id: windowId(),
            run_commands: choice.runCommands,
            new_window: choice.newWindow,
        });
    } catch (err) {
        notify("error", "Couldn't open the layout", errorText(err));
        return;
    }
    const count = `${result.tab_ids.length} ${result.tab_ids.length === 1 ? "tab" : "tabs"}`;
    if (choice.newWindow) {
        try {
            await getApi().openWorkspaceInNewWindow(result.workspace_id);
        } catch (err) {
            // Nothing shows the new workspace, so don't keep it (the same
            // rollback a tear-off whose window never opened does).
            await WorkspaceService.DeleteWorkspace(result.workspace_id).catch((e: unknown) =>
                console.error("[open-layout] couldn't remove the layout's workspace:", e),
            );
            notify("error", "Couldn't open the layout's window", errorText(err));
            return;
        }
    }
    const done = choice.newWindow ? "Layout opened in a new window" : "Layout opened";
    const summary = choice.newWindow ? count : `${count} added`;
    if (result.notes.length > 0) {
        notify("warning", `${done} — ${summary}`, result.notes.join("\n"));
    } else {
        notify("info", done, summary);
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
        notify("error", "Couldn't read the layout", errorText(err));
        return;
    }
    openModal(LayoutPreviewModal, { preview, onOpen: (choice: LayoutOpenChoice) => applyLayout(path, choice) });
}
