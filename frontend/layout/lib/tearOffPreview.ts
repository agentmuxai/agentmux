// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The tear-off of a pane drag, shown before the release. A whole-pane drag released over the pane's own slot
 * tears the pane off (app/drag/pane-self-release.ts), and nothing on screen said so: the pane sat in its slot
 * as if the release would put it back. Once the pointer rests there, the layout shows what the release leaves:
 * the pane's tile goes and the other panes close over its slot (the live reflow, liveReflow.ts), as a
 * browser's tab strip closes over a tab pulled out of it. The pane itself is the picture in hand.
 *
 * It waits for the pointer to rest, because every drag starts over its own pane: a drag on its way to a
 * neighbour would otherwise move every pane twice. Once shown it stays while the pointer is over the pane, and
 * through the release until the tree has let the pane go, so the panes do not spring back between the release
 * and the tear-off.
 */

import { createEffect, createSignal, on, type Accessor } from "solid-js";
import type { LayoutModel } from "./layoutModel";

/** How long the pointer rests over the dragged pane before the layout closes over its slot. */
export const TEAR_OFF_REST_MS = 220;
/** A pointer that moved less than this since it came to rest is still resting, px. */
export const TEAR_OFF_REST_PX = 6;
/** How long a released tear-off stays shown while the tree still has the pane; then the panes go back. */
export const TEAR_OFF_COMMIT_WAIT_MS = 2000;

interface Rest {
    nodeId: string;
    /** Where the pointer came to rest, in the layout container's coordinates. */
    x: number;
    y: number;
    timer?: ReturnType<typeof setTimeout>;
    /** The drag was released over the pane: the tear-off is on its way. */
    committed?: boolean;
}

interface State {
    shown: Accessor<string | undefined>;
    setShown: (nodeId: string | undefined) => void;
    rest?: Rest;
}

const states = new WeakMap<LayoutModel, State>();

/**
 * The pane whose tear-off `model`'s layout shows now, or none. Made in the model's reactive owner, once per
 * model.
 */
export function createTearOffPreview(model: LayoutModel): Accessor<string | undefined> {
    const [shown, setShown] = createSignal<string | undefined>(undefined);
    const state: State = { shown, setShown };
    states.set(model, state);

    // The drag ended. A release over the pane commits the tear-off in the same turn (the layout's drop handler,
    // before or after this one), so what is left by the end of the turn was a cancel or a drop elsewhere.
    createEffect(
        on(
            () => model.activeDrag(),
            (active) => {
                const rest = state.rest;
                if (active || !rest) return;
                queueMicrotask(() => {
                    if (state.rest === rest && !rest.committed) leaveOwnPane(model);
                });
            },
            { defer: true }
        )
    );

    // The tree let the pane go: the tear-off happened, and the layout is what was shown.
    createEffect(() => {
        const nodeId = shown();
        if (nodeId && !model.leafs().some((leaf) => leaf.id === nodeId)) leaveOwnPane(model);
    });

    return shown;
}

/** A sample of the drag of `nodeId` with the pointer over that pane itself, at (x, y) in the layout container. */
export function restOverOwnPane(model: LayoutModel, nodeId: string, x: number, y: number): void {
    const state = states.get(model);
    if (!state) return;
    const rest = state.rest;
    if (rest?.nodeId === nodeId) {
        if (rest.committed || state.shown() === nodeId) return;
        if (Math.hypot(x - rest.x, y - rest.y) < TEAR_OFF_REST_PX) return;
    }
    clearTimeout(rest?.timer);
    const next: Rest = { nodeId, x, y };
    next.timer = setTimeout(() => {
        if (state.rest === next && model.activeDrag()) state.setShown(nodeId);
    }, TEAR_OFF_REST_MS);
    state.rest = next;
}

/** The pointer is no longer over the dragged pane, or the drag is over: nothing is torn off. */
export function leaveOwnPane(model: LayoutModel): void {
    const state = states.get(model);
    if (!state?.rest) return;
    clearTimeout(state.rest.timer);
    state.rest = undefined;
    state.setShown(undefined);
}

/**
 * The drag of `nodeId` was released over the pane itself: it tears off. The layout shows it from now, rested
 * or not, until the tree lets the pane go, or for `TEAR_OFF_COMMIT_WAIT_MS` if it never does.
 */
export function commitTearOff(model: LayoutModel, nodeId: string): void {
    const state = states.get(model);
    if (!state) return;
    clearTimeout(state.rest?.timer);
    const rest: Rest = { nodeId, x: 0, y: 0, committed: true };
    rest.timer = setTimeout(() => {
        if (state.rest === rest) leaveOwnPane(model);
    }, TEAR_OFF_COMMIT_WAIT_MS);
    state.rest = rest;
    state.setShown(nodeId);
}
