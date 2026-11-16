// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A one-character ASCII spinner, the app's "turn in progress" mark: the agent
// pane's Working row (beside the composer), its preparing row and the Swarm's
// status chip. The rotating slash means that and nothing else; each kind of
// long-running work has its own animation (ActivityAnimation.tsx). It animates
// on its own signal, so a frame never re-renders the text beside it. When the
// motion policy reduces `status`, it shows a static mark.
//
// Every animation on screen shares one timer and one tick (shared-ticker.ts).
// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10.

import { createEffect, onCleanup, type JSX } from "solid-js";

import { shouldReduceMotion } from "@/app/motion/motion";
import { acquireTicker, registerMark, sharedTick, TICKER_MS } from "./shared-ticker";

export const SPINNER_FRAMES = ["|", "/", "-", "\\"] as const;
export const SPINNER_FRAME_MS = TICKER_MS;
export const SPINNER_STILL = "*";

/** `class` styles the mark where it sits; the default is the Working row's. */
export function AsciiSpinner(props: { class?: string }): JSX.Element {
    const reducedMotion = () => shouldReduceMotion("status");
    let drew: (tick: number) => void = () => {};
    createEffect(() => {
        if (reducedMotion()) return;
        const mark = registerMark();
        drew = mark.drew;
        onCleanup(acquireTicker());
        onCleanup(() => {
            drew = () => {};
            mark.release();
        });
    });
    const frame = () => {
        if (reducedMotion()) return SPINNER_STILL;
        const t = sharedTick();
        drew(t);
        return SPINNER_FRAMES[t % SPINNER_FRAMES.length];
    };
    return (
        <span class={props.class ?? "agent-spinner-ascii"} aria-hidden="true">
            {frame()}
        </span>
    );
}
