// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The settling pass: how the panes take their places after a pane left the tree outside a drag (a close, or a
 * pane moved to another window). The others used to take their new rects in one frame. Now each pane that
 * changed place or size glides there by the live reflow's own means (liveReflow.ts: four edges, a clip, the
 * size set once underneath), and the frame of the pane that left fades where it stood.
 *
 * The tree, the layout's rects, focus and what goes to srv are final before a pass opens: only the drawing
 * glides. A pass opens in the same batch as the new rects, with every tile still at the rect it showed, and
 * sends them to their own rects on the next frame, so the glide has a start. It ends when the glide is over
 * (`reflowSettleMs`), and the tiles' plain style, already final, takes over. Terminals keep their grid for the
 * pass and fit once at its end, by the reflow's own hold.
 *
 * A pass never opens over a drag's reflow: a drop and a tear-off commit what the panes already show.
 * docs/specs/SPEC_PANE_REFLOW_GLIDE_2026_11_01.md §6.
 */

import { createEffect, createSignal, on, untrack, type Accessor } from "solid-js";
import { paneCaptureInFlight } from "@/app/drag/pane-capture-gate";
import { scaledMs } from "@/app/motion/motion";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import type { LayoutModel } from "./layoutModel";
import {
    liveReflowEnabled,
    movesWithReflow,
    onNextFrame,
    REFLOW_EDGES,
    REFLOW_MAX_PANES,
    reflowSettleMs,
    shownPx,
} from "./liveReflow";
import type { LayoutNode, LayoutNodeAdditionalProps } from "./types";

/** One pane's glide. */
export interface PaneMove {
    /** The rect the pane showed before the change. None for a pane that was already gliding: it goes on from where it is drawn. */
    from?: Dimensions;
    /** Its rect in the layout now. */
    to: Dimensions;
}

/** The frame of a pane that left, at the rect it had. */
export interface PaneGhost {
    id: string;
    rect: Dimensions;
}

export interface SettlingPass {
    /** The panes that changed place or size, by node id. */
    moves: Map<string, PaneMove>;
    /** The panes that left. */
    ghosts: PaneGhost[];
    /** False for the frame the pass opens on (the tiles stand at `from`), true from the next. */
    gliding: boolean;
}

interface State {
    pass: Accessor<SettlingPass | undefined>;
    setPass: (pass: SettlingPass | undefined) => void;
    /** Stops the open pass's frame and timer. */
    cancel?: () => void;
    /**
     * A drag's reflow was on screen in this task. The source's release ends the drag before the drop commits
     * the tree, in the same task, so by the commit no drag is live and the panes still show the drop.
     */
    dragShown: boolean;
}

const states = new WeakMap<LayoutModel, State>();

type Props = Record<string, LayoutNodeAdditionalProps>;

/** The settling pass open on `model`'s layout, or none. Made in the model's reactive owner, once per model. */
export function createSettlingPass(model: LayoutModel): Accessor<SettlingPass | undefined> {
    const [pass, setPass] = createSignal<SettlingPass | undefined>(undefined);
    const state: State = { pass, setPass, dragShown: false };
    states.set(model, state);

    createEffect(
        on(
            () => model.activeDrag() || !!model.tearOffLayout(),
            (live) => {
                if (live) state.dragShown = true;
                else setTimeout(() => (state.dragShown = untrack(() => model.activeDrag() || !!model.tearOffLayout())), 0);
            }
        )
    );

    // A pane's picture is a capture of the window: a fading frame or a pane mid-glide must not be in it.
    createEffect(() => {
        if (paneCaptureInFlight()) closeSettlingPass(model);
    });

    return pass;
}

/** Whether the two rects are the same as a tile shows them (`shownPx`): a pane that moved less does not glide. */
function sameShown(a: Dimensions, b: Dimensions): boolean {
    const [p, q] = [shownPx(a), shownPx(b)];
    return p.left === q.left && p.top === q.top && p.width === q.width && p.height === q.height;
}

/**
 * Where the tile of a pane in the middle of its glide stands now: its four edges as their transition has them
 * (the tile's element carries the node's id), or `fallback` where they cannot be read.
 */
function drawnRect(nodeId: string, fallback: Dimensions): Dimensions {
    const tile = typeof document === "undefined" ? null : document.getElementById(nodeId);
    if (!tile) return fallback;
    const style = getComputedStyle(tile);
    const [left, top, right, bottom] = REFLOW_EDGES.map((edge) => parseFloat(style.getPropertyValue(edge)));
    if (![left, top, right, bottom].every(Number.isFinite)) return fallback;
    return { left, top, width: right - left, height: bottom - top };
}

/** Whether a change of `model`'s tree may glide now. */
function mayGlide(model: LayoutModel, state: State, paneCount: number): boolean {
    if (!liveReflowEnabled() || scaledMs(DURATION_BASE_MS) <= 0) return false;
    if (paneCount > REFLOW_MAX_PANES || paneCaptureInFlight()) return false;
    if (state.dragShown || model.activeDrag() || model.tearOffLayout()) return false;
    // Magnified now, or as last published (the pane that closed was the magnified one): the tiles are hidden.
    if (model.isResizing() || model.treeState.magnifiedNodeId || model.magnifiedNodeIdAtom()) return false;
    // A window tab that is not shown keeps its layout mounted (workspace.tsx marks it, pane-overlay.ts
    // `HIDDEN_TAB_ATTR`): nothing of it is drawn, and a pass would hold the shown tab's terminals too.
    if (model.displayContainerRef.current?.closest?.("[data-window-tab-hidden]")) return false;
    return typeof document === "undefined" || document.visibilityState !== "hidden";
}

/**
 * The pass for a change of `model`'s layout from `prevLeafs` at `prevProps` to `nextLeafs` at `nextProps`, or
 * none when the change does not glide: no pane left or one came, the reflow is off or the animations are, a
 * drag's reflow is on screen, the layout is being resized or magnified, its tab or window is not shown, or
 * nothing a tile shows changed.
 */
export function planSettlingPass(
    model: LayoutModel,
    prevLeafs: LayoutNode[],
    prevProps: Props,
    nextLeafs: LayoutNode[],
    nextProps: Props
): SettlingPass | undefined {
    const state = states.get(model);
    if (!state || prevLeafs.length === 0) return undefined;
    const nextIds = new Set(nextLeafs.map((leaf) => leaf.id));
    const gone = prevLeafs.filter((leaf) => !nextIds.has(leaf.id));
    if (gone.length === 0 || nextLeafs.length + gone.length !== prevLeafs.length) return undefined;
    if (!untrack(() => mayGlide(model, state, prevLeafs.length))) return undefined;

    const open = untrack(state.pass);
    const moves = new Map<string, PaneMove>();
    for (const leaf of nextLeafs) {
        const to = nextProps[leaf.id]?.rect;
        if (!to || !movesWithReflow(leaf)) continue;
        const gliding = open?.moves.get(leaf.id);
        if (gliding) {
            moves.set(leaf.id, open!.gliding ? { to } : { from: gliding.from, to });
            continue;
        }
        const from = prevProps[leaf.id]?.rect;
        if (from && !sameShown(from, to)) moves.set(leaf.id, { from, to });
    }
    const ghosts = [...(open?.ghosts ?? [])];
    for (const leaf of gone) {
        const rect = prevProps[leaf.id]?.rect;
        if (!rect) continue;
        // A pane that was gliding itself is not at its layout rect yet: its ghost stands where the pane is
        // drawn, its start while its glide has not begun and its travelling edges after.
        const own = open?.moves.get(leaf.id);
        const stood = !own ? rect : own.from && !open!.gliding ? own.from : drawnRect(leaf.id, rect);
        ghosts.push({ id: leaf.id, rect: stood });
    }
    return moves.size > 0 || ghosts.length > 0 ? { moves, ghosts, gliding: false } : undefined;
}

/** Open `pass` on `model`'s layout, in place of one that is open. Call it in the batch that writes the new rects. */
export function openSettlingPass(model: LayoutModel, pass: SettlingPass): void {
    const state = states.get(model);
    if (!state) return;
    state.cancel?.();
    state.setPass(pass);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cancelFrame = onNextFrame(() => {
        state.setPass({ ...pass, gliding: true });
        timer = setTimeout(() => closeSettlingPass(model), reflowSettleMs());
    });
    state.cancel = () => {
        cancelFrame();
        clearTimeout(timer);
    };
}

/** End the pass open on `model`'s layout: every tile takes its own rect at once. */
export function closeSettlingPass(model: LayoutModel): void {
    const state = states.get(model);
    if (!state) return;
    state.cancel?.();
    state.cancel = undefined;
    if (untrack(state.pass)) state.setPass(undefined);
}

/**
 * The layout pass of `model` is about to publish `nextLeafs` at `nextProps` (`updateTree`, inside its batch,
 * before the write): open a pass when a pane left, or end the open one when the rects it glides to are no
 * longer the layout's (a resize in the middle of a glide).
 */
export function settleTreeChange(model: LayoutModel, nextLeafs: LayoutNode[], nextProps: Props): void {
    const pass = planSettlingPass(model, untrack(model.leafs), untrack(model.additionalProps), nextLeafs, nextProps);
    if (pass) {
        openSettlingPass(model, pass);
        return;
    }
    const open = untrack(() => states.get(model)?.pass());
    if (!open) return;
    for (const [id, move] of open.moves) {
        const rect = nextProps[id]?.rect;
        if (!rect || !sameShown(rect, move.to)) {
            closeSettlingPass(model);
            return;
        }
    }
}
