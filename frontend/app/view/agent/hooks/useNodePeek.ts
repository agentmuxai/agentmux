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
 */

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { PEEK_ENTER_DELAY_MS } from "../components/hover-anchor";

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
        clearTimeout(timer);
        timer = setTimeout(() => setIsPeeking(true), delayMs);
    };
    const handlePeekLeave = () => {
        clearTimeout(timer);
        setIsPeeking(false);
    };
    onCleanup(() => clearTimeout(timer));

    return { isPeeking, rowEl, setRowEl, handlePeekEnter, handlePeekLeave };
}
