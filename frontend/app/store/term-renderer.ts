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
    /** The mount that set it; see `claimTermRenderer`. */
    owner?: object;
}

/**
 * Which mount of each block's terminal may report its renderer. A block's
 * terminal can be mounted again before the old mount is disposed, and the old
 * mount's canvas can still lose or regain its context; only the newest mount,
 * which claims the block when it is created, may set or clear the entry.
 */
const claims = new Map<string, object>();

export function claimTermRenderer(blockId: string, owner: object): void {
    claims.set(blockId, owner);
}

const [renderers, setRenderers] = createSignal<ReadonlyMap<string, TermRendererState>>(new Map());

export function termRendererOf(blockId: string): TermRendererState | undefined {
    return renderers().get(blockId);
}

/** Set a block's renderer; ignored from a mount that no longer holds the claim. */
export function setTermRendererOf(blockId: string, state: TermRendererState): void {
    if (state.owner && claims.get(blockId) !== state.owner) return;
    setRenderers((prev) => new Map(prev).set(blockId, state));
}

/** Drop a block's entry when its terminal closes, if `owner` still holds the claim. */
export function forgetTermRenderer(blockId: string, owner?: object): void {
    if (owner) {
        if (claims.get(blockId) !== owner) return;
        claims.delete(blockId);
    }
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
