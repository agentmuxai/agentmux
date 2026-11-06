// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's one list, cut into a labelled part per place its agents run: this machine and channel
// first, then each other channel on it, each LAN host and the cloud (the order `remoteSections` gives).
// The cut keeps the order the list holds inside every part, and leaves out a part with nothing to show,
// so a filter that hides every row of a place hides its label too.

import type { RemoteSection } from "./swarm-remote";

export const LOCAL_PART_ID = "local";

/** One labelled part of the list: the keys of its rows, in the list's order. */
export interface OriginPart {
    id: string;
    label: string;
    keys: string[];
    /** This instance's own part, the one the stopped agents are listed in. */
    local: boolean;
}

/** This machine's label: its host name, with the channel when other channels run on it too. */
export function localPartLabel(hostname: string | undefined, channel: string | undefined, otherChannels: boolean): string {
    const host = hostname || "This machine";
    return otherChannels && channel ? `${host} · ${channel}` : host;
}

/**
 * Cuts `order` (every row key, this instance's and the others') into parts. `remote` maps a remote row's
 * key to its row; a key not in it is this instance's. Parts follow `sections` (the other instances in their
 * display order). The local part is kept while it has `localExtra` to list (stopped agents) even with no row.
 */
export function partsByOrigin(
    order: readonly string[],
    remote: ReadonlyMap<string, { section: Pick<RemoteSection, "key"> }>,
    sections: readonly Pick<RemoteSection, "key" | "title">[],
    localLabel: string,
    localExtra: boolean
): OriginPart[] {
    const parts = new Map<string, OriginPart>();
    parts.set(LOCAL_PART_ID, { id: LOCAL_PART_ID, label: localLabel, keys: [], local: true });
    for (const s of sections) parts.set(s.key, { id: s.key, label: s.title, keys: [], local: false });
    for (const key of order) {
        const origin = remote.get(key)?.section.key;
        const part = parts.get(origin ?? LOCAL_PART_ID);
        // A remote row whose section went between the two reads has no part to show in; the next read drops it.
        if (part) part.keys.push(key);
    }
    return [...parts.values()].filter((p) => p.keys.length > 0 || (p.local && localExtra));
}
