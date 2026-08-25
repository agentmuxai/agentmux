// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { HamburgerMenu } from "@/app/window/hamburger-menu";
import { deleteLayoutModelForTab } from "@/layout/index";
import { settingsAtom } from "@/store/config-signals";
import { atoms, setActiveTab } from "@/store/global";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { isMacOS } from "@/util/platformutil";
import { fireAndForget } from "@/util/util";
import type { JSX } from "solid-js";
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { WorkspaceService } from "../store/services";
import { DroppableTab } from "./droppable-tab";
import { TabCloseConfirmModal } from "./tab-close-confirm-modal";
import { registerTabCloseRequestHandler } from "./tab-close-request";
import { useTabDragAndDrop } from "./tab-reorder";
import { useTabTearOffEvents } from "./tab-tearoff-events";
import { createTearOffTabAtRelease } from "./tab-tearoff-rpc";
import "./tabbar.scss";

interface TabBarProps {
    workspace: Workspace;
}

function TabBar(props: TabBarProps): JSX.Element {
    const activeTabId = atoms.activeTabId;
    let tabBarRef!: HTMLDivElement;
    let tabBarScrollRef!: HTMLDivElement;
    let tabBarFillRef!: HTMLDivElement;

    // Pin feature removed — merge any legacy pinnedtabids into the regular list
    // so existing workspaces don't lose tabs. A one-time UpdateTabIds (below)
    // drains pinnedtabids server-side so this concat becomes a no-op.
    const tabIds = () => {
        const ws = props.workspace;
        if (!ws) return [];
        return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    };

    const handleSelect = (tabId: string) => {
        if (tabId === activeTabId()) return;
        setActiveTab(tabId);
    };

    const handleClose = (tabId: string) => {
        const allTabs = tabIds();
        if (allTabs.length <= 1) return;
        fireAndForget(async () => {
            if (tabId === activeTabId()) {
                const idx = allTabs.indexOf(tabId);
                const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
                if (nextTab) await setActiveTab(nextTab);
            }
            await WorkspaceService.CloseTab(props.workspace.oid, tabId);
            deleteLayoutModelForTab(tabId);
        });
    };

    const [pendingCloseTabId, setPendingCloseTabId] = createSignal<string | null>(null);

    const requestClose = (tabId: string) => {
        if (tabIds().length <= 1) return;
        if ((settingsAtom() as any)["tab:skipcloseconfirm"]) {
            handleClose(tabId);
        } else {
            setPendingCloseTabId(tabId);
        }
    };

    onMount(() => {
        const unregister = registerTabCloseRequestHandler(() => requestClose(activeTabId()));
        onCleanup(unregister);
    });

    const { dragProps } = useWindowDrag();

    // One-time migration: if this workspace still has pinned tabs from an older
    // build, fold them into tabids and clear pinnedtabids server-side.
    onMount(() => {
        const ws = props.workspace;
        if (ws && (ws.pinnedtabids?.length ?? 0) > 0) {
            const merged = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
            fireAndForget(async () => {
                try {
                    await WorkspaceService.UpdateTabIds(ws.oid, merged, []);
                } catch (e) {
                    console.error("[tabbar] pin migration failed:", e);
                }
            });
        }
    });

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
    useTabDragAndDrop({ tabBarScrollRef: () => tabBarScrollRef }, () => props.workspace, tabIds, tearOffTabAtRelease);

    // Phase 4/5 — cross-window tear-off event listeners (hover/merge/
    // standalone/cancel-back).
    useTabTearOffEvents(
        () => props.workspace,
        () => tabBarScrollRef,
        tabIds
    );

    if (!props.workspace) return null;

    const activeIndex = () => tabIds().indexOf(activeTabId());

    return (
        <div ref={tabBarRef!} class="tab-bar" {...dragProps}>
            {/* Windows/Linux: hamburger sits at the LEFT of the tab strip.
                On macOS it's rendered at the far right of the window header
                instead (see window-header.tsx) so it clears the native
                traffic-light controls. */}
            <Show when={!isMacOS()}>
                <HamburgerMenu />
            </Show>
            <div ref={tabBarScrollRef!} class="tab-bar-scroll" data-drag-region="false">
                {/* When the hamburger sits to the left of the tabs (Windows/
                    Linux), give the hamburger→first-tab boundary the SAME 1px
                    separator every tab-to-tab boundary has — otherwise the
                    first tab is flush against the hamburger and reads tighter
                    than the rest. macOS renders the hamburger at the far right,
                    so no leading separator there. */}
                <Show when={!isMacOS()}>
                    <div class="tab-separator" aria-hidden="true" />
                </Show>
                <For each={tabIds()}>
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
                                workspaceId={props.workspace.oid}
                                activeTabId={activeTabId()}
                                isActive={tabId === activeTabId()}
                                isFirst={i() === 0}
                                isBeforeActive={i() === activeIndex() - 1}
                                allTabCount={tabIds().length}
                                tabIndex={i()}
                                tabIds={tabIds()}
                                onSelect={() => handleSelect(tabId)}
                                onClose={() => requestClose(tabId)}
                            />
                        </>
                    )}
                </For>
                {/* Fill lives INSIDE the scroll container so the genuine empty
                    space to the right of the last tab is draggable. isInDragRegion
                    walks UP the DOM from the clicked element: the fill's own
                    data-drag-region="true" is found before the scroll container's
                    "false", so a click here starts a window drag. Moving it outside
                    the scroll (as a sibling) left a dead zone — the empty interior
                    of the scroll container looked draggable but wasn't. */}
                <div ref={tabBarFillRef!} class="tab-bar-fill" data-drag-region="true" />
            </div>
            <Show when={pendingCloseTabId() !== null}>
                <TabCloseConfirmModal
                    tabId={pendingCloseTabId()!}
                    onConfirm={(skipFuture) => {
                        const tabId = pendingCloseTabId()!;
                        setPendingCloseTabId(null);
                        if (skipFuture) {
                            fireAndForget(() =>
                                RpcApi.SetConfigCommand(TabRpcClient, { "tab:skipcloseconfirm": true } as any)
                            );
                        }
                        handleClose(tabId);
                    }}
                    onCancel={() => setPendingCloseTabId(null)}
                />
            </Show>
        </div>
    );
}

export { TabBar };
