// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { LogoMenu } from "@/app/window/logo-menu";
import { isMacOS } from "@/util/platformutil";
import type { JSX } from "solid-js";
import { Show } from "solid-js";
import { useTabDragAndDrop } from "./tab-reorder";
import { useTabTearOffEvents } from "./tab-tearoff-events";
import { createTearOffTabAtRelease } from "./tab-tearoff-rpc";
import { createScrollEdges } from "@/app/element/scroll-edges";
import { CloseConfirm, TabStripScroll } from "./tabbar-parts";
import {
    usePinnedTabsMigration,
    useRevealActiveTab,
    useTabClose,
    useTabLists,
    useTabSelection,
} from "./tabbar-state";
import "./tabbar.scss";

interface TabBarProps {
    workspace: Workspace;
}

function TabBar(props: TabBarProps): JSX.Element {
    let tabBarScrollRef!: HTMLDivElement;

    const lists = useTabLists(props);
    const { allTabIds, tabIds } = lists;
    const selection = useTabSelection(lists);
    const { displayActiveTabId } = selection;
    const close = useTabClose(props, lists, selection);

    const { dragProps } = useWindowDrag();

    usePinnedTabsMigration(props);

    // The startup tab intentionally has no `tab:color` — see
    // docs/reports/REPORT_REMOVE_AUTO_TAB_COLOR_2026_08_18.md. This used to
    // backfill a fixed "Blue" here so the first tab wouldn't look different
    // from every (then-randomly-colored) subsequent tab; now that new tabs
    // no longer auto-assign a color either (tab-actions.ts's createTab()),
    // there's no inconsistency left to paper over.

    // Commit-on-release tab tear-off. Fired from the drag monitor's onDrop
    // (useTabDragAndDrop, tab-reorder.ts) when the tab is released below
    // the strip. See tab-tearoff-rpc.ts for the full derivation.
    const tearOffTabAtRelease = createTearOffTabAtRelease(
        () => props.workspace,
        () => tabBarScrollRef
    );

    // In-strip reorder DnD + pane-drag-over-strip cleanup + Windows
    // tear-off-cursor workaround + wheel-scroll.
    //
    // RAW list, not the filtered one: these hooks compute BACKEND indices
    // (executeReorder's insertion math, tear-off merge/restore positions)
    // against the workspace's real tab_ids, which still contains any
    // optimistically-hidden mid-close tab until its RPC lands. Feeding
    // them the filtered list shifts every computed index by the number of
    // hidden tabs and silently misplaces a concurrently dragged tab
    // (#2818). The filtered tabIds() is for RENDERING only.
    useTabDragAndDrop({ tabBarScrollRef: () => tabBarScrollRef }, () => props.workspace, allTabIds, tearOffTabAtRelease);

    // Edge fades while the tabs overflow the strip (scroll-edge-fade,
    // tabbar.scss): the strip hides its scrollbar, so this is the cue that
    // more tabs are a wheel-scroll away.
    const edges = createScrollEdges(() => tabBarScrollRef);

    // Phase 4/5 — cross-window tear-off event listeners (hover/merge/
    // standalone/cancel-back). Raw list for the same reason as above.
    useTabTearOffEvents(
        () => props.workspace,
        () => tabBarScrollRef,
        allTabIds
    );

    if (!props.workspace) return null;

    const activeIndex = () => tabIds().indexOf(displayActiveTabId());

    useRevealActiveTab(displayActiveTabId, tabIds, () => tabBarScrollRef);

    return (
        <div class="tab-bar" {...dragProps}>
            {/* Windows/Linux: AgentMux logo sits at the LEFT of the tab strip.
                On macOS it's rendered at the far right of the window header
                instead (see window-header.tsx) so it clears the native
                traffic-light controls. */}
            <Show when={!isMacOS()}>
                <LogoMenu />
            </Show>
            <TabStripScroll
                setScrollRef={(el) => (tabBarScrollRef = el)}
                moreLeft={edges.moreLeft()}
                moreRight={edges.moreRight()}
                workspaceId={props.workspace.oid}
                tabIds={tabIds()}
                allTabIds={allTabIds()}
                displayActiveTabId={displayActiveTabId()}
                activeIndex={activeIndex()}
                onSelect={selection.handleSelect}
                onClose={close.requestClose}
            />
            <CloseConfirm tabId={close.pendingCloseTabId()} onConfirm={close.confirmClose} onCancel={close.cancelClose} />
        </div>
    );
}

export { TabBar };
