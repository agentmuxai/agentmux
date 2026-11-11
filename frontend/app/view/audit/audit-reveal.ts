// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Accessor, Setter } from "solid-js";

import type { AuditFilter } from "./audit-shared";

/**
 * What the Audit does before it looks for the row a sound came from (audit-focus.ts): shows the kind of row,
 * drops an agent or a search that would leave it out (and what is typed in their boxes), and goes to the newest
 * entries, since the sound is recent and a paused or scrolled-back list does not hold them.
 */
export function revealSoundRow(
    filter: Accessor<AuditFilter>,
    setFilter: Setter<AuditFilter>,
    clearTyped: () => void,
    showNewest: () => void
): (kind: string) => void {
    return (kind) => {
        const f = filter();
        if (f.agent || f.text || !f.kinds.has(kind)) {
            clearTyped();
            setFilter({ ...f, agent: "", text: "", kinds: new Set([...f.kinds, kind]) });
        }
        showNewest();
    };
}
