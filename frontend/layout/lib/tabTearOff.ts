// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One tab dragged out of a pane that keeps other tabs: the tab's own place while its drag is on. A whole pane's
 * drag starts over the pane's own place, which says a release there tears the pane off (TearOffPlace.tsx,
 * futureLayout.ts `tearOffOffered`). A tab's drag had no such place, and what left the window with it was a
 * picture of the whole pane, its other tabs included (the operator, 2026-11-13: "when a pane tab tears off, its
 * taking all the other pane tabs with it. If you tear only a pane tab, we need the same checker-red tear off,
 * but only the pane tab goes away, but follows the same rules where if it comes back it simply becomes a pane
 * trying to drop in").
 *
 * So a tab has the same two moments a pane has:
 * - `offered`, from the drag's start: the pane keeps its header and its other tabs, and the tab's own place
 *   (the pane's body) is the tear-off place. A release there tears the tab off. The pane's body places nothing
 *   for its own tab meanwhile, as a dragged pane's own place does not.
 * - gone, from the first time anything else is shown (the tab over another pane, a drop the layout shows, the
 *   pointer out of the window) to the end of the drag: the red does not come back. The pane is shown as the
 *   tab leaves it: no pill for it, and in front the tab that comes to the front when it goes
 *   (app/tab/pane-leaf-state.ts `useShownBlockId`; the operator, 2026-11-13: "when tearing a pane tab with
 *   pane tabs left behind, instead of seeing their content, we see the agentmux logo"). The tab is a pane
 *   trying to drop in wherever it goes, its own pane included.
 *
 * A release that tears the tab off (over its own place while offered, or outside the window) keeps the pane
 * showing the tab gone until the tab has left it (`holdTabTearOff`), so the tab does not come back for the
 * moment the tear-off takes.
 *
 * Only for a single tab of a pane with others: a pane's only tab moves the pane, and several tabs dragged
 * together do not tear off. Only the state, with no dependencies, so the tile, the strip and the drop zones
 * can all read it; the drag says when it starts and ends (app/drag/pane-tab-drag.ts).
 */

import { createSignal } from "solid-js";
import type { LayoutModel } from "./layoutModel";

/** How long a released tear-off keeps the tab shown gone while it has not happened yet; then the tab is back. */
export const TAB_TEAR_OFF_WAIT_MS = 2000;

interface TabTearOff {
    model: LayoutModel;
    blockId: string;
    /** The pane the tab is dragged out of. */
    nodeId: string;
    /** The tab's place still says a release tears it off. */
    offered: boolean;
}

const [state, setState] = createSignal<TabTearOff | undefined>(undefined);
let held: ReturnType<typeof setTimeout> | undefined;

/** The drag of tab `blockId` out of pane `nodeId`, which keeps other tabs, began. */
export function beginTabTearOff(model: LayoutModel, blockId: string, nodeId: string): void {
    clearTimeout(held);
    held = undefined;
    setState({ model, blockId, nodeId, offered: true });
}

/** Something else was shown for the tab (another pane takes it, the layout shows a drop, it left the window). */
export function tabTearOffShown(): void {
    const now = state();
    if (now?.offered) setState({ ...now, offered: false });
}

/** The drag is over and nothing tears the tab off, or the tear-off is done: the pane is as its tabs say. */
export function endTabTearOff(): void {
    clearTimeout(held);
    held = undefined;
    if (state()) setState(undefined);
}

/** Whether a released tear-off is being waited for (`holdTabTearOff`). */
export const tabTearOffHeld = (): boolean => held !== undefined;

/**
 * The drag was released where the tab tears off: the pane shows the tab gone until the tear-off says it is
 * done (`endTabTearOff`), or for `TAB_TEAR_OFF_WAIT_MS` if it never does.
 */
export function holdTabTearOff(): void {
    const now = state();
    if (!now) return;
    clearTimeout(held);
    if (now.offered) setState({ ...now, offered: false });
    held = setTimeout(endTabTearOff, TAB_TEAR_OFF_WAIT_MS);
}

/** Whether the body of pane `nodeId` is the tear-off place of a tab dragged out of it. Reactive. */
export function tabTearOffOfferedIn(model: LayoutModel, nodeId: string): boolean {
    const now = state();
    return !!now && now.offered && now.model === model && now.nodeId === nodeId;
}

/** The tab being dragged out of pane `nodeId`, whether its place still offers the tear-off or it has left, or none. Reactive. */
export function tabDraggedFrom(model: LayoutModel, nodeId: string): string | undefined {
    const now = state();
    return now && now.model === model && now.nodeId === nodeId ? now.blockId : undefined;
}

/** The tab that has left pane `nodeId` for the rest of its drag, or none. Reactive. */
export function tabGoneFrom(model: LayoutModel, nodeId: string): string | undefined {
    const now = state();
    return now && !now.offered && now.model === model && now.nodeId === nodeId ? now.blockId : undefined;
}

/** Whether tab `blockId` of pane `nodeId` has left the pane for the rest of its drag. Reactive. */
export function isTabGone(blockId: string, nodeId: string | undefined): boolean {
    const now = state();
    return !!now && !now.offered && now.blockId === blockId && now.nodeId === nodeId;
}

/** Whether tab `blockId`'s own place still offers its tear-off. Reactive. */
export function tabTearOffOffered(blockId: string): boolean {
    const now = state();
    return !!now && now.offered && now.blockId === blockId;
}
