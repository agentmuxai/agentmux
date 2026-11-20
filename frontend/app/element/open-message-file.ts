// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A click on a file named in a container agent's message (an inline picture or video, or a card for one too big to
 * show inline): srv opens it, from the agent's own folders and as a read-only copy, and refuses a path anywhere
 * else with the reason. The path is never handed to the Editor here: this computer would read it as the person.
 * `message_of` names the agent's pane, which tells srv whose folders the path may lead into. srv decides by that
 * pane's runtime; the caller's "confined" flag only picks this route. With no pane to name there is no route: nothing is
 * opened. A pane the app calls confined that srv finds is not a container agent's (one on a connection, say) is opened
 * by srv as written, which is what an agent that runs as the person is.
 */

import { atoms, pushNotification } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { fireAndForget } from "@/util/util";

const NOTICE_MS = 10_000;

export function openMessageFile(path: string, blockId: string | undefined): void {
    if (!blockId) {
        notify("This message isn't tied to its agent's pane, so its file can't be opened from here.");
        return;
    }
    fireAndForget(async () => {
        try {
            await RpcApi.PaneOpenCommand(TabRpcClient, { view: "editor", file: path, message_of: blockId, tab_id: atoms.activeTabId() }, {});
        } catch (err) {
            notify(err instanceof Error ? err.message : String(err));
        }
    });
}

function notify(message: string): void {
    pushNotification({
        icon: "fa-triangle-exclamation",
        title: "Couldn't open the file",
        message,
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + NOTICE_MS,
    });
}
