// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { ModalLayer } from "@/element/ModalLayer";
import { CenteredDiv } from "@/element/quickelems";
import logoUrl from "@/app/asset/logo-brain.svg?url";
import { ContentRenderer, NodeModel, PreviewRenderer, TileLayout } from "@/layout/index";
import { TileLayoutContents } from "@/layout/lib/types";
import { atoms, createBlock, getApi, getHostName, getUserName, isDev } from "@/store/global";
import * as services from "@/store/services";
import * as WOS from "@/store/wos";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { createMemo, Show } from "solid-js";
import type { JSX } from "solid-js";

/**
 * Build a widget menu for right-clicking an empty tab (no panes). Grouped
 * widgets (e.g. Messengers' Discord/Slack/etc.) nest under their parent's
 * own label instead of each showing up individually — see
 * buildPaneWidgetMenuItems.
 */
function buildEmptyTabMenu(): ContextMenuItem[] {
    const wmap = atoms.fullConfigAtom()?.widgets ?? {};
    return buildPaneWidgetMenuItems(wmap, (blockdef) => void createBlock(blockdef));
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

    const isEmpty = createMemo(() => (tabData()?.blockids?.length ?? 0) === 0);

    const rootStyle = (): JSX.CSSProperties => ({
        background: "var(--workspace-surface)",
    });

    return (
        <div
            class="flex flex-row flex-grow min-h-0 w-full items-center justify-center overflow-hidden relative"
            style={rootStyle()}
            onContextMenu={handleContextMenu}
        >
            <Show
                when={tabData() != null}
                fallback={<CenteredDiv>Tab Not Found</CenteredDiv>}
            >
                <Show
                    when={isEmpty()}
                    fallback={
                        <ModalLayer scope="tab">
                            <TileLayout
                                contents={tileLayoutContents()}
                                tabAtom={tabAtom()}
                                getCursorPoint={getApi().getCursorPoint}
                            />
                        </ModalLayer>
                    }
                >
                    <EmptyTabIdentity />
                </Show>
            </Show>
        </div>
    );
}

/**
 * Identity panel shown on an empty tab — logo + the standard "who/what/where"
 * line every desktop app shows during a quiet startup screen: `user@host`,
 * version, git hash. Pulled from cached globals (`getUserName` / `getHostName`)
 * and the host's about-details payload, no IPC on render.
 */
function EmptyTabIdentity(): JSX.Element {
    const details = getApi().getAboutModalDetails();
    const version = details?.version ?? "";
    const gitHash = details?.gitHash ?? "";
    const buildLabel = gitHash ? `${isDev() ? "dev-" : ""}${gitHash}` : "";

    return (
        <div
            class="flex flex-col items-center gap-3 select-none pointer-events-none"
            style={{ opacity: "0.4" }}
        >
            <img
                src={logoUrl}
                alt="AgentMux"
                class="empty-tab-logo"
                style={{ "max-width": "160px", "max-height": "160px" }}
            />
            <div class="flex flex-col items-center gap-1 text-secondary text-[11px] leading-4 text-center">
                <div>{getUserName()}@{getHostName()}</div>
                <Show when={version}>
                    <div>
                        v{version}
                        <Show when={buildLabel}>{" "}({buildLabel})</Show>
                    </div>
                </Show>
            </div>
        </div>
    );
}

export { TabContent };
