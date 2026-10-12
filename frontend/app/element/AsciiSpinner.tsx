// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A one-character ASCII spinner, the app's "still working" mark: the agent
// pane's Working row (beside the composer) and the Swarm's status chip.
// It replaced a pulsing dot and animates on its own signal, so a frame never
// re-renders the text beside it. Reduced motion shows a static mark.
//
// Every spinner on screen shares one timer and one frame: the Swarm can show
// dozens at once, and independent timers would give each its own phase, which
// reads as a strobe rather than one steady motion. The timer runs only while
// at least one spinner is mounted.
// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10.

import { createEffect, createSignal, onCleanup, type JSX } from "solid-js";

import { atoms } from "@/app/store/global";

export const SPINNER_FRAMES = ["|", "/", "-", "\\"] as const;
export const SPINNER_FRAME_MS = 120;
export const SPINNER_STILL = "*";

const [frame, setFrame] = createSignal(0);
let mounted = 0;
let timer: ReturnType<typeof setInterval> | undefined;

function acquireTicker(): () => void {
    if (mounted++ === 0) {
        timer = setInterval(() => setFrame((f) => (f + 1) % SPINNER_FRAMES.length), SPINNER_FRAME_MS);
    }
    return () => {
        if (--mounted === 0) {
            clearInterval(timer);
            timer = undefined;
            setFrame(0);
        }
    };
}

/** `class` styles the mark where it sits; the default is the Working row's. */
export function AsciiSpinner(props: { class?: string }): JSX.Element {
    const reducedMotion = atoms.prefersReducedMotionAtom;
    createEffect(() => {
        if (reducedMotion()) return;
        onCleanup(acquireTicker());
    });
    return (
        <span class={props.class ?? "agent-spinner-ascii"} aria-hidden="true">
            {reducedMotion() ? SPINNER_STILL : SPINNER_FRAMES[frame()]}
        </span>
    );
}
