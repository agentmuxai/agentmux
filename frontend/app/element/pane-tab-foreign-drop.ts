// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane's tab strip does when a tab is dragged over it from another pane (or from its own, to
 * gather several): which element takes the drop, the hover feedback, the placement ghost over the
 * body, and the move one task after the drop. SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.4.
 */

import { createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { paneTabItemType } from "@/app/drag/drag-types";
import { clearPillGhost, showPillGhost, type PillZone } from "@/layout/lib/paneBodyDrop";
import { pillPlacingIn } from "@/layout/lib/pillDragState";
import { beginPillPlacement, dropPill, followPill, panePillZone, registerPillPane, type PillRelease } from "@/layout/lib/pillPlacement";
import type { DropDirection } from "@/layout/lib/types";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { edgeFlashArea } from "./drop-blink-clock";
import { markLanded } from "./pane-tab-landing";
import { createForeignHover } from "./pane-tab-foreign-hover";

/** Placing a single pill dropped on a pane's body as a pane of its own (`PaneTabStripProps.bodyDrop`). */
export interface PaneTabBodyDrop {
    model: LayoutModel;
    /** False for a header-only pane (minimized, collapsed): no body zone. */
    hasBody: () => boolean;
    place: (blockId: string, zone: Extract<PillZone, { kind: "place" }>) => string | false;
}

/** What the drop zone reads of a strip's props. */
export interface ForeignTabDropProps<T> {
    tabs: T[];
    getId: (tab: T) => string;
    paneKey?: string;
    onReceiveForeignTab?: (blockId: string, opts?: { keepSeparate?: boolean }) => boolean | void;
    onReceiveForeignTabs?: (
        blockIds: readonly string[],
        grabbed: string,
        opts?: { keepSeparate?: boolean }
    ) => readonly string[] | false;
    acceptsForeignTabs?: (blockIds: readonly string[]) => boolean;
    bodyDrop?: PaneTabBodyDrop;
}

/**
 * The ghost pill's text for a pane-tab drag over a strip that holds `here`: the grabbed tab's title, or the
 * count of the picked tabs this strip lacks. None when nothing would be added (the drag is not a pane tab, or
 * every tab is already here).
 */
export function incomingTabLabel(data: Record<string | symbol, unknown>, here: ReadonlySet<string>): string | null {
    if (data.type !== paneTabItemType) return null;
    const many = multiIdsOf(data);
    if (many) {
        const added = many.filter((blockId) => !here.has(blockId)).length;
        return added === 0 ? null : added === 1 ? "1 tab" : `${added} tabs`;
    }
    const blockId = data.blockId;
    if (typeof blockId !== "string" || here.has(blockId)) return null;
    return typeof data.label === "string" && data.label ? data.label : "Tab";
}

/** The block ids a pane-tab drag carries when it carries several (a Ctrl/Cmd+click selection), else undefined. */
export function multiIdsOf(data: Record<string | symbol, unknown>): readonly string[] | undefined {
    const ids = data.blockIds;
    return Array.isArray(ids) && ids.length > 1 && ids.every((x) => typeof x === "string") ? (ids as string[]) : undefined;
}

/** Selector for the Pane header row that a header-hosted strip lives in —
 *  `BlockFrame_Header`'s own `data-role`, which is also what TileLayout
 *  already uses as the whole-pane drag handle. Matched by attribute rather
 *  than by class (`.block-frame-default-header`) deliberately: `data-role`
 *  is the stable contract blockframe.tsx exposes for exactly this kind of
 *  outside-in lookup, while the class name is styling. */
const PANE_HEADER_SELECTOR = '[data-role="block-header"]';

/** Selector for a whole Pane — PaneChrome's root (`.pane-stack`), header
 *  and body together. Matched by `data-role` for the same reason as the
 *  header selector above: it's the stable contract, the class is styling. */
const PANE_SELECTOR = '[data-role="pane"]';

/**
 * Which element is the cross-pane drop zone for a strip: the WHOLE Pane
 * (PaneChrome's root, header and body together) when the strip is that
 * pane's header strip, else the header row (a header outside PaneChrome),
 * else the strip box itself (a strip that isn't in a header at all).
 *
 * Dropping a Pane Tab anywhere on another pane, header or body, does the same
 * thing: the tab joins that pane (repo owner, 2026-09-24,
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.3). It was the header row alone
 * before, and before that just the pills: aiming at a thin strip to move a tab
 * there is fussy, and the whole pane is what reads as "into this pane" mid-drag.
 * Widening it is free: every drop target inside the pane either accepts the
 * drag itself (the per-pill same-pane reorder targets) or declines and lets it
 * bubble here (pragmatic-dnd walks up the DOM on a false `canDrop`). Nothing
 * else inside a pane registers an element drop target, and the tile overlay
 * that sits above pane bodies only takes pointer events during a whole-PANE
 * drag (`layoutModel.activeDrag`), never during a pane-TAB drag.
 *
 * Pure and exported for the same reason as `dropPositionForPointerX` in PaneTabStrip.tsx:
 * the gesture itself can't be unit-tested (no real drag pipeline in jsdom),
 * but *which element gets the registration* can — and getting that wrong is
 * silent, since pragmatic-dnd simply never fires a handler for an element
 * the pointer never reaches (cf. ReAgent's P0 on PR #3447, a cross-pane
 * move that was dead in the UI while its own unit tests passed).
 */
export function foreignDropRootFor(strip: HTMLElement): HTMLElement {
    // Widen to the pane only from a strip that IS the pane's header strip. A
    // strip rendered inside a pane's content (editor file tabs, the agent
    // History strip) must never resolve to the pane around it.
    const header = strip.closest<HTMLElement>(PANE_HEADER_SELECTOR);
    if (!header) return strip;
    return header.closest<HTMLElement>(PANE_SELECTOR) ?? header;
}

/**
 * Which element shows the "valid drop target" flash while a foreign tab
 * hovers the drop zone: the header row, wherever over the pane the pointer
 * is. The header is where the tab will appear, and the flash is the Window
 * Tab look, which is sized for a strip, not a whole pane body
 * (SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §3).
 * Falls back to the strip box for a strip not hosted in a header.
 */
export function foreignDropHighlightFor(strip: HTMLElement): HTMLElement {
    return strip.closest<HTMLElement>(PANE_HEADER_SELECTOR) ?? strip;
}

type PillArgs = {
    source: { data: Record<string | symbol, unknown> };
    location: { current: { input: { clientX: number; clientY: number } } };
};

type HoverControl = { hovering: () => boolean; begin: (source: PillArgs["source"]) => void; end: () => void };

/**
 * A single pill over this pane, with `bodyDrop`: what the pointer is over decides the feedback (the
 * header glow, or the placement ghost), read again as it moves. A tab of this layout over a placing zone is
 * handed to the layout, which shows the drop by reflowing the panes and reads the pointer itself from then on
 * (pillPlacement.ts); the ghost here is left for a tab from another window tab.
 */
function createPillPlacement(opts: { body: PaneTabBodyDrop | undefined; paneKey: string; headerEl: HTMLElement; hover: HoverControl }) {
    const { body, paneKey, headerEl, hover } = opts;
    let zone: PillZone = { kind: "none" };
    let ghost: DropDirection | undefined;
    let lastFollow = -Infinity;
    const singlePill = (args?: PillArgs) => !!body && !!args?.source && !multiIdsOf(args.source.data);
    const zoneInput = (args: PillArgs) => {
        const { clientX, clientY } = args.location.current.input;
        return {
            model: body!.model,
            nodeId: paneKey,
            blockId: args.source.data.blockId as string,
            pointer: { x: clientX, y: clientY },
            headerRect: headerEl.getBoundingClientRect(),
            ownPane: args.source.data.sourceNodeId === paneKey,
            hasBody: body!.hasBody(),
        };
    };
    const follow = (args: PillArgs, force = false) => {
        const now = performance.now();
        if (!force && now - lastFollow < 30) return;
        lastFollow = now;
        const input = zoneInput(args);
        // The layout has the drag, and this pane still gets its events until the browser has sent the layout's
        // catcher one: they are the layout's samples. The pane reads again once the layout gives the drag back.
        if (pillPlacingIn(input.model)) {
            followPill(input.model, input.pointer);
            if (pillPlacingIn(input.model)) return;
        }
        zone = panePillZone(input);
        // The header glows where the pill would join, and nowhere else.
        if (zone.kind === "join") {
            if (!hover.hovering()) hover.begin(args.source);
        } else if (hover.hovering()) {
            hover.end();
        }
        if (zone.kind === "place") {
            if (beginPillPlacement(input.model, input.blockId, input.pointer)) return;
            body!.model.showOverlay._set(true);
            if (zone.direction !== ghost) {
                ghost = zone.direction;
                showPillGhost(body!.model, input, zone);
            }
        } else if (ghost !== undefined) {
            ghost = undefined;
            clearPillGhost(body!.model);
        }
    };
    const end = () => {
        zone = { kind: "none" };
        if (!body) return;
        if (ghost !== undefined) clearPillGhost(body.model);
        ghost = undefined;
        // The layout's own while it places the pill: leaving this pane is how the pointer got to its catcher.
        if (!pillPlacingIn(body.model)) body.model.showOverlay._set(false);
    };
    /**
     * The release came to this pane while the layout has the drag (a quick drag: the layout's catcher has had no
     * event yet). The layout takes it; false when it places nothing there and the release is this pane's.
     */
    const dropInLayout = (args: PillArgs): boolean => {
        if (!singlePill(args) || !pillPlacingIn(body!.model)) return false;
        const { clientX, clientY } = args.location.current.input;
        return dropPill(body!.model, { x: clientX, y: clientY });
    };
    /** Where the pointer was released, not where the throttled drag last looked. */
    const zoneAtDrop = (args: PillArgs): PillZone => {
        if (!singlePill(args)) return zone;
        return panePillZone(zoneInput(args));
    };
    return { singlePill, follow, end, zoneAtDrop, dropInLayout };
}

type Placement = ReturnType<typeof createPillPlacement>;

/** Place the pill beside a pane and mark where it landed, for the bounce. */
const placeAndMark = (body: PaneTabBodyDrop) => (blockId: string, zone: Extract<PillZone, { kind: "place" }>): string | false => {
    const landedIn = body.place(blockId, zone);
    if (landedIn !== false) markLanded(blockId, landedIn);
    return landedIn;
};

/** Whether this pane takes the dragged tab(s). */
function canDropForeign<T>(props: ForeignTabDropProps<T>, paneKey: string, data: Record<string | symbol, unknown>): boolean {
    if (data.type !== paneTabItemType) return false;
    // Several tabs: accepted when this pane lacks at least one of them, even
    // the pane the grabbed tab came from (gathering into where you started).
    const many = multiIdsOf(data);
    if (many && props.acceptsForeignTabs) return props.acceptsForeignTabs(many);
    // A single pill from this very pane can only be placed (it is already a tab here),
    // which needs another tab to stay behind.
    if (props.bodyDrop && !many && data.sourceNodeId === paneKey) return props.tabs.length >= 2;
    return data.sourceNodeId !== paneKey;
}

/**
 * What a drop does. The move runs on the next task: it unmounts the dragged pill, the live drag
 * source, and unmounting that mid-dispatch is the teardown hazard of
 * SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md. The hover styling clears at once.
 * SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.2.
 */
function dropForeign<T>(props: ForeignTabDropProps<T>, paneKey: string, placement: Placement, endHover: () => void, args: any): void {
    const { source } = args;
    endHover();
    const dropZone = placement.dropInLayout(args) ? undefined : placement.zoneAtDrop(args);
    placement.end();
    if (!dropZone) return;
    const blockId = source.data.blockId as string | undefined;
    const body = props.bodyDrop;
    if (body && blockId && !multiIdsOf(source.data)) {
        // Over this pane's header, or the pill's own pane where it cannot be placed.
        if (dropZone.kind === "none") return;
        if (dropZone.kind === "place") {
            setCurrentDragPayload(null);
            // Next task, like a join: the move unmounts the dragged pill.
            setTimeout(() => placeAndMark(body)(blockId, dropZone), 0);
            return;
        }
    }
    // Handled in-window: the cross-window monitor must not also
    // tear this tab off on dragend (pane-tab-tearoff.ts).
    setCurrentDragPayload(null);
    if (!blockId) return;
    const many = multiIdsOf(source.data);
    // Alt held on the drop keeps two document panes as two pane tabs (the layout merges them
    // otherwise). Passed only when held, so a plain drop is the call it always was.
    const keepSeparate = args.location?.current?.input?.altKey === true;
    if (many && props.onReceiveForeignTabs) {
        const receiveMany = props.onReceiveForeignTabs;
        setTimeout(() => {
            const moved = keepSeparate ? receiveMany(many, blockId, { keepSeparate }) : receiveMany(many, blockId);
            if (moved !== false) markLanded(moved, paneKey);
        }, 0);
        return;
    }
    const receive = props.onReceiveForeignTab!;
    setTimeout(() => {
        const moved = keepSeparate ? receive(blockId, { keepSeparate }) : receive(blockId);
        if (moved !== false) markLanded(blockId, paneKey);
    }, 0);
}

/**
 * Registers the strip as a drop zone for tabs from another pane, when it has `onReceiveForeignTab`
 * and a `paneKey`. Returns the hover state the strip draws: whether a tab is over it, how its flash
 * runs, the ghost pill's text, and whether the strip box itself (rather than the header row) lights up.
 */
export function useForeignTabDrop<T>(props: ForeignTabDropProps<T>, stripEl: () => HTMLDivElement | undefined) {
    // Cross-pane drop-to-append (Phase 4, §3.4): a dwell-free hover flash, since the target pane's
    // content is already visible and there is nothing to reveal before committing.
    // The element that flashes, to size the hover against the flash rule (drop-blink-clock.ts).
    let flashEl: HTMLElement | undefined;
    const flashArea = (): number | undefined => {
        const r = flashEl?.getBoundingClientRect();
        if (!r) return undefined;
        // A pane header flashes its two edge strips only; a strip used as its own drop zone, all of it.
        return flashEl?.matches(PANE_HEADER_SELECTOR) ? edgeFlashArea(r.width, r.height) : r.width * r.height;
    };
    const {
        foreignHover,
        flash,
        incoming,
        begin: beginForeignHover,
        end: endForeignHover,
        hovering,
    } = createForeignHover({
        flashArea,
        incomingOf: (data) => incomingTabLabel(data, new Set(props.tabs.map((tab) => props.getId(tab)))),
    });
    // Whether the hover highlight belongs on the strip box itself. False once the drop zone resolves
    // to the enclosing header row, so the feedback outlines exactly the area that accepts the drop.
    // It only flips during onMount, before `foreignHover` can become true.
    const [highlightStrip, setHighlightStrip] = createSignal(true);
    onMount(() => {
        const strip = stripEl();
        if (!strip || !props.onReceiveForeignTab || !props.paneKey) return;
        const paneKey = props.paneKey;
        // The whole pane, header and body — see `foreignDropRootFor`'s own doc comment for why.
        const el = foreignDropRootFor(strip);
        // The flash goes on the header either way (`foreignDropHighlightFor`).
        const highlightEl = foreignDropHighlightFor(strip);
        flashEl = highlightEl;
        if (highlightEl !== strip) {
            setHighlightStrip(false);
            // `highlightEl` is blockframe.tsx's element, outside this component's own JSX, so the
            // class goes on imperatively, and comes off on cleanup: the header can outlive this strip.
            createEffect(() => {
                highlightEl.classList.toggle("pane-header--foreign-hover", foreignHover());
                highlightEl.classList.toggle("pane-header--no-blink", foreignHover() && flash() !== "blink");
            });
            onCleanup(() => highlightEl.classList.remove("pane-header--foreign-hover", "pane-header--no-blink"));
        }
        const placement = createPillPlacement({
            body: props.bodyDrop,
            paneKey,
            headerEl: highlightEl,
            hover: { hovering, begin: beginForeignHover, end: endForeignHover },
        });
        onCleanup(placement.end);
        const body = props.bodyDrop;
        if (body) {
            // A release the layout's catcher got where the tab joins a pane: this pane's drop, under its own rule.
            const release = (drop: PillRelease) => {
                if (canDropForeign(props, paneKey, drop.source.data)) dropForeign(props, paneKey, placement, endForeignHover, drop);
            };
            onCleanup(registerPillPane(body.model, paneKey, { headerEl: highlightEl, hasBody: body.hasBody, place: placeAndMark(body), release }));
        }
        const { singlePill, follow } = placement;
        const cleanup = dropTargetForElements({
            element: el,
            canDrop: ({ source }) => canDropForeign(props, paneKey, source.data),
            // A drag that starts inside this pane (gathering tabs into the pane the grabbed one came
            // from) is over this drop zone from the first moment: pragmatic-dnd calls onDragStart for
            // that, not onDragEnter.
            onDragStart: (args) => (singlePill(args) ? follow(args, true) : beginForeignHover(args?.source)),
            onDragEnter: (args) => (singlePill(args) ? follow(args, true) : beginForeignHover(args?.source)),
            onDrag: (args) => {
                if (singlePill(args)) follow(args);
            },
            onDragLeave: () => {
                endForeignHover();
                placement.end();
            },
            onDrop: (args) => dropForeign(props, paneKey, placement, endForeignHover, args),
        });
        onCleanup(cleanup);
    });
    return { foreignHover, flash, incoming, highlightStrip };
}
