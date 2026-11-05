// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The parts of the window tab strip: the scrolling tab list and the close confirmation. */

import { isMacOS } from "@/util/platformutil";
import type { JSX } from "solid-js";
import { For, Show } from "solid-js";
import { DroppableTab } from "./droppable-tab";
import { TabCloseConfirmModal } from "./tab-close-confirm-modal";

/** The strip's scrolling area: separators, the tabs, and the draggable fill after the last one. */
export function TabStripScroll(props: {
    setScrollRef: (el: HTMLDivElement) => void;
    moreLeft: boolean;
    moreRight: boolean;
    workspaceId: string;
    /** What is rendered: the tabs minus any mid-close ones. */
    tabIds: string[];
    /** The raw list, for the backend-facing props. */
    allTabIds: string[];
    displayActiveTabId: string;
    activeIndex: number;
    onSelect: (tabId: string) => void;
    onClose: (tabId: string) => void;
}): JSX.Element {
    return (
        <div
            ref={props.setScrollRef}
            class="tab-bar-scroll"
            classList={{ "has-more-left": props.moreLeft, "has-more-right": props.moreRight }}
            data-drag-region="false"
        >
            {/* When the AgentMux logo sits to the left of the tabs (Windows/
                Linux), give the AgentMux logo→first-tab boundary the SAME 1px
                separator every tab-to-tab boundary has — otherwise the
                first tab is flush against the AgentMux logo and reads tighter
                than the rest. macOS renders the AgentMux logo at the far right,
                so no leading separator there. */}
            <Show when={!isMacOS()}>
                <div class="tab-separator" aria-hidden="true" />
            </Show>
            <For each={props.tabIds}>
                {(tabId, i) => (
                    <>
                        {/* Real DOM separator between adjacent tabs (skipped
                            before index 0). Constant width + identical CSS
                            in every position guarantees uniform inter-tab
                            spacing, regardless of which tab is active /
                            hovered / dragged. Per
                            SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25 §3.4. */}
                        <Show when={i() > 0}>
                            <div class="tab-separator" aria-hidden="true" />
                        </Show>
                        <DroppableTab
                            tabId={tabId}
                            workspaceId={props.workspaceId}
                            activeTabId={props.displayActiveTabId}
                            isActive={tabId === props.displayActiveTabId}
                            isFirst={i() === 0}
                            isBeforeActive={i() === props.activeIndex - 1}
                            // RAW count, not the filtered one: while a
                            // close is pending the workspace still HAS
                            // the hidden tab, and undercounting here
                            // would flip droppable-tab's isLoneTabDrag/
                            // canDrag on the survivor of a 2-tab
                            // workspace, spuriously disabling its drag
                            // until the RPC resolves (#2818).
                            allTabCount={props.allTabIds.length}
                            // Backend-facing index/list (drag payload's
                            // tabIndex feeds ReorderTab math): use the
                            // RAW list so indices line up with the
                            // workspace's real tab_ids even while a
                            // mid-close tab is hidden from rendering
                            // (#2818). i() stays for
                            // the visual props above (isFirst /
                            // isBeforeActive / separators).
                            tabIndex={props.allTabIds.indexOf(tabId)}
                            tabIds={props.allTabIds}
                            onSelect={() => props.onSelect(tabId)}
                            onClose={() => props.onClose(tabId)}
                        />
                    </>
                )}
            </For>
            {/* The last tab ends with the same separator every other tab
                boundary has, so it does not run straight into the drag
                square (SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13). */}
            <Show when={props.tabIds.length > 0}>
                <div class="tab-separator" aria-hidden="true" />
            </Show>
            {/* Fill lives INSIDE the scroll container so the genuine empty
                space to the right of the last tab is draggable. isInDragRegion
                walks UP the DOM from the clicked element: the fill's own
                data-drag-region="true" is found before the scroll container's
                "false", so a click here starts a window drag. Moving it outside
                the scroll (as a sibling) left a dead zone — the empty interior
                of the scroll container looked draggable but wasn't. */}
            <div class="tab-bar-fill" data-drag-region="true" />
        </div>
    );
}

/** The confirmation that opens when closing a tab is set to ask first. */
export function CloseConfirm(props: {
    tabId: string | null;
    onConfirm: (skipFuture: boolean) => void;
    onCancel: () => void;
}): JSX.Element {
    return (
        <Show when={props.tabId !== null}>
            <TabCloseConfirmModal tabId={props.tabId!} onConfirm={props.onConfirm} onCancel={props.onCancel} />
        </Show>
    );
}
