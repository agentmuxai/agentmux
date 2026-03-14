// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Block } from "@/app/block/block";
import { CenteredDiv } from "@/element/quickelems";
import { ContentRenderer, NodeModel, PreviewRenderer, TileLayout } from "@/layout/index";
import { TileLayoutContents } from "@/layout/lib/types";
import { atoms, getApi } from "@/store/global";
import * as services from "@/store/services";
import * as WOS from "@/store/wos";
import { createMemo, Show } from "solid-js";
import type { JSX } from "solid-js";

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

    return (
        <div class="flex flex-row flex-grow min-h-0 w-full items-center justify-center overflow-hidden relative">
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
