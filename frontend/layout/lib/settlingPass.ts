// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The settling pass: how the panes take their places after the arrangement changed outside a drag: a pane
 * left (a close, or a pane moved to another window), a pane came (an open, a split), a pane was minimized or
 * restored, or two were swapped. The panes used to take their new rects in one frame. Now each pane that
 * changed place or size glides there by the live reflow's own means (liveReflow.ts: four edges, a clip, the
 * size set once underneath), the frame of a pane that left fades where it stood, and a pane that came or was
 * restored fades in at its own rect.
 *
 * The tree, the layout's rects, focus and what goes to srv are final before a pass opens: only the drawing
 * glides. A pass opens in the same batch as the new rects, with every tile still at the rect it showed, and
 * sends them to their own rects on the first frame after that which runs on time (`whenFramesFlow`), so the
 * glide has a start and does not begin while the panes are being laid out at their new sizes. It ends when
 * the glide is over
 * (`reflowSettleMs`), and the tiles' plain style, already final, takes over. Terminals keep their grid for the
 * pass and fit once at its end, by the reflow's own hold. A pane that shrinks (its neighbour opened) keeps its
 * larger size under its closing clip and takes its real size at the end too (liveReflow.ts `envelope`).
 *
 * A change of size alone never opens a pass: a window or splitter resize, a zoom, a pane resized by a
 * command. The trigger is the arrangement (which panes, in which order, which of them minimized).
 *
 * A pass never opens over a drag's reflow: a drop and a tear-off commit what the panes already show.
 * docs/specs/SPEC_PANE_REFLOW_GLIDE_2026_11_01.md §6.
 */

import { createEffect, createSignal, on, untrack, type Accessor } from "solid-js";
import { paneCaptureInFlight } from "@/app/drag/pane-capture-gate";
import { scaledMs } from "@/app/motion/motion";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { tabSwitching } from "@/app/store/tab-reveal";
import type { LayoutModel } from "./layoutModel";
import {
    liveReflowEnabled,
    movesWithReflow,
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
    /** The panes that came or were restored, by node id: each fades in at its own rect, with the glide. */
    enters: string[];
    /** False while the tiles stand at `from` (the frame the pass opens on, and any late ones after it), true from then. */
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
function mayGlide(model: LayoutModel, state: State, paneCount: number, tearOffShown = false): boolean {
    if (!liveReflowEnabled() || scaledMs(DURATION_BASE_MS) <= 0) return false;
    if (paneCount > REFLOW_MAX_PANES || paneCaptureInFlight()) return false;
    // A tab that is being switched to or made is hidden until it has settled (tab-reveal.ts): the panes that
    // open into it meanwhile are not seen arriving.
    if (tabSwitching()) return false;
    if (model.activeDrag() || (!tearOffShown && (state.dragShown || model.tearOffLayout()))) return false;
    // Magnified now, or as last published (the pane that closed was the magnified one): the tiles are hidden.
    if (model.isResizing() || model.treeState.magnifiedNodeId || model.magnifiedNodeIdAtom()) return false;
    // A window tab that is not shown keeps its layout mounted (workspace.tsx marks it, pane-overlay.ts
    // `HIDDEN_TAB_ATTR`): nothing of it is drawn, and a pass would hold the shown tab's terminals too.
    if (model.displayContainerRef.current?.closest?.("[data-window-tab-hidden]")) return false;
    return typeof document === "undefined" || document.visibilityState !== "hidden";
}

/** What changed in the arrangement between two layouts of `model`. */
interface Change {
    /** The panes that left, by node id, and those that came. */
    gone: string[];
    came: LayoutNode[];
    /** The panes minimized by this change, and those restored by it, by node id. */
    minimized: Set<string>;
    restored: Set<string>;
    /** The same panes, in another order in the tree (a swap). */
    reordered: boolean;
}

/**
 * The change from `prevLeafs` to `nextLeafs`. Which panes there were, their order and which were minimized
 * are read from what `model` last published; the new order is the tree's (`updateTree` has set it by now).
 *
 * The panes there were are the published order's ids, not `prevLeafs`' own: the tree changes its nodes in
 * place, and a leaf that a new pane is put under becomes the branch of the two and hands its id to the node
 * that now holds its pane (`addIntermediateNode`), so by now that old leaf carries another id.
 */
function changeOf(model: LayoutModel, prevLeafs: LayoutNode[], nextLeafs: LayoutNode[]): Change {
    const before = untrack(model.leafOrder) ?? [];
    const after = model.treeState.leafOrder ?? [];
    const published = before.length === prevLeafs.length;
    const prevIds = new Set(published ? before.map((entry) => entry.nodeid) : prevLeafs.map((leaf) => leaf.id));
    const nextIds = new Set(nextLeafs.map((leaf) => leaf.id));
    const wasMinimized = untrack(model.minimizedNodeIds);
    const stayed = nextLeafs.filter((leaf) => prevIds.has(leaf.id));
    const flipped = (to: boolean) =>
        new Set(stayed.filter((leaf) => !!leaf.minimized === to && wasMinimized.has(leaf.id) !== to).map((leaf) => leaf.id));
    const gone = [...prevIds].filter((id) => !nextIds.has(id));
    const came = nextLeafs.filter((leaf) => !prevIds.has(leaf.id));
    const sameSet = gone.length === 0 && came.length === 0 && before.length === after.length;
    const reordered = sameSet && before.some((entry, i) => entry.nodeid !== after[i].nodeid);
    return { gone, came, minimized: flipped(true), restored: flipped(false), reordered };
}

/** Whether `change` is one the panes glide for: not none, and not panes leaving and coming at once (a layout replaced). */
function glides(change: Change): boolean {
    if (change.gone.length > 0 && change.came.length > 0) return false;
    return change.gone.length + change.came.length + change.minimized.size + change.restored.size > 0 || change.reordered;
}

/** The glides of the panes that stayed where `change` did not touch them itself. */
function movesOf(
    open: SettlingPass | undefined,
    change: Change,
    prevProps: Props,
    nextLeafs: LayoutNode[],
    nextProps: Props
): Map<string, PaneMove> {
    const cameIds = new Set(change.came.map((leaf) => leaf.id));
    const moves = new Map<string, PaneMove>();
    for (const leaf of nextLeafs) {
        const to = nextProps[leaf.id]?.rect;
        if (!to || !movesWithReflow(leaf)) continue;
        // A pane that came, was restored or was minimized takes its rect at once: the first two fade in.
        if (cameIds.has(leaf.id) || change.restored.has(leaf.id) || change.minimized.has(leaf.id)) continue;
        const gliding = open?.moves.get(leaf.id);
        if (gliding) {
            moves.set(leaf.id, open!.gliding ? { to } : { from: gliding.from, to });
            continue;
        }
        const from = prevProps[leaf.id]?.rect;
        if (from && !sameShown(from, to)) moves.set(leaf.id, { from, to });
    }
    return moves;
}

/**
 * A tear-off a drag showed is taken off with nothing committed (Escape, a drag that ended elsewhere): the
 * panes are drawn at the rects it gave them (`shown`, by node id) and glide back to their own. A drag that is
 * still live keeps its own reflow, which does this by itself.
 */
export function settleBack(model: LayoutModel, shown: Record<string, Dimensions>): void {
    const state = states.get(model);
    if (!state || untrack(model.activeDrag)) return;
    const leafs = untrack(model.leafs);
    if (!untrack(() => mayGlide(model, state, leafs.length, true))) return;
    const props = untrack(model.additionalProps);
    const moves = new Map<string, PaneMove>();
    for (const leaf of leafs) {
        const from = shown[leaf.id];
        const to = props[leaf.id]?.rect;
        if (from && to && movesWithReflow(leaf) && !sameShown(from, to)) moves.set(leaf.id, { from, to });
    }
    if (moves.size > 0) openSettlingPass(model, { moves, ghosts: [], enters: [], gliding: false });
}

/**
 * The pass for a change of `model`'s layout from `prevLeafs` at `prevProps` to `nextLeafs` at `nextProps`, or
 * none when the change does not glide: the arrangement is the same (a resize) or was replaced, the reflow is
 * off or the animations are, a drag's reflow is on screen, the layout is being resized or magnified, its tab
 * or window is not shown or is still being revealed, or nothing a tile shows changed.
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
    const change = changeOf(model, prevLeafs, nextLeafs);
    if (!glides(change)) return undefined;
    if (!untrack(() => mayGlide(model, state, Math.max(prevLeafs.length, nextLeafs.length)))) return undefined;

    const open = untrack(state.pass);
    const moves = movesOf(open, change, prevProps, nextLeafs, nextProps);
    const ghosts = [...(open?.ghosts ?? [])];
    for (const id of change.gone) {
        const rect = prevProps[id]?.rect;
        if (!rect) continue;
        // A pane that was gliding itself is not at its layout rect yet: its ghost stands where the pane is
        // drawn, its start while its glide has not begun and its travelling edges after.
        const own = open?.moves.get(id);
        const stood = !own ? rect : own.from && !open!.gliding ? own.from : drawnRect(id, rect);
        ghosts.push({ id, rect: stood });
    }
    if (moves.size === 0 && ghosts.length === 0) return undefined;
    // A pane still fading in from the open pass goes on fading.
    const still = (open?.enters ?? []).filter((id) => nextProps[id] && !change.minimized.has(id));
    const enters = [...new Set([...still, ...change.came.map((leaf) => leaf.id), ...change.restored])];
    return { moves, ghosts, enters, gliding: false };
}

/** A frame whose callbacks run this long after the frame's own time is late: the main thread was busy. */
export const LATE_FRAME_MS = 34;
/** How long a pass stands at its start for a frame on time before it glides all the same. */
export const SEED_WAIT_MAX_MS = 400;

/** Whether a frame with the time `frameTime` (what its callbacks are given), running at `now`, is late. */
export function frameIsLate(frameTime: number | undefined, now: number): boolean {
    return typeof frameTime === "number" && now - frameTime > LATE_FRAME_MS;
}

/**
 * Run `fn` on the first frame after the one being made now that runs on time, or at once where there are no
 * frames (a test). Returns what cancels it.
 *
 * The change that opens a pass gives the panes their new sizes, and the frame being made pays for all of it:
 * the layout of every resized pane, what watches their sizes, the paint. A glide that starts in that frame
 * runs its clock through that work with nothing drawn. One frame later is not enough either: the frame after
 * a slow one runs late, with the time it was due as its own, and a transition that starts in it is given
 * that time as its start, so the next frame on time shows it most of the way there (a minimize to the
 * header on macOS: a frame of about 96 ms, then 70% of the glide in one step, #7066). So the tiles stand
 * where they were until a frame runs on time, and at most `SEED_WAIT_MAX_MS`.
 */
function whenFramesFlow(fn: () => void): () => void {
    if (typeof requestAnimationFrame !== "function") {
        fn();
        return () => {};
    }
    const since = performance.now();
    let frames = 0;
    let id = 0;
    const frame = (frameTime?: number) => {
        const now = performance.now();
        const wait = ++frames < 2 || (frameIsLate(frameTime, now) && now - since < SEED_WAIT_MAX_MS);
        if (wait) id = requestAnimationFrame(frame);
        else fn();
    };
    id = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(id);
}

/** Open `pass` on `model`'s layout, in place of one that is open. Call it in the batch that writes the new rects. */
export function openSettlingPass(model: LayoutModel, pass: SettlingPass): void {
    const state = states.get(model);
    if (!state) return;
    state.cancel?.();
    state.setPass(pass);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const cancelFrame = whenFramesFlow(() => {
        // A frame that outlived its pass (one that was replaced or ended) must not bring it back.
        if (untrack(state.pass) !== pass) return;
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
 * before the write): open a pass when the arrangement changed, or end the open one when the rects it glides
 * to are no longer the layout's (a resize in the middle of a glide).
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
