// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useNodePeek — shared hover-to-peek timer/state.
 *
 * Factored out of ToolBlock.tsx/MarkdownBlock.tsx/UserMessageBlock.tsx,
 * which each hand-rolled an identical isPeeking signal + enter-delay timer
 * before SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25 extended the
 * peek overlay to every document-node kind — three (soon many more)
 * near-identical copies of this exact logic was the point where drift
 * (a delay changed in one file but not the others, the exact bug this
 * feature just went through) becomes a real risk rather than a
 * hypothetical one.
 *
 * `rowEl` is a signal (not a plain `let` closure var, unlike the original
 * three copies) so it can be handed straight to `<PeekOverlay rowEl={rowEl}>`
 * without each caller re-deriving its own accessor wrapper.
 *
 * Entry (mount) no-ops while the mouse's primary button is held (see
 * `pointer-drag-state.ts`): a header row's mouseenter still fires normally
 * while the user is dragging out a text selection (native selection-drag
 * does not suppress hover events), and mounting the Portal-rendered
 * `PeekOverlay` under the cursor mid-drag was intermittently breaking the
 * browser's native selection-extend hit-testing — see
 * docs/plans/PLAN_AGENT_PANE_TEXT_SELECTION_DRAG_FLICKER_2026_09_20.md.
 * Leave (unmount) is deliberately NOT gated: removing an overlay from
 * under the cursor doesn't reintroduce that hit-testing problem, and
 * freezing it too — reagentx P1 on PR #3470 — left a peek stuck open
 * indefinitely whenever its row's mouseleave fired mid-drag but the mouse
 * button was later released somewhere else, since no further
 * mouseenter/mouseleave would ever fire for a row the cursor had already
 * left.
 */

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { PEEK_ENTER_DELAY_MS } from "../components/hover-anchor";
import { isPrimaryButtonDown } from "@/app/util/pointer-drag-state";

export interface NodePeek {
    isPeeking: Accessor<boolean>;
    rowEl: Accessor<HTMLElement | undefined>;
    setRowEl: (el: HTMLElement) => void;
    handlePeekEnter: () => void;
    handlePeekLeave: () => void;
}

export function useNodePeek(delayMs: number = PEEK_ENTER_DELAY_MS): NodePeek {
    const [isPeeking, setIsPeeking] = createSignal(false);
    const [rowEl, setRowEl] = createSignal<HTMLElement>();
    let timer: ReturnType<typeof setTimeout> | undefined;

    const handlePeekEnter = () => {
        if (isPrimaryButtonDown()) return;
        clearTimeout(timer);
        // Re-checked when the delay elapses, not just at call time: the
        // primary button can go down during the delay window (enter fired
        // just before mousedown), and without this check the timeout would
        // still mount the overlay mid-drag.
        timer = setTimeout(() => {
            if (isPrimaryButtonDown()) return;
            setIsPeeking(true);
        }, delayMs);
    };
    const handlePeekLeave = () => {
        clearTimeout(timer);
        setIsPeeking(false);
    };
    onCleanup(() => clearTimeout(timer));

    return { isPeeking, rowEl, setRowEl, handlePeekEnter, handlePeekLeave };
}
