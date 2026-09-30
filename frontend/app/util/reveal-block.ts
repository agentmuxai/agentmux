// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Reveal a block: bring it on screen and focus it, whatever it takes — switch
 * the window tab, switch its pane to it when it's a background tab of a
 * multi-tab pane, un-magnify another pane that would hide it, focus the pane,
 * and put the caret in it.
 *
 * `focusBlock` (Swarm rows, the token popover) used to stop at "focus the
 * pane": `getNodeByBlockId` returns the PANE for a background member, and
 * `focusNode` never switches a pane's active tab, so the pane was selected but
 * kept showing another agent. This adds that step.
 *
 * SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md §4.1 (this window), §4.3 (another
 * window).
 */

import { giveBlockFocus } from "@/app/store/focusManager";
import { getApi, MOS, setActiveTab, workspace } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WorkspaceService } from "@/app/store/services";
import { setActiveBlockInStack } from "@/layout/index";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import type { LayoutNode } from "@/layout/lib/types";

export interface RevealOptions {
    /** The tab holding the block, when the caller already knows it. */
    tabId?: string;
    /** Put the caret in the block (its composer / terminal). Default true. */
    focusCaret?: boolean;
}

/** How long to wait for a just-activated tab's layout to contain the block. */
const LAYOUT_POLL_TRIES = 10;
const LAYOUT_POLL_MS = 50;

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** The tab in THIS window's workspace whose `blockids` include `blockId`.
 *  Reads Tab objects rather than layout models, so it also finds tabs whose
 *  layout hasn't been built yet. Pinned tabs first. */
async function findTabHolding(blockId: string, hint?: string): Promise<string | null> {
    const ws = workspace();
    if (!ws) return null;
    const tabIds = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    const candidates = hint && tabIds.includes(hint) ? [hint, ...tabIds.filter((t) => t !== hint)] : tabIds;
    for (const tabId of candidates) {
        const oref = MOS.makeORef("tab", tabId);
        const tab = MOS.getObjectValue<Tab>(oref) ?? (await MOS.reloadMuxObject<Tab>(oref));
        if (tab?.blockids?.includes(blockId)) return tabId;
    }
    return null;
}

/**
 * Make `blockId` visible within its layout without switching the window tab,
 * moving layout focus or the caret: restore its pane if minimized, un-magnify
 * another pane that would hide it, and switch its pane to it when it's a
 * background member of a multi-tab pane. `node` is `getNodeByBlockId`'s
 * result (the shared pane node for a stack member).
 * SPEC_EDITOR_REUSE_RESTORES_MINIMIZED_PANE_2026_09_30.md.
 */
export function showBlockInPane(model: LayoutModel, node: LayoutNode, blockId: string): void {
    // minimizeNodeToggle is a toggle, so only call it on a minimized pane.
    // focusNode doesn't restore one either, which is why reveal needed this.
    if (node.minimized) model.minimizeNodeToggle(node.id);

    // Another pane magnified would keep the target hidden behind it.
    const magnified = model.magnifiedNodeId;
    if (magnified && magnified !== node.id) model.magnifyNodeToggle(magnified);

    // A no-op for a single-block pane or the already-active tab.
    setActiveBlockInStack(model, node.id, blockId);
}

/**
 * `showBlockInPane` for a block in THIS window, found by id, WITHOUT switching
 * to its tab. For an agent opening a file into an existing Editor pane: show it,
 * but don't pull the user off the tab they're on or take their caret. Returns
 * false when the block isn't in a built layout in this window.
 */
export async function showBlockWithoutFocus(blockId: string, opts: { tabId?: string } = {}): Promise<boolean> {
    const tabId = await findTabHolding(blockId, opts.tabId);
    if (!tabId) return false;
    const model = getLayoutModelForTabById(tabId);
    const node = model?.getNodeByBlockId(blockId);
    if (!model || node?.id == null) return false;
    showBlockInPane(model, node, blockId);
    return true;
}

/**
 * Reveal `blockId` in THIS window. Returns false when the block isn't in this
 * window's workspace (the caller may then look in other windows).
 */
export async function revealBlockLocally(blockId: string, opts: RevealOptions = {}): Promise<boolean> {
    const tabId = await findTabHolding(blockId, opts.tabId);
    if (!tabId) return false;

    await setActiveTab(tabId); // no-op when already active

    // A tab that wasn't visited yet builds its layout only after the switch
    // renders; wait briefly for the block to appear in it.
    let model = getLayoutModelForTabById(tabId);
    let node = model?.getNodeByBlockId(blockId);
    for (let i = 0; i < LAYOUT_POLL_TRIES && node?.id == null; i++) {
        await sleep(LAYOUT_POLL_MS);
        model = getLayoutModelForTabById(tabId);
        node = model?.getNodeByBlockId(blockId);
    }
    if (!model || node?.id == null) return false;

    showBlockInPane(model, node, blockId);

    model.focusNode(node.id);

    // After the switch: a hidden kept-alive tab is still registered and would
    // otherwise take the caret while invisible. Also covers focusNode's early
    // return when the pane was already focused.
    if (opts.focusCaret !== false) giveBlockFocus(blockId);
    return true;
}

/**
 * Reveal `blockId` wherever it is: in this window if it's here, else in the
 * window that shows it. A renderer can't drive another window's layout, so
 * srv (`block.reveal`) tells THAT window's renderer to reveal it
 * (`block:reveal`, reveal-block-events.ts) and this one raises that window.
 * Spec §4.3.
 */
export async function revealBlock(blockId: string, opts: RevealOptions = {}): Promise<void> {
    if (await revealBlockLocally(blockId, opts)) return;
    await revealInOtherWindow(blockId);
}

async function revealInOtherWindow(blockId: string): Promise<void> {
    try {
        const r = await RpcApi.BlockRevealCommand(TabRpcClient, { block_id: blockId });
        if (r?.found && r.window_id) {
            await raiseWindowById(r.window_id);
            return;
        }
    } catch (e) {
        // An older srv without block.reveal: fall back below.
        console.warn("[reveal-block] block.reveal failed", { blockId, error: String(e) });
    }
    await activateTabInOtherWorkspace(blockId);
}

/** Raise the window whose `Window` oid is `windowId`. */
async function raiseWindowById(windowId: string): Promise<void> {
    const instances = await getApi().listWindowInstances();
    const instance = instances.find((i) => i.windowId === windowId);
    if (instance?.label) await getApi().focusWindow(instance.label);
}

/** Fallback when srv can't name an open window for the block: activate its
 *  tab in the workspace that holds it and raise that workspace's window.
 *  Selects the window tab, not the pane or its tab. */
async function activateTabInOtherWorkspace(blockId: string): Promise<void> {
    // We need Tab.blockids to find which tab holds the block: layout models for
    // other windows' tabs aren't available in this renderer.
    const ws = workspace();
    const allWorkspaces = await RpcApi.WorkspaceListCommand(TabRpcClient);
    for (const wsInfo of allWorkspaces) {
        if (wsInfo.workspacedata.oid === ws?.oid) continue;
        const wsData = wsInfo.workspacedata;
        const allTabIds = [...(wsData.pinnedtabids ?? []), ...(wsData.tabids ?? [])];
        for (const tabId of allTabIds) {
            const oref = MOS.makeORef("tab", tabId);
            const tab = MOS.getObjectValue<Tab>(oref) ?? (await MOS.reloadMuxObject<Tab>(oref));
            if (!tab?.blockids?.includes(blockId)) continue;
            await WorkspaceService.SetActiveTab(wsData.oid, tabId);
            if (wsInfo.windowid) await raiseWindowById(wsInfo.windowid);
            return;
        }
    }
}
