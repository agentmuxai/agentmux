// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a link to something inside AgentMux goes (link-target.ts says which kind it is): a local file in the
 * Editor (a tab of the one on screen, else a new pane), a folder in Files, a task on the Kanban. Loaded on
 * the first such click, so a page with only web links never pulls in the Editor and the Kanban.
 */

import { atoms } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openInEditorPaneOnScreen } from "@/app/view/editor/editor-open";
import { openKanbanTask } from "@/app/view/kanban/kanban-held";

/** Whether `path` is a folder: it says so (a trailing slash), or listing one entry of it works. */
async function isFolder(path: string): Promise<boolean> {
    if (/[\/]$/.test(path)) return true;
    try {
        await RpcApi.FsListCommand(TabRpcClient, { path, limit: 1 }, {});
        return true;
    } catch {
        return false;
    }
}

/** A file in the Editor, a folder in Files. Whatever isn't there is left for the Editor to say so. */
export async function openPathLink(path: string): Promise<void> {
    if (await isFolder(path)) {
        await RpcApi.PaneOpenCommand(TabRpcClient, { view: "files", file: path, tab_id: atoms.activeTabId() }, {});
        return;
    }
    if (await openInEditorPaneOnScreen(path)) return;
    await RpcApi.PaneOpenCommand(TabRpcClient, { view: "editor", file: path, tab_id: atoms.activeTabId() }, {});
}

/** The Kanban (the one open, else a new one) with task #`num`'s card open. */
export function openTaskLink(num: number): Promise<void> {
    return openKanbanTask(String(num));
}
