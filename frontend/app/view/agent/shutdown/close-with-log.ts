// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isAllBlocksGoneError } from "@/app/tab/pane-close-guard";
import { pushFlashError } from "@/app/store/flash-notifications";
import * as services from "@/store/services";
import { removeMovedBlock } from "@/layout/lib/layoutMagnify";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { beginShutdownLog, endShutdownLog, failShutdownLog } from "./shutdown-log";

/**
 * Close agent panes in place (§5.5): the pane stays, covered by its shutdown
 * log, and srv removes each tab from the layout once it is down — the
 * `delete` actions it queues for a waiting frontend. srv reports failures
 * during the close in the log itself; a rejection before it started is put
 * there too, so the pane offers Keep open / Try again instead of hanging.
 * Shared by the pane × and the overlay's Try again, so neither can drop a
 * rejection (ReAgent P1 on #3784).
 */
export function closeWithShutdownLog(tabId: string, blockIds: string[], isAgent: (blockId: string) => boolean): Promise<void> {
    const agentIds = blockIds.filter(isAgent);
    for (const id of agentIds) beginShutdownLog(id);
    return services.ObjectService.ClosePane(blockIds, true).then(
        () => {},
        (err) => {
            if (isAllBlocksGoneError(err)) {
                // Every block was already gone, so srv can't name their tab to
                // queue the layout change: drop them here instead (a safe no-op
                // for any the layout no longer has).
                const model = getLayoutModelForTabById(tabId);
                for (const id of blockIds) {
                    if (model) removeMovedBlock(model, id);
                    endShutdownLog(id);
                }
                return;
            }
            for (const id of agentIds) failShutdownLog(id, String(err));
            pushFlashError({
                id: "",
                icon: "triangle-exclamation",
                title: "Couldn't close the pane",
                message: String(err),
                expiration: Date.now() + 10_000,
            });
        },
    );
}
