// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which renderer each terminal draws with, by block id, for the stats badge's
 * hover panel. `termRendererAtom` (store/global.ts) is one value for the whole
 * app, set by whichever terminal last loaded, so it can't say which terminals
 * fell back to the DOM renderer.
 */

import { createSignal } from "solid-js";

export type TermRendererKind = "webgl" | "dom" | "lost";

export interface TermRendererState {
    kind: TermRendererKind;
    /** Why, for the DOM renderer or a lost context. */
    why?: string;
}

const [renderers, setRenderers] = createSignal<ReadonlyMap<string, TermRendererState>>(new Map());

export function termRendererOf(blockId: string): TermRendererState | undefined {
    return renderers().get(blockId);
}

export function setTermRendererOf(blockId: string, state: TermRendererState): void {
    setRenderers((prev) => new Map(prev).set(blockId, state));
}

export function forgetTermRenderer(blockId: string): void {
    setRenderers((prev) => {
        if (!prev.has(blockId)) return prev;
        const next = new Map(prev);
        next.delete(blockId);
        return next;
    });
}

/** The hover panel's row: the renderer, and what that means. */
export function describeTermRenderer(state: TermRendererState): { value: string; meaning: string } {
    switch (state.kind) {
        case "webgl":
            return { value: "WebGL (GPU)", meaning: "drawn on the graphics card" };
        case "lost":
            return { value: "WebGL, lost", meaning: state.why ?? "its graphics context was lost; switching to DOM" };
        case "dom":
            return { value: "DOM (software)", meaning: state.why ?? "drawn as page elements, without the GPU" };
    }
}
