// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the Working row says the turn is waiting on, beside its rotating slash (the turn is in progress;
// this is the kind of work it is stuck behind): a question or approval for the person, a rate-limit hold,
// a compaction, or the oldest tool call in flight once it has run long enough to matter.
// SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §6.

import { activityBucket, BUCKET_AFTER_MS, BUCKETS, type Bucket } from "@/app/element/activity-bucket";
import type { ActivityState, CompactionState } from "@/app/store/agent-pane-state/types";
import { formatDurationShort } from "@/util/format-time";

export interface WorkingOperation {
    bucket: Bucket;
    /** Unix ms the operation began, when known. */
    since?: number;
}

/** The parts of the Working row's props that decide the operation. */
export interface WorkingOperationInput {
    needsYou?: string | null;
    waitingReason?: "rate_limited" | null;
    compacting?: CompactionState | null;
    activity?: ActivityState | null;
}

/**
 * First match wins: the person is waited on, a rate limit holds the turn, a compaction runs, then the
 * oldest top-level tool call that has run for `BUCKET_AFTER_MS`. A subagent's own calls (`parentId`) are
 * its Agent call's business, so they never count here.
 */
export function workingOperation(row: WorkingOperationInput, now: number): WorkingOperation | null {
    if (row.needsYou) return { bucket: BUCKETS.wait };
    if (row.waitingReason === "rate_limited") return { bucket: BUCKETS.hold };
    if (row.compacting) return { bucket: activityBucket({ source: "compaction" }), since: row.compacting.startedAt };
    const tool = row.activity?.tools.find((t) => !t.parentId && now - t.startedAt >= BUCKET_AFTER_MS);
    if (!tool) return null;
    return { bucket: activityBucket({ tool: tool.name, command: tool.command }), since: tool.startedAt };
}

/** "Building 4m 12s": the bucket's word and how long it has run (the word alone when the start is unknown). */
export function workingOperationText(op: WorkingOperation, now: number): string {
    return op.since == null ? op.bucket.label : `${op.bucket.label} ${formatDurationShort(now - op.since)}`;
}
