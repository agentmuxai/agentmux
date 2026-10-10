// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Working row's "still working" signal: a one-character ASCII spinner in
// the pane's color, in place of the pulsing dot. It animates on its own
// signal, so a frame never re-renders the status text beside it. Reduced
// motion shows a static mark. SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.10.

import { createEffect, createSignal, onCleanup, type JSX } from "solid-js";

import { atoms } from "@/app/store/global";

export const SPINNER_FRAMES = ["|", "/", "-", "\\"] as const;
export const SPINNER_FRAME_MS = 120;
export const SPINNER_STILL = "*";

export function AsciiSpinner(): JSX.Element {
    const [frame, setFrame] = createSignal(0);
    const reducedMotion = atoms.prefersReducedMotionAtom;
    createEffect(() => {
        if (reducedMotion()) return;
        const id = setInterval(() => setFrame((f) => (f + 1) % SPINNER_FRAMES.length), SPINNER_FRAME_MS);
        onCleanup(() => clearInterval(id));
    });
    return (
        <span class="agent-spinner-ascii" aria-hidden="true">
            {reducedMotion() ? SPINNER_STILL : SPINNER_FRAMES[frame()]}
        </span>
    );
}
