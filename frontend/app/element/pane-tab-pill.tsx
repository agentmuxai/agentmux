// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What one tab pill of the strip does: how it drags and takes drops (a pane tab, or a document tab),
 * how it reads a press and a click, how it flashes for activity, and what it says about its work.
 */

import { createMemo, onCleanup, onMount, Show, createSignal, type Accessor, type JSX } from "solid-js";
import { draggable, dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { markEscaped } from "@/app/drag/drag-session";
import { hideDragImageOnNativeWayland } from "@/app/drag/wayland-drag-image";
import {
    releaseDocTabDrag,
    releasePaneTabDrag,
    startDocTabDrag,
    startPaneTabDrag,
} from "@/app/drag/pane-tab-drag";
import { asDocTabDragData, docTabItemType, paneTabItemType, type DocTabDragData } from "@/app/drag/drag-types";
import { cancelFlashPattern, flashElement, onActivityFlash } from "@/app/notification/activity-flash";
import { LOCATE_PILL_FLASH, onPaneLocate, onPaneLocatePlay } from "@/app/util/pane-locate";
import type { PaneTabProgress } from "@/app/store/pane-tab-progress";
import { isModKey, isWindows } from "@/util/platformutil";
import { classifyPillClick, markPillDragEnded, msSinceLastPillDrag } from "./pane-tab-gesture";
import { multiIdsOf } from "./pane-tab-foreign-drop";
import { markLanded } from "./pane-tab-landing";
import type { PaneTabSelectionApi } from "./pane-tab-selection";

/**
 * Document-tab drag for a strip of a pane's documents (an Editor's files, a
 * Media pane's files): the same pills, drop marks and landing bounce as pane
 * tabs, but its own drag kind. A document tab moves only within its strip or
 * to another pane of the same `docType`, and never tears off into a window.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md section 3.1.
 */
export interface DocTabDrag {
    /** The pane type ("editor", "media"): tabs move only between panes of one type. */
    docType: string;
    /** The block whose tabs these are. */
    blockId: string;
    /** Whether a tab can be dragged at all (an empty Media tab can not). */
    canDrag?: (tabId: string) => boolean;
    /** A tab of this strip dropped before or after another of its tabs.
     *  `false`: refused (no landing bounce). */
    onReorder: (tabId: string, targetId: string, position: "before" | "after") => boolean | void;
    /** A tab of another pane of this type dropped on one of these tabs. Runs
     *  one task after the drop: the move unmounts the dragged pill, the live
     *  source of the drag. Omitted: such a drop is not accepted here. */
    onReceive?: (sourceBlockId: string, tabId: string, at: { targetId: string; position: "before" | "after" }) => boolean | void;
}

/**
 * Which side of a hovered pill's rect a dragged pill should land on, given the pointer's clientX.
 * Pure and exported so it can be unit-tested: jsdom has no real HTML5 drag pipeline, so this is the
 * one piece of the gesture's logic pure enough to verify directly.
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.2.
 */
export function dropPositionForPointerX(rect: Pick<DOMRect, "left" | "width">, clientX: number): "before" | "after" {
    return clientX < rect.left + rect.width / 2 ? "before" : "after";
}

/**
 * What a pane tab's pill carries when dragged: its block, the pane and window tab it comes from, the picked set
 * when it is dragged with several, and its title (the ghost pill's text over another pane's strip).
 */
export function paneTabDragData(blockId: string, label: string, paneKey?: string, sourceTabId?: string, many?: readonly string[]) {
    return {
        kind: "pane-tab",
        blockId,
        label,
        type: paneTabItemType,
        sourceNodeId: paneKey,
        sourceTabId,
        ...(many && many.length > 1 ? { blockIds: many } : {}),
    };
}

/** How one pill drags, for either kind of tab: what it carries, what it
 *  accepts, and what a drop on it does. One registration serves both. */
interface PillDrag {
    data: () => Record<string, unknown>;
    canDrag: () => boolean;
    start: () => void;
    release: () => void;
    accepts: (data: Record<string | symbol, unknown>) => boolean;
    /** Act on a drop at `position`; returns the id of the tab to bounce, or null. */
    drop: (data: Record<string | symbol, unknown>, position: "before" | "after") => string | null;
}

/** What a pill's drag reads of the strip's props. */
export interface PillDragProps<T> {
    tab: T;
    getLabel: (tab: T) => string;
    onReorder?: (blockId: string, targetId: string, position: "before" | "after") => boolean | void;
    paneKey?: string;
    sourceTabId?: string;
    selection?: PaneTabSelectionApi;
    docDrag?: DocTabDrag;
}

type TearOff = { start: () => void; stop: () => void };

/** The cross-window payload and the drag-image hold while a pane tab is dragged, so it can tear off into a floating pane. */
function createTearOff<T>(props: PillDragProps<T>, id: () => string, pill: () => HTMLDivElement | undefined): TearOff {
    // Tear-off to a floating pane (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
    // §3.5). On drag start, hand the cross-window monitors a "pane-tab"
    // payload; they tear the tab off on dragend only if no drop target inside
    // the window claimed it (both pill drop targets clear it) and it landed on
    // no AgentMux window. The payload is deliberately NOT cleared in the
    // draggable's own onDrop: dragend, which the monitors read it on, comes
    // after — same as whole-pane (tile) drags.
    const onEscape = (e: KeyboardEvent) => {
        // keydown still reaches the page during an HTML5 drag; an escaped
        // drag never tears off (the monitors read it off the session).
        if (e.key === "Escape") markEscaped();
    };
    let tracking = false;
    const startTearOffTracking = () => {
        if (!props.paneKey || !props.sourceTabId || !pill()) return;
        const paneRect = pill()!.closest<HTMLElement>('[data-role="pane"]')?.getBoundingClientRect();
        setCurrentDragPayload({
            kind: "pane-tab",
            blockId: id(),
            sourceNodeId: props.paneKey,
            sourceTabId: props.sourceTabId,
            paneSize: paneRect ? { width: paneRect.width, height: paneRect.height } : undefined,
        });
        // macOS/Linux: without this, a drop outside every drop target (i.e. a
        // tear-off) animates the drag image back into the window first, the
        // same snapback whole-pane drags suppress (TileLayout.darwin.tsx).
        if (!isWindows()) preventUnhandled.start();
        window.addEventListener("keydown", onEscape, true);
        tracking = true;
    };
    const stopTearOffTracking = () => {
        if (!tracking) return;
        tracking = false;
        if (!isWindows()) preventUnhandled.stop();
        window.removeEventListener("keydown", onEscape, true);
    };
    return { start: startTearOffTracking, stop: stopTearOffTracking };
}

// A pane tab: same-pane reorder (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md sections 3.1 and 3.2),
// gated on `onReorder`, with tear-off when the strip names its window tab (3.5). A pill from a
// DIFFERENT pane (another `sourceNodeId`) is rejected here rather than accepted and then silently
// ignored on drop: cross-pane is the whole-pane target's job (#3444).
function paneTabPillDrag<T>(props: PillDragProps<T>, id: () => string, tearOff: TearOff): PillDrag | null {
    const startTearOffTracking = tearOff.start;
    const stopTearOffTracking = tearOff.stop;
    const onReorder = props.onReorder;
    if (!onReorder) return null;
    return {
        data: () =>
            paneTabDragData(id(), props.getLabel(props.tab), props.paneKey, props.sourceTabId, props.selection?.dragIds(id())),
        canDrag: () => true,
        start: () => {
            const many = props.selection?.dragIds(id());
            const several = !!many && many.length > 1;
            startPaneTabDrag(id(), props.paneKey, props.sourceTabId, several ? many : undefined);
            // Dropping several tabs outside every window does nothing (no partial tear-off):
            // without the tear-off payload the monitors treat the release as a cancel.
            if (!several) startTearOffTracking();
        },
        release: () => {
            releasePaneTabDrag();
            stopTearOffTracking();
        },
        // A multi-tab drag is not a reorder: the pane's own drop zone takes it.
        accepts: (data) =>
            data.type === paneTabItemType &&
            data.blockId !== id() &&
            data.sourceNodeId === props.paneKey &&
            !multiIdsOf(data),
        drop: (data, position) => {
            // Handled in-window: never also a tear-off.
            setCurrentDragPayload(null);
            const blockId = data.blockId as string | undefined;
            if (!blockId) return null;
            return onReorder(blockId, id(), position) !== false ? blockId : null;
        },
    };
}

// A document tab (`docDrag`): reorder within the strip, or a tab of
// another pane of the same type dropped onto this one. Never a tear-off.
function docTabPillDrag<T>(props: PillDragProps<T>, id: () => string): PillDrag | null {
    const doc = props.docDrag;
    if (!doc) return null;
    return {
        data: () => ({ type: docTabItemType, tabId: id(), docType: doc.docType, sourceBlockId: doc.blockId }) satisfies DocTabDragData,
        canDrag: () => doc.canDrag?.(id()) ?? true,
        start: () => startDocTabDrag(doc.blockId, id()),
        release: releaseDocTabDrag,
        accepts: (raw) => {
            const data = asDocTabDragData(raw);
            if (!data || data.docType !== doc.docType) return false;
            return data.sourceBlockId === doc.blockId ? data.tabId !== id() : !!doc.onReceive;
        },
        drop: (raw, position) => {
            const data = asDocTabDragData(raw);
            if (!data) return null;
            if (data.sourceBlockId === doc.blockId) {
                return doc.onReorder(data.tabId, id(), position) !== false ? data.tabId : null;
            }
            // From another pane: commit on the next task, after
            // pragmatic-dnd has finished dispatching this drop, because
            // the move unmounts the dragged pill, the live source of the
            // drag (SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md 4.2).
            const receive = doc.onReceive!;
            const at = { targetId: id(), position };
            setTimeout(() => {
                if (receive(data.sourceBlockId, data.tabId, at) !== false) markLanded(data.tabId, doc.blockId);
            }, 0);
            return null;
        },
    };
}

/**
 * A pill is both a drag source (itself) and a drop target (for another pill dragged onto it), the
 * same dual role droppable-tab.tsx's tab buttons have in the window tab bar. A strip with neither
 * `onReorder` nor `docDrag` registers nothing. Returns the side a dragged pill would land on.
 */
export function usePillDrag<T>(opts: {
    props: PillDragProps<T>;
    id: () => string;
    landingKey: () => string | undefined;
    pill: () => HTMLDivElement | undefined;
    onDragStarted: () => void;
}): Accessor<"before" | "after" | null> {
    const { props, id, landingKey, pill, onDragStarted } = opts;
    const [dropSide, setDropSide] = createSignal<"before" | "after" | null>(null);
    const tearOff = createTearOff(props, id, pill);
    onCleanup(tearOff.stop);
    const paneTabDrag = () => paneTabPillDrag(props, id, tearOff);
    const docTabDrag = () => docTabPillDrag(props, id);
    onMount(() => {
        const drag = docTabDrag() ?? paneTabDrag();
        const el = pill();
        if (!drag || !el) return;
        const cleanupDraggable = draggable({
            element: el,
            canDrag: () => drag.canDrag(),
            getInitialData: () => drag.data(),
            // A release outside every window (a tear-off) would slide the
            // picture back on native Wayland (wayland-drag-image.ts).
            onGenerateDragPreview: ({ nativeSetDragImage }) => {
                hideDragImageOnNativeWayland(nativeSetDragImage);
            },
            onDragStart: () => {
                onDragStarted();
                drag.start();
            },
            onDrop: () => {
                markPillDragEnded();
                drag.release();
            },
        });
        const cleanupDropTarget = dropTargetForElements({
            element: el,
            canDrop: ({ source }) => drag.accepts(source.data),
            onDrag: ({ location }) => {
                setDropSide(dropPositionForPointerX(el.getBoundingClientRect(), location.current.input.clientX));
            },
            onDragLeave: () => setDropSide(null),
            onDrop: ({ source, location }) => {
                setDropSide(null);
                // Computed fresh here, not reused from the `onDrag`-updated
                // signal above: that value may not be current at the drop.
                const position = dropPositionForPointerX(el.getBoundingClientRect(), location.current.input.clientX);
                // The moved pill (not the one it was dropped on) bounces,
                // same as a reordered Window Tab.
                const landed = drag.drop(source.data, position);
                if (landed) markLanded(landed, landingKey());
            },
        });
        onCleanup(() => {
            cleanupDraggable();
            cleanupDropTarget();
        });
    });
    return dropSide;
}

/** The pill pulses with the activity flash of its block, and flashes when its pane is located. */
export function usePillFlash(opts: {
    enabled: () => boolean | undefined;
    id: () => string;
    paneKey: () => string | undefined;
    pill: () => HTMLDivElement | undefined;
    windowDisplayed: Accessor<boolean>;
}): void {
    const { id, pill, windowDisplayed } = opts;
    onMount(() => {
        if (!opts.enabled()) return;
        const unsubscribe = onActivityFlash((target) => {
            if (target.blockId === id() && pill() && windowDisplayed()) flashElement(pill()!, target);
        });
        // The pane is flashing to show it was just selected: its pill does too.
        const unsubscribeLocate = onPaneLocatePlay((blockId, paneKey) => {
            if (blockId === id() && paneKey === opts.paneKey() && pill()) flashElement(pill()!, LOCATE_PILL_FLASH);
        });
        // A reveal of another block replaces the earlier one's pill, so two never show at once. A repeat
        // request for this block is the same flash (the pane ignores it while it plays): its pill stays lit.
        const unsubscribeRequest = onPaneLocate((blockId) => {
            if (blockId !== id() && pill()) cancelFlashPattern(pill()!, LOCATE_PILL_FLASH.pattern);
        });
        onCleanup(() => {
            unsubscribe();
            unsubscribeLocate();
            unsubscribeRequest();
        });
    });
}

/**
 * What the pointer did since it went down on this pill: where, and whether a drag started
 * (pane-tab-gesture.ts). A drag or a slip past the drag threshold is not a click.
 */
export function usePillGestures<T>(props: PillGestureProps<T>, id: () => string) {
    let pressedAt: { x: number; y: number } | null = null;
    let dragStartedSincePress = false;
    const onMouseDown = (e: MouseEvent) => {
        if (e.button === 0) {
            pressedAt = { x: e.clientX, y: e.clientY };
            dragStartedSincePress = false;
        }
        // Middle-click → close (matches VS Code / Chrome convention).
        if (e.button === 1) {
            e.preventDefault();
            props.onClose?.(id());
        }
    };

    const onClick = (e: MouseEvent) => {
        const intent = classifyPillClick({
            button: e.button,
            mod: isModKey(e),
            shift: e.shiftKey,
            alt: e.altKey,
            pressedAt,
            clickedAt: { x: e.clientX, y: e.clientY },
            dragStartedSincePress,
            msSinceLastPillDrag: msSinceLastPillDrag(),
        });
        pressedAt = null;
        dragStartedSincePress = false;
        // Another button (a middle-click closes in onMouseDown), or the end of a drag: nothing.
        if (intent === "ignore") return;
        // Ctrl (Cmd on macOS) + click picks or unpicks this tab for a multi-tab drag. It
        // does not activate, focus or expand anything, and the click goes no further, so
        // the pane's own click handler (focus) does not run either.
        if (intent === "toggle-selection" && props.selection) {
            e.stopPropagation();
            props.selection.toggle(id());
            return;
        }
        props.selection?.clear();
        // Only a plain click expands a collapsed pane: a modifier makes it a selection gesture.
        if (intent === "expand") props.onSelect?.(id());
        if (!props.active) props.onActivate(id());
    };

    const onDblClick = (e: MouseEvent) => {
        e.stopPropagation();
        // Two quick Ctrl/Cmd+clicks toggle the pick twice; they are not a rename.
        if (props.selection && isModKey(e)) return;
        props.onDoubleClick?.(props.tab);
    };
    return { onMouseDown, onClick, onDblClick, noteDragStarted: () => { dragStartedSincePress = true; } };
}

/** What the pill's gestures read of the strip's props. */
export interface PillGestureProps<T> {
    tab: T;
    active: boolean;
    selection?: PaneTabSelectionApi;
    onClose?: (id: string) => void;
    onSelect?: (id: string) => void;
    onActivate: (id: string) => void;
    onDoubleClick?: (tab: T) => void;
}

/**
 * The pill's progress state: read for the words and `aria-busy` whatever the pane shows; the line
 * itself is not drawn on the selected pill of an expanded pane, whose border and transcript already
 * say it (SPEC_PANE_TAB_PROGRESS_LINE_2026_10_24.md §3). A line that ends fades out in its last look.
 */
export function usePillProgress<T>(
    props: {
        tab: T;
        active: boolean;
        useProgress?: (tab: T) => Accessor<PaneTabProgress | null>;
        collapsed?: Accessor<boolean>;
    },
    id: () => string
) {
    const progress = props.useProgress?.(props.tab);
    const progressInfo = (): PaneTabProgress | null => progress?.() ?? null;
    const lineState = (): PaneTabProgress["state"] | null => {
        const info = progressInfo();
        if (!info) return null;
        return props.active && !(props.collapsed?.() ?? false) ? null : info.state;
    };
    const lineLook = createMemo<PaneTabProgress["state"] | null>((prev) => lineState() ?? prev ?? null);
    const descriptionId = () => `pane-tab-state-${id()}`;
    return { progressInfo, lineState, lineLook, descriptionId };
}

/** The tooltip's body when the pill has a detail, a seat or a progress state to show; else its plain text. */
export function pillTooltipContent<T>(
    props: {
        tab: T;
        getLabel: (tab: T) => string;
        getTooltip?: (tab: T) => string;
        getTooltipDetail?: (tab: T) => string | undefined;
        getTooltipSeat?: (tab: T) => string | undefined;
    },
    progressInfo: () => PaneTabProgress | null
): JSX.Element {
    return props.getTooltipDetail?.(props.tab) || props.getTooltipSeat?.(props.tab) || progressInfo() ? (
        <div class="pane-tab-tip-body" data-testid="pane-tab-tip-body">
            <div class="pane-tab-tip-label">{props.getTooltip?.(props.tab) ?? props.getLabel(props.tab)}</div>
            <Show when={progressInfo()}>{(info) => <div class="pane-tab-tip-state">{info().text}</div>}</Show>
            <Show when={props.getTooltipSeat?.(props.tab)}>{(seat) => <div class="pane-tab-tip-seat">{seat()}</div>}</Show>
            <Show when={props.getTooltipDetail?.(props.tab)}>{(detail) => <div class="pane-tab-tip-summary">{detail()}</div>}</Show>
        </div>
    ) : (
        (props.getTooltip?.(props.tab) ?? props.getLabel(props.tab))
    );
}
