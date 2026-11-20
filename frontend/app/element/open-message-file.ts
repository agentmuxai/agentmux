// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A click on a file named in a container agent's message (an inline picture or video, or a card for one too big to
 * show inline): srv opens it, from the agent's own folders and as a read-only copy, and refuses a path anywhere
 * else with the reason. The path is never handed to the Editor here: this computer would read it as the person.
 * `message_of` names the agent's pane, which tells srv whose folders the path may lead into.
 */

import { atoms, pushNotification } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { fireAndForget } from "@/util/util";

const NOTICE_MS = 10_000;

export function openMessageFile(path: string, blockId: string): void {
    fireAndForget(async () => {
        try {
            await RpcApi.PaneOpenCommand(TabRpcClient, { view: "editor", file: path, message_of: blockId, tab_id: atoms.activeTabId() }, {});
        } catch (err) {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Couldn't open the file",
                message: err instanceof Error ? err.message : String(err),
                timestamp: new Date().toISOString(),
                type: "info",
                expiration: Date.now() + NOTICE_MS,
            });
        }
    });
}
