// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a link to something inside AgentMux goes (link-target.ts says which kind it is): a local file in the
 * Editor (a tab of the one on screen, else a new pane), a folder in Files, a task on the Kanban. Loaded on
 * the first such click, so a page with only web links never pulls in the Editor and the Kanban.
 */

import { atoms, pushNotification } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openInEditorPaneOnScreen } from "@/app/util/editor-open";
import { openKanbanTask } from "@/app/util/open-kanban-task";

/** Whether `path` is a folder: it says so (a trailing slash), or listing one entry of it works. A listing
 *  that fails says why in its `error` (it does not throw): a file ("not a directory") and something that is
 *  not there are not folders; a folder AgentMux may not read is, and Files says why it can't show it. */
async function isFolder(path: string): Promise<boolean> {
    if (/[\\/]$/.test(path)) return true;
    try {
        const listing = await RpcApi.FsListCommand(TabRpcClient, { path, limit: 1 }, {});
        const kind = listing?.error?.kind;
        return !kind || kind === "permission_denied" || kind === "os_blocked";
    } catch {
        return false;
    }
}

/** Whether `path` is in a folder a container agent can write. The agent can change such a folder while a
 *  window reads it, so srv opens what is in it: a file as a read-only copy, a folder in a Files pane
 *  confined to it. */
async function inAgentFolder(path: string): Promise<boolean> {
    try {
        return (await RpcApi.FsAgentFolderCommand(TabRpcClient, { path }, {}))?.agent_folder === true;
    } catch {
        return false;
    }
}

/** A file in the Editor, a folder in Files. Whatever isn't there is left for the Editor to say so. With
 *  `floating` (a Ctrl+click), in a window of its own rather than a tab of the Editor on screen. A path in a
 *  container agent's folder is never given to the Editor on screen, which would read the path itself: srv
 *  opens it (`from_link`), and when it refuses, the person is told why. */
export async function openPathLink(path: string, opts: { floating?: boolean } = {}): Promise<void> {
    const open = { file: path, tab_id: atoms.activeTabId(), from_link: true, ...(opts.floating ? { floating: true } : {}) };
    const agents = await inAgentFolder(path);
    try {
        if (await isFolder(path)) {
            await RpcApi.PaneOpenCommand(TabRpcClient, { view: "files", ...open }, {});
            return;
        }
        if (!agents && !opts.floating && (await openInEditorPaneOnScreen(path))) return;
        await RpcApi.PaneOpenCommand(TabRpcClient, { view: "editor", ...open }, {});
    } catch (err) {
        if (agents) {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Couldn't open the link",
                message: err instanceof Error ? err.message : String(err),
                timestamp: new Date().toISOString(),
                type: "info",
                expiration: Date.now() + 10000,
            });
        }
        throw err;
    }
}

/** The Kanban (the one open, else a new one) with task #`num`'s card open; with `floating`, a new Kanban
 *  in a window of its own. */
export async function openTaskLink(num: number, opts: { floating?: boolean } = {}): Promise<void> {
    if (!opts.floating) return openKanbanTask(String(num));
    await RpcApi.PaneOpenCommand(
        TabRpcClient,
        { view: "kanban", meta: { view: "kanban", "kanban:open": String(num) }, tab_id: atoms.activeTabId(), floating: true },
        {}
    );
}

/** A web page in a browser pane of its own window. */
export async function openWebLinkFloating(url: string): Promise<void> {
    await RpcApi.PaneOpenCommand(TabRpcClient, { view: "browser", url, tab_id: atoms.activeTabId(), floating: true }, {});
}
