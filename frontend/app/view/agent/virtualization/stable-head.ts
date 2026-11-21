// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Split the document at `at` like `partitionAt`, but keep the head array the list already holds when the
 * head is the same nodes in the same order.
 *
 * Every flush gives the list a new document array, so a plain split makes a new head array each time, and
 * everything that follows the head (the layout feed, the node-by-id map, putting the rows on screen back)
 * runs for it: work that follows the length of the conversation, for a flush that only touched the live
 * tail. With the same array handed back, a reader of the head sees no change and none of that runs.
 * Compared from the end, where a change is most likely, by reference: a node changed in place is a new
 * object, so it still counts as a change.
 */

import { partitionAt } from "../../../agent-model/streaming-buffer";
import type { DocumentNode } from "../../../agent-model/types";

export function partitionKeepingHead(
    nodes: readonly DocumentNode[],
    at: number,
    prevHead: readonly DocumentNode[] | null,
): ReturnType<typeof partitionAt> {
    if (prevHead && at > 0 && prevHead.length === at && nodes.length >= at) {
        let i = at - 1;
        while (i >= 0 && prevHead[i] === nodes[i]) i--;
        if (i < 0) return { virtualizedNodes: prevHead as DocumentNode[], streamingNodes: nodes.slice(at), splitIndex: at };
    }
    return partitionAt(nodes, at);
}
