// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag. While a pane is dragged over the layout, every pane's tile, the dragged one
 * included, takes the rect the armed drop would leave it (`futureLayoutOf`): its real position and its real
 * size, so its content lays out as it will after the drop and the layout is covered edge to edge, with nothing
 * cropped, stretched or left empty. The drop then commits what is already on screen.
 *
 * What glides is a tile's four edges, each on its own (`reflowEdges`): its place and the clip it is shown
 * through are both read from them, and its size is set at once underneath. An edge two neighbours share has
 * the same target in both, so it moves as one line whatever else changes, and no gap or overlap opens while
 * they travel. A tile is laid out once per armed candidate: one that grows on an axis takes the new size at
 * once (the clip uncovers it), and one that shrinks keeps the larger size under the closing clip until the
 * glide is over (`reflowSettleMs`), then takes its real size. So a candidate that is replaced inside the
 * glide never costs the shrinking panes a layout.
 *
 * Nothing here touches the tree or the layout's own rects: they stay the drag's hit-test snapshot
 * (placementPreview.ts), and the commit on drop stays the one tree write. Terminals hold their grid until the
 * drag ends (pane-size-hold.ts). A browser pane's page is a native window that neither a transform nor a clip
 * moves; for the length of a drag it is cut away behind a picture of itself and held where it was
 * (browser-view.tsx, use-pane-rect-sync.ts), so its tile is plain DOM and moves with the rest, once every
 * page's cut has landed (page-cut-gate.ts). On unless the `layout:livereflow` setting is false.
 * docs/specs/SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6;
 * docs/reports/REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md.
 *
 * Outside a drag the same entries have a second source: a settling pass (settlingPass.ts), which glides the
 * panes to the rects a change of the tree has already given them, after a pane closed.
 */

import { createEffect, createMemo, createSignal, on, onCleanup, untrack } from "solid-js";
import { pageCutsPending } from "@/app/drag/page-cut-gate";
import { paneCaptureInFlight } from "@/app/drag/pane-capture-gate";
import { scaledMs, shouldReduceMotion } from "@/app/motion/motion";
import { DURATION_BASE_MS, DURATION_FAST_MS } from "@/app/motion/tokens";
import { notifyPaneReflow } from "@/app/platform/pane-anim";
import { holdPaneSizes } from "@/app/platform/pane-size-hold";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { getObjectValue, makeORef } from "@/app/store/mos";
import type { LayoutModel } from "./layoutModel";
import type { SettlingPass } from "./settlingPass";
import type { LayoutNode } from "./types";

/** A size in whole CSS px. */
export interface BoxSize {
    width: number;
    height: number;
}

/** How a tile takes part in the reflow while a drag is live. */
export interface ReflowEntry {
    /**
     * The rect the tile shows, by its edges in whole px (`shownPx`): where the armed drop leaves the pane, or
     * its own layout rect while no drop is armed.
     */
    rect: Dimensions;
    /**
     * The size the tile is laid out at: the target's own (`wholePx`), or larger on an axis while the glide to
     * a smaller rect is running (the clip shows `rect` either way).
     */
    box: BoxSize;
    /** The dragged pane, in its landing rect: the drop's own preview, so not blurred as a dragged tile otherwise is. */
    moved: boolean;
    /** The dragged pane while a tear-off is shown (tearOffPreview.ts): it has left the layout, so it is not shown. */
    leaving: boolean;
}

/** With more panes than this the layout does not reflow: nothing moves until the drop (§5). */
export const REFLOW_MAX_PANES = 24;
/** A frame or two after the glide, before a shrinking tile takes its real size. */
const SETTLE_MARGIN_MS = 40;
/** How long after a candidate arms a shrinking tile takes its real size, at the designed timing. */
export const REFLOW_SETTLE_MS = DURATION_BASE_MS + SETTLE_MARGIN_MS;

/**
 * The same as this window plays it: the length of the glide a tile is given (TileLayout.core.tsx: the base
 * duration, the fast one under reduced motion, both scaled by the animation speed) and the margin. A tile that
 * took its smaller size while its clip was still closing would be cut short of it, a gap beside the neighbour.
 */
export function reflowSettleMs(): number {
    return scaledMs(shouldReduceMotion("transition") ? DURATION_FAST_MS : DURATION_BASE_MS) + SETTLE_MARGIN_MS;
}

export interface LiveReflow {
    /** How the tile of `nodeId` takes part in the reflow now, or undefined when the reflow is off. */
    reflowFor: (nodeId: string) => ReflowEntry | undefined;
    /** Whether the reflow is showing a drop now: the panes are the outline, so no wireframe and no dim. */
    liveReflowActive: () => boolean;
    /**
     * Whether the reflow is how this layout shows the drops of the drag that is on: it is on, the drag is this
     * layout's own, and the panes are few enough. True between two drops too, when none is shown for a moment
     * (`liveReflowActive` is false then): the landing zone must not fall back to its other look, the layout
     * dimmed around it, for that moment.
     */
    liveReflowExpected: () => boolean;
    /**
     * The dragged pane's landing rect once every tile stands in the drop being shown: the glide to it is over,
     * and each tile is laid out at its own size there. None before that, and none with no drop shown. A new
     * object each time the panes come to rest, so it also says when they did.
     */
    landingSettled: () => Dimensions | undefined;
}

/** The block a pane shows: its active tab's, or its own. */
function shownBlockId(leaf: LayoutNode): string | undefined {
    return leaf.data?.activeBlockId ?? leaf.data?.blockId;
}

function isBrowserBlock(blockId: string | undefined): boolean {
    return !!blockId && getObjectValue<Block>(makeORef("block", blockId))?.meta?.view === "browser";
}

/**
 * Whether a glide outside a drag (settlingPass.ts) can move `leaf`'s tile: every pane but a browser pane, whose
 * page is live then, a native window that stays where the layout put it. A drag's reflow moves them all.
 */
export function movesWithReflow(leaf: LayoutNode): boolean {
    return !isBrowserBlock(shownBlockId(leaf));
}

/** Whether panes glide at all: unless the `layout:livereflow` setting is false. Reactive. */
export function liveReflowEnabled(): boolean {
    return getSettingsKeyAtom("layout:livereflow")() !== false;
}

/** `rect` in the whole px the layout's own transform uses: the corner rounded down, the size up (`setTransform`). */
export function wholePx(rect: Dimensions): Dimensions {
    return {
        left: Math.floor(rect.left),
        top: Math.floor(rect.top),
        width: Math.ceil(rect.width),
        height: Math.ceil(rect.height),
    };
}

/**
 * `rect` as a tile shows it: each edge rounded down on its own. Two panes side by side are laid out with the
 * one's far edge as the other's near edge, the same number, so rounded this way the edge they share is one
 * whole px in both, in every layout. (`wholePx` rounds a size up instead, which can put it a px past.)
 */
export function shownPx(rect: Dimensions): Dimensions {
    const left = Math.floor(rect.left);
    const top = Math.floor(rect.top);
    return { left, top, width: Math.floor(rect.left + rect.width) - left, height: Math.floor(rect.top + rect.height) - top };
}

/**
 * The four edges of a shown rect as the properties a tile's place and clip are read from (`REFLOW_TRANSFORM`,
 * `REFLOW_CLIP`; registered as lengths in tilelayout.scss). The glide is a transition on each edge, not on
 * the place and the size: a transition starts again only for a property whose target changed, so when a new
 * candidate moved a pane's place and not its size, the place started again and the size carried on, and its
 * far edge left its neighbour's (measured on macOS: up to 58 px for 275 ms). A shared edge has one target.
 */
export function reflowEdges(rect: Dimensions): Record<string, string> {
    return {
        "--reflow-l": `${rect.left}px`,
        "--reflow-t": `${rect.top}px`,
        "--reflow-r": `${rect.left + rect.width}px`,
        "--reflow-b": `${rect.top + rect.height}px`,
    };
}

/** The properties of `reflowEdges`, for a transition. */
export const REFLOW_EDGES = ["--reflow-l", "--reflow-t", "--reflow-r", "--reflow-b"] as const;
/** A reflowing tile's place: its near edges. */
export const REFLOW_TRANSFORM = "translate3d(var(--reflow-l), var(--reflow-t), 0)";
/**
 * The clip a reflowing tile is shown through: from its corner to its far edges, whatever size it is laid out
 * at (the insets are measured from the laid-out far edges, `100% - shown size`).
 */
export const REFLOW_CLIP =
    "inset(0px calc(100% - (var(--reflow-r) - var(--reflow-l))) calc(100% - (var(--reflow-b) - var(--reflow-t))) 0px)";

/**
 * The size a tile is laid out at for a new target: the larger, on each axis, of the size it is laid out at now
 * and the size it gets. Growth is laid out at once, to be uncovered; a shrink waits under the clip.
 */
export function envelope(laidOut: BoxSize, target: BoxSize): BoxSize {
    return { width: Math.max(laidOut.width, target.width), height: Math.max(laidOut.height, target.height) };
}

/** The landing rect of the dragged pane: where the armed drop leaves it, or none while no drop is armed. */
export function landingRectOf(model: Pick<LayoutModel, "futureLayout">): Dimensions | undefined {
    const future = model.futureLayout();
    return future?.rects[future.movedId];
}

interface Targets {
    /** Per tile: the rect it shows (`shownPx`), the size it is laid out at for it (`wholePx`), whether it is the dragged one. */
    byId: Map<string, { rect: Dimensions; size: BoxSize; moved: boolean; leaving: boolean }>;
    /** Whether a drop is armed, or a tear-off shown: the tiles show it, not their own layout. */
    showing: boolean;
}

/**
 * Whether two sets of targets send every tile to the same place. Each sample of a drag lays the tree out
 * again, so the targets are made again many times a second, and a pointer held on one drop makes the same
 * ones each time: they must count as one, or the settle below would start again with every sample and a
 * tile that shrinks would never take its real size.
 */
function sameTargets(a: Targets | undefined, b: Targets | undefined): boolean {
    if (!a || !b) return a === b;
    if (a.showing !== b.showing || a.byId.size !== b.byId.size) return false;
    for (const [id, x] of a.byId) {
        const y = b.byId.get(id);
        if (!y || x.moved !== y.moved || x.leaving !== y.leaving) return false;
        const [p, q] = [x.rect, y.rect];
        if (p.left !== q.left || p.top !== q.top || p.width !== q.width || p.height !== q.height) return false;
        if (x.size.width !== y.size.width || x.size.height !== y.size.height) return false;
    }
    return true;
}

/**
 * The targets of a settling pass: each pane that changed place stands at the rect it showed for the frame the
 * pass opens on, so its glide has a start, and goes to its own rect from the next. A pane that was already
 * gliding has no start of its own: it goes on from where it is drawn. Nothing is shown that the layout does
 * not already hold, so `showing` stays false.
 */
function settlingTargets(pass: SettlingPass | undefined): Targets | undefined {
    if (!pass) return undefined;
    const byId: Targets["byId"] = new Map();
    for (const [id, move] of pass.moves) {
        const at = pass.gliding || !move.from ? move.to : move.from;
        const { width, height } = wholePx(at);
        byId.set(id, { rect: shownPx(at), size: { width, height }, moved: false, leaving: false });
    }
    return { byId, showing: false };
}

/** Run `fn` on the next frame, or at once where there are no frames (a test). Returns what cancels it. */
export function onNextFrame(fn: () => void): () => void {
    if (typeof requestAnimationFrame !== "function") {
        fn();
        return () => {};
    }
    const id = requestAnimationFrame(fn);
    return () => cancelAnimationFrame(id);
}

/**
 * The targets whose glide is over: every tile then has exactly its rect's size. Timed from the frame the
 * glide starts on, not from the arming: that frame lays the panes out and can be a long time coming.
 */
function settledOf(targets: () => Targets | undefined): () => Targets | undefined {
    const [settled, setSettled] = createSignal<Targets | undefined>(undefined);
    createEffect(
        on(targets, (t) => {
            if (!t) return;
            let timer: ReturnType<typeof setTimeout> | undefined;
            const cancelFrame = onNextFrame(() => {
                timer = setTimeout(() => setSettled(t), reflowSettleMs());
            });
            onCleanup(() => {
                cancelFrame();
                clearTimeout(timer);
            });
        })
    );
    return settled;
}

/** The reflow entries for a drag of `model`, kept in the model's reactive owner. */
export function createLiveReflow(model: LayoutModel): LiveReflow {
    // Where each tile goes. While a drag is live every pane the reflow can move has a target, even when no
    // candidate is armed (then it is its own layout rect), so a tile's transition stays on from the drag's
    // start to its end and only the end takes it off, in the same batch as the drop's real rects. No drop is
    // shown while the dragged pane's picture is still being taken (a capture of the window, which must show
    // the pane as it stood, pane-capture-gate.ts), nor while a browser page is still on screen at the drag's
    // start (page-cut-gate.ts: its tile would leave it behind), but each tile has its entry at rest by then:
    // the first candidate's clip glides from the resting one, where from no clip at all it would jump.
    // A tear-off is shown through the release, until the tree lets the pane go, so the drag being over does
    // not end it (tearOffPreview.ts).
    const targets = createMemo((): Targets | undefined => {
        if (!liveReflowEnabled()) return undefined;
        const tearOff = model.tearOffLayout();
        if (!model.activeDrag() && !tearOff) return settlingTargets(model.settlingPass());
        const leafs = model.leafs();
        if (leafs.length > REFLOW_MAX_PANES) return undefined;
        // A tear-off shown after the release has no page to wait for: the drag's cuts are over.
        const waiting = paneCaptureInFlight() || (model.activeDrag() && pageCutsPending());
        const future = waiting ? undefined : (model.futureLayout() ?? tearOff);
        const current = model.additionalProps();
        const futureRectOfBlock = new Map<string, Dimensions>();
        for (const id of future?.leafIds ?? []) {
            const blockId = future!.leafBlockIds[id];
            if (blockId && future!.rects[id]) futureRectOfBlock.set(blockId, future!.rects[id]);
        }
        const byId: Targets["byId"] = new Map();
        for (const leaf of leafs) {
            const from = current[leaf.id]?.rect;
            if (!from) continue;
            // The dragged pane keeps its node id through the preview: its future rect is the landing rect.
            const moved = !!future && leaf.id === future.movedId;
            const to = moved ? future!.rects[leaf.id] : leaf.data?.blockId ? futureRectOfBlock.get(leaf.data.blockId) : undefined;
            const { width, height } = wholePx(to ?? from);
            const leaving = moved && !!future!.leaves;
            byId.set(leaf.id, { rect: shownPx(to ?? from), size: { width, height }, moved: moved && !!to, leaving });
        }
        return { byId, showing: !!future && byId.size > 0 };
    }, undefined, { equals: sameTargets });

    // A browser page is put where its pane's placeholder is, and nothing tells it when only a transform
    // moved that: each change of what the tiles show has the pages look again (pane-anim.ts). A page held for
    // the drag ignores it; one whose tile a tear-off still shows after the release follows.
    createEffect(on(targets, () => notifyPaneReflow(), { defer: true }));

    // Terminals keep their grid for as long as tiles can take a size the drop may never commit. On a boolean,
    // not on the targets themselves: those are new for every candidate, and letting go and holding again
    // between two candidates would fit every terminal once per candidate.
    const reflowing = createMemo(() => !!targets());
    createEffect(() => {
        if (!reflowing()) return;
        holdPaneSizes(model, true);
        onCleanup(() => holdPaneSizes(model, false));
    });

    const settled = settledOf(targets);

    const entries = createMemo<Map<string, ReflowEntry>>((before) => {
        const t = targets();
        const out = new Map<string, ReflowEntry>();
        if (!t) return out;
        const done = settled() === t;
        // Not tracked: a rect that changes under the drag changes the targets too.
        const current = untrack(() => model.additionalProps());
        for (const [id, target] of t.byId) {
            // What the tile is laid out at now: its box from the previous target, or its own layout size.
            const laidOut = before.get(id)?.box ?? wholePx(current[id].rect);
            const box = done ? target.size : envelope(laidOut, target.size);
            out.set(id, { rect: target.rect, box, moved: target.moved, leaving: target.leaving });
        }
        return out;
    }, new Map());

    const landingSettled = createMemo(() => {
        const t = targets();
        if (!t?.showing || settled() !== t) return undefined;
        for (const target of t.byId.values()) if (target.moved) return { ...target.rect };
        return undefined;
    });

    return {
        reflowFor: (nodeId) => entries().get(nodeId),
        liveReflowActive: () => !!targets()?.showing,
        liveReflowExpected: () => liveReflowEnabled() && model.activeDrag() && model.leafs().length <= REFLOW_MAX_PANES,
        landingSettled,
    };
}
