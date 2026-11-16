// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Activity tab's lines with a batch import's lines together: the batch's own line, and under it the line of each
// of its secrets (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §4.3). srv writes
// `batch:<id>` into the references of every line of a batch.

import type { VaultActivity } from "@/types/rpc/VaultActivity";

/** What a batch's reference starts with. */
const BATCH_REF = "batch:";
/** The action of a batch's own line. */
const BATCH_ACTION = "imported a batch";

/** A line of the Activity tab, with the lines of a batch's secrets when it is a batch's own line. */
export interface ActivityGroup {
    row: VaultActivity;
    /** The batch the line is the own line of; empty for any other line. */
    batch: string;
    items: VaultActivity[];
}

/** The batch a line belongs to, from its references; empty for a line of no batch. */
export function batchOf(a: VaultActivity): string {
    return a.refs.find((r) => r.startsWith(BATCH_REF))?.slice(BATCH_REF.length) ?? "";
}

/** `rows`, in their order (newest first), with the lines of each batch's secrets folded under the batch's own line,
 *  oldest first: the order the person read them in on the approval. A batch whose own line is not among `rows` (it
 *  is older than the list goes back) keeps its lines where they are. */
export function groupBatches(rows: VaultActivity[]): ActivityGroup[] {
    const heads = new Map<string, ActivityGroup>();
    for (const row of rows) {
        const batch = batchOf(row);
        if (batch && row.action === BATCH_ACTION && !heads.has(batch)) heads.set(batch, { row, batch, items: [] });
    }
    const out: ActivityGroup[] = [];
    for (const row of rows) {
        const head = heads.get(batchOf(row));
        if (!head) out.push({ row, batch: "", items: [] });
        else if (head.row === row) out.push(head);
        else head.items.unshift(row);
    }
    return out;
}
