// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The mark at the start of a dock row. A running row shows the animation of its kind of work once it has
// run for BUCKET_AFTER_MS (before that, and once it is over, the row's own glyph); the rotating slash is
// not used here, it stays the turn's sign. SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §6.

import { Show, createMemo, type Accessor, type JSX } from "solid-js";

import { ActivityAnimation } from "@/app/element/ActivityAnimation";
import { activityBucket, BUCKET_AFTER_MS, type Bucket } from "@/app/element/activity-bucket";
import type { PinnedActivity } from "../../../agent-model/activity-types";

/** The kind of work a dock row is: a subagent, or by its shell command or tool. */
export function bucketOfActivity(a: PinnedActivity): Bucket {
    if (a.kind === "subagent") return activityBucket({ source: "subagent" });
    const command = a.shell?.cmd ?? (a.tool?.params as { command?: string } | undefined)?.command;
    return activityBucket({ tool: a.tool?.toolName ?? a.tool?.tool, command });
}

/** `tick` is the row's 1 s clock: it moves a row past the threshold without an event. */
export function ActivitySigil(props: { activity: PinnedActivity; glyph: string; tick: Accessor<number> }): JSX.Element {
    const bucket = createMemo(() => bucketOfActivity(props.activity));
    const long = () => (props.tick(), Date.now() - props.activity.startedAt >= BUCKET_AFTER_MS);
    return (
        <Show when={props.activity.status === "running" && long()} fallback={props.glyph}>
            <ActivityAnimation bucket={bucket()} since={props.activity.startedAt} />
        </Show>
    );
}
