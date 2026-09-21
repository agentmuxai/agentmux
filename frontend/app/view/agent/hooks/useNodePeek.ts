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
 * Both handlers no-op while the mouse's primary button is held (see
 * `pointer-drag-state.ts`): a header row's mouseenter/mouseleave still
 * fires normally while the user is dragging out a text selection (native
 * selection-drag does not suppress hover events), and letting either one
 * through mid-drag mounts/unmounts the Portal-rendered `PeekOverlay` under
 * the cursor, which was intermittently breaking the browser's native
 * selection-extend hit-testing — see
 * docs/plans/PLAN_AGENT_PANE_TEXT_SELECTION_DRAG_FLICKER_2026_09_20.md.
 * Freezing whatever peek state was already showing (rather than only
 * gating entry) also matches the desired UX: no peek chrome popping in and
 * out while the user is mid-drag-select.
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
        timer = setTimeout(() => setIsPeeking(true), delayMs);
    };
    const handlePeekLeave = () => {
        if (isPrimaryButtonDown()) return;
        clearTimeout(timer);
        setIsPeeking(false);
    };
    onCleanup(() => clearTimeout(timer));

    return { isPeeking, rowEl, setRowEl, handlePeekEnter, handlePeekLeave };
}
