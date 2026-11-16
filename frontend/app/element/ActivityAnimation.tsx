// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The animation of one kind of long-running work: three monospace cells that move on the shared ticker
// (shared-ticker.ts), beside AsciiSpinner's slash, which stays the sign of the turn. Under reduced motion
// each bucket shows its own still glyph, decided by the motion policy and never by a raw media query, so
// the kind of work stays readable. The glyph is `aria-hidden`: the row keeps its words, and with `since`
// the tooltip reads the bucket and how long it has run ("Building, 4m 12s").
// SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §3.

import { createEffect, on, onCleanup, untrack, type JSX } from "solid-js";

import { shouldReduceMotion } from "@/app/motion/motion";
import { formatDurationShort } from "@/util/format-time";
import { bucketFrame, bucketTooltip, type Bucket } from "./activity-bucket";
import { currentTick, subscribeFrames } from "./shared-ticker";
import { Tooltip } from "./tooltip";
import "./ActivityAnimation.scss";

/** `class` styles the glyph where it sits; `since` (Unix ms) turns on the tooltip. The shared clock calls it each
 *  tick and it writes its own glyph, so no other computation's failure can stop it (shared-ticker.ts). */
export function ActivityAnimation(props: { bucket: Bucket; since?: number; class?: string }): JSX.Element {
    let el: HTMLSpanElement | undefined;
    const reducedMotion = () => shouldReduceMotion("status");
    const glyph = (tick: number) => (reducedMotion() ? props.bucket.still : bucketFrame(props.bucket, tick));
    let stop: (() => void) | undefined;
    const follow = (on: boolean) => {
        stop?.();
        stop = on
            ? subscribeFrames((tick) => {
                  if (el) el.textContent = glyph(tick);
              })
            : undefined;
    };
    // Following starts here, not in an effect: an effect queued in a flush that throws never runs.
    follow(!untrack(reducedMotion));
    createEffect(
        on(
            reducedMotion,
            (reduced) => {
                follow(!reduced);
                if (el) el.textContent = glyph(currentTick());
            },
            { defer: true }
        )
    );
    onCleanup(() => stop?.());
    return (
        <Tooltip
            disable={props.since == null}
            divClassName="activity-animation-tip"
            content={bucketTooltip(props.bucket, formatDurationShort(Date.now() - (props.since ?? Date.now())))}
        >
            <span
                ref={el}
                class={`activity-animation${props.class ? ` ${props.class}` : ""}`}
                data-bucket={props.bucket.id}
                aria-hidden="true"
            >
                {untrack(() => glyph(currentTick()))}
            </span>
        </Tooltip>
    );
}
