// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { CenteredDiv } from "@/element/quickelems";
import { ContentRenderer, NodeModel, PreviewRenderer, TileLayout } from "@/layout/index";
import { TileLayoutContents } from "@/layout/lib/types";
import { atoms, createBlock, getApi } from "@/store/global";
import * as services from "@/store/services";
import * as WOS from "@/store/wos";
import { createMemo, Show } from "solid-js";
import type { JSX } from "solid-js";

/** Non-pane widget views excluded from the empty-tab context menu. */
const nonPaneViews = new Set(["devtools", "settings"]);

/** Build a flat widget menu for right-clicking an empty tab (no panes). */
function buildEmptyTabMenu(): ContextMenuItem[] {
    const fullConfig = atoms.fullConfigAtom();
    const widgets = fullConfig?.widgets ?? {};

    return Object.values(widgets)
        .filter((w) => {
            const view = w.blockdef?.meta?.view;
            return view && !nonPaneViews.has(view);
        })
        .sort((a, b) => {
            const orderA = a["display:order"] ?? 0;
            const orderB = b["display:order"] ?? 0;
            if (orderA !== orderB) return orderA - orderB;
            return (a.label ?? "").localeCompare(b.label ?? "");
        })
        .map((widget) => ({
            label: widget.label ?? "Unnamed",
            click: () => void createBlock(widget.blockdef),
        }));
}

function TabContent(props: { tabId: string }): JSX.Element {
    const oref = createMemo(() => WOS.makeORef("tab", props.tabId));
    const tabAtom = createMemo(() => WOS.getWaveObjectAtom<Tab>(oref()));
    const tabData = createMemo(() => tabAtom()());

    const tileGapSize = createMemo(() => {
        const settings = atoms.settingsAtom();
        return settings["window:tilegapsize"];
    });

    const tileLayoutContents = createMemo<TileLayoutContents>(() => {
        const renderContent: ContentRenderer = (nodeModel: NodeModel) => {
            return <Block nodeModel={nodeModel} preview={false} />;
        };

        const renderPreview: PreviewRenderer = (nodeModel: NodeModel) => {
            return <Block nodeModel={nodeModel} preview={true} />;
        };

        async function onNodeDelete(data: TabLayoutData) {
            getApi().sendLog(`[BUG-TRACE] onNodeDelete ENTER for blockId: ${data.blockId}`);
            try {
                const result = await services.ObjectService.DeleteBlock(data.blockId);
                getApi().sendLog(`[BUG-TRACE] onNodeDelete DeleteBlock returned: ${JSON.stringify(result)}`);
                return result;
            } catch (err) {
                getApi().sendLog(`[BUG-TRACE] onNodeDelete ERROR: ${err}`);
                throw err;
            }
        }

        return {
            renderContent,
            renderPreview,
            tabId: props.tabId,
            onNodeDelete,
            gapSizePx: tileGapSize(),
        };
    });

    const handleContextMenu = (e: MouseEvent) => {
        const tab = tabData();
        if (!tab || (tab.blockids?.length ?? 0) > 0) return;
        e.preventDefault();
        e.stopPropagation();
        const menu = buildEmptyTabMenu();
        if (menu.length > 0) {
            ContextMenuModel.showContextMenu(menu, e);
        }
    };

    return (
        <div
            class="flex flex-row flex-grow min-h-0 w-full items-center justify-center overflow-hidden relative"
            onContextMenu={handleContextMenu}
        >
            <Show
                when={tabData() != null}
                fallback={<CenteredDiv>Tab Not Found</CenteredDiv>}
            >
                <TileLayout
                    contents={tileLayoutContents()}
                    tabAtom={tabAtom()}
                    getCursorPoint={getApi().getCursorPoint}
                />
            </Show>
        </div>
    );
}

export { TabContent };
