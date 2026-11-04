// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The animation of one kind of long-running work: three monospace cells that move on the shared ticker
// (shared-ticker.ts), beside AsciiSpinner's slash, which stays the sign of the turn. Under reduced motion
// each bucket shows its own still glyph, decided by the motion policy and never by a raw media query, so
// the kind of work stays readable. The glyph is `aria-hidden`: the row keeps its words, and with `since`
// the tooltip reads the bucket and how long it has run ("Building, 4m 12s").
// SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §3.

import { createEffect, onCleanup, type JSX } from "solid-js";

import { shouldReduceMotion } from "@/app/motion/motion";
import { formatDurationShort } from "@/util/format-time";
import { bucketFrame, bucketTooltip, type Bucket } from "./activity-bucket";
import { acquireTicker, sharedTick } from "./shared-ticker";
import { Tooltip } from "./tooltip";
import "./ActivityAnimation.scss";

/** `class` styles the glyph where it sits; `since` (Unix ms) turns on the tooltip. */
export function ActivityAnimation(props: { bucket: Bucket; since?: number; class?: string }): JSX.Element {
    const reducedMotion = () => shouldReduceMotion("status");
    createEffect(() => {
        if (reducedMotion()) return;
        onCleanup(acquireTicker());
    });
    return (
        <Tooltip
            disable={props.since == null}
            divClassName="activity-animation-tip"
            content={bucketTooltip(props.bucket, formatDurationShort(Date.now() - (props.since ?? Date.now())))}
        >
            <span class={`activity-animation${props.class ? ` ${props.class}` : ""}`} data-bucket={props.bucket.id} aria-hidden="true">
                {reducedMotion() ? props.bucket.still : bucketFrame(props.bucket, sharedTick())}
            </span>
        </Tooltip>
    );
}
