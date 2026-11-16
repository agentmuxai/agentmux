// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A one-character ASCII spinner, the app's "turn in progress" mark: the agent
// pane's Working row (beside the composer), its preparing row and the Swarm's
// status chip. The rotating slash means that and nothing else; each kind of
// long-running work has its own animation (ActivityAnimation.tsx). The shared
// clock calls it each tick and it writes its own character, so a frame never
// re-renders the text beside it and no other computation's failure can stop it
// (shared-ticker.ts). When the motion policy reduces `status`, it shows a static mark.
//
// Every animation on screen shares one timer and one tick (shared-ticker.ts).
// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10.

import { createEffect, on, onCleanup, untrack, type JSX } from "solid-js";

import { shouldReduceMotion } from "@/app/motion/motion";
import { currentTick, subscribeFrames, TICKER_MS } from "./shared-ticker";

export const SPINNER_FRAMES = ["|", "/", "-", "\\"] as const;
export const SPINNER_FRAME_MS = TICKER_MS;
export const SPINNER_STILL = "*";

/** `class` styles the mark where it sits; the default is the Working row's. */
export function AsciiSpinner(props: { class?: string }): JSX.Element {
    let el: HTMLSpanElement | undefined;
    const reducedMotion = () => shouldReduceMotion("status");
    const glyph = (tick: number) => (reducedMotion() ? SPINNER_STILL : SPINNER_FRAMES[tick % SPINNER_FRAMES.length]);
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
        <span ref={el} class={props.class ?? "agent-spinner-ascii"} aria-hidden="true">
            {untrack(() => glyph(currentTick()))}
        </span>
    );
}
