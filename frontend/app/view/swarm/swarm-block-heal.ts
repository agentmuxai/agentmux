// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A running agent the Swarm did not list because this window's copy of its block said "not found".
 *
 * The Swarm lists the blocks srv has a controller for, and shows a row only while the window can read
 * the block (`hasRenderableBlock`). The window's object cache keeps a "not found" answer, or a
 * delete push, until another push for that block arrives, so an agent whose block it had once
 * read as missing stayed out of that pane for good although srv listed it every time. A pane opened
 * later reads the block afresh, which is why only the long-open one lacked the agent.
 *
 * Each time the list of controller-backed blocks is read, the ones the cache holds as missing are read
 * again. A block that is really gone is not in that list, so this asks about live agents only.
 */

import { MOS } from "@/app/store/global";

/** What the heal reads and asks of the window's object cache. */
export interface BlockCacheAccess {
    /** The cache holds `oref` as read and not found (not still loading, not failed to read). */
    isAbsent(oref: string): boolean;
    /** Reads `oref` from srv into the cache. */
    reload(oref: string): Promise<unknown>;
}

/** The block ids being read again, so a poll that comes before the answer does not ask twice. */
const inFlight = new Set<string>();

/** Reads again each of `blockIds` the cache holds as missing. Returns how many reads it started. */
export function refetchAbsentBlocks(blockIds: readonly string[], cache: BlockCacheAccess): number {
    let started = 0;
    for (const blockId of blockIds) {
        if (inFlight.has(blockId)) continue;
        const oref = `block:${blockId}`;
        if (!cache.isAbsent(oref)) continue;
        inFlight.add(blockId);
        started++;
        // A block srv does not have either rejects: it stays missing and the row stays out.
        void cache
            .reload(oref)
            .catch(() => {})
            .finally(() => inFlight.delete(blockId));
    }
    return started;
}

/** The window's object cache, as the heal reads it: a block read as not found, and finished loading. */
const WINDOW_BLOCKS: BlockCacheAccess = {
    isAbsent: (oref) => MOS.getMuxObjectLoadingAtom(oref)() === false && MOS.getObjectValue(oref) == null,
    reload: (oref) => MOS.reloadMuxObject(oref),
};

/** `refetchAbsentBlocks` against this window's object cache. Returns `blockIds` as given. */
export function refetchAbsentWindowBlocks(blockIds: string[]): string[] {
    refetchAbsentBlocks(blockIds, WINDOW_BLOCKS);
    return blockIds;
}
