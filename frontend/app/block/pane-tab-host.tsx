// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host side of a NATIVE pane tab (a manifest with `create(ctx)`, Pane Tab
 * contract Phase 2b — docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3/§4):
 * the context the host hands an instance, and the adapter that presents the
 * instance as the ViewModel the rest of the host consumes today. Legacy
 * ViewModel classes (`legacyAdapter`) never pass through here.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { NodeModel } from "@/layout/index";
import { getMuxObjectAtom, makeORef } from "@/store/mos";
import type { PaneTabHostContext, PaneTabInstance, PaneTabManifest } from "./pane-tab-registry";
import { usePaneTabVisibility } from "./pane-tab-visibility";

/** Call inside the instance's own reactive root (block.tsx's `makeViewModel`),
 *  which then pins the block for as long as the instance lives, and inherits
 *  the block's context (its window tab, for `visibility`). */
export function makePaneTabHostContext(blockId: string, nodeModel: NodeModel): PaneTabHostContext {
    const oref = makeORef("block", blockId);
    const block = getMuxObjectAtom<Block>(oref);
    return {
        blockId,
        meta: () => block()?.meta,
        setMeta: async (patch) => {
            await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: patch as MetaType });
        },
        isFocused: () => nodeModel.isFocused?.() ?? false,
        visibility: usePaneTabVisibility(blockId),
    };
}

export function adaptPaneTabInstance(
    manifest: PaneTabManifest,
    ctx: PaneTabHostContext,
    instance: PaneTabInstance
): ViewModel {
    const Component = instance.component;
    const vm: ViewModel = {
        viewType: manifest.view,
        blockId: ctx.blockId,
        viewComponent: () => <Component ctx={ctx} />,
        dispose: () => instance.dispose?.(),
    };
    const title = instance.liveTitle;
    if (title) {
        vm.viewName = () => title().text;
        vm.viewNameIsPlaceholder = () => title().placeholder === true;
    }
    if (instance.liveFavicon) vm.viewFaviconUrl = instance.liveFavicon;
    if (instance.headerText) vm.viewText = instance.headerText;
    if (instance.headerActions) vm.endIconButtons = instance.headerActions;
    if (instance.contextMenu) vm.getBodyContextMenuItems = (c) => instance.contextMenu!(c);
    if (instance.settingsMenu) vm.getSettingsMenuItems = () => instance.settingsMenu!();
    if (manifest.capabilities?.connection) vm.manageConnection = () => true;
    if (manifest.capabilities?.noPadding) vm.noPadding = () => true;
    if (instance.focus) vm.giveFocus = () => instance.focus!();
    if (instance.onKeyDown) vm.keyDownHandler = (e) => instance.onKeyDown!(e);
    if (instance.onActivate) vm.onActivate = () => instance.onActivate!();
    if (instance.onDeactivate) vm.onDeactivate = () => instance.onDeactivate!();
    return vm;
}
