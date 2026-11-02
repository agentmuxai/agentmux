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
import { createEffect, createRoot } from "solid-js";
import { createRequestedTabs, createSelectionRunner, type SelectionDeps } from "@/app/selection/selection-effects";
import { isFailure, type Intent, type RemoteOutcome, type SelectionResult } from "@/app/selection/selection-machine";
import { createSelectionTrace } from "@/app/selection/selection-trace";
import { atoms, getApi, MOS, pushNotification, setActiveTab, workspace } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ClientService, WorkspaceService } from "@/app/store/services";
import { setActiveBlockInStack } from "@/layout/index";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import type { LayoutNode } from "@/layout/lib/types";
import { emitPaneLocate } from "./pane-locate";
import { paneIdentity } from "./pane-identity";
import { selectTrace } from "./select-trace";

export interface RevealOptions {
    /** The tab holding the block, when the caller already knows it. */
    tabId?: string;
    /** Put the caret in the block (its composer / terminal). Default true. */
    focusCaret?: boolean;
    /** Who asked, for the selection trace: "swarm", "tower", "tokens", "picker". */
    source?: string;
    /** This window only: a block that is not here is not found, and no other window is asked. */
    localOnly?: boolean;
    /** No failure notice: whoever handed this request over tells the user. */
    quiet?: boolean;
}

/** The tab in THIS window's workspace whose `blockids` include `blockId`.
 *  Reads Tab objects rather than layout models, so it also finds tabs whose
 *  layout hasn't been built yet. Pinned tabs first. */
export async function findTabHolding(blockId: string, hint?: string): Promise<string | null> {
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
 * A pane of `view` open in THIS window: the first of this window's tabs (the shown one first) whose blocks
 * include one with that view. Read from the tabs' block lists, not from mounted components, so it also
 * finds a pane that is an inactive member of a pane's stack (not mounted) and a minimized one.
 * Null when none is open.
 */
export async function findOpenBlockByView(view: string, identity?: string): Promise<string | null> {
    const ws = workspace();
    if (!ws) return null;
    const tabIds = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    const shown = atoms.activeTabId();
    const ordered = tabIds.includes(shown) ? [shown, ...tabIds.filter((t) => t !== shown)] : tabIds;
    for (const tabId of ordered) {
        const tabRef = MOS.makeORef("tab", tabId);
        const tab = MOS.getObjectValue<Tab>(tabRef) ?? (await MOS.reloadMuxObject<Tab>(tabRef));
        for (const blockId of tab?.blockids ?? []) {
            const blockRef = MOS.makeORef("block", blockId);
            const block = MOS.getObjectValue<Block>(blockRef) ?? (await MOS.reloadMuxObject<Block>(blockRef));
            if (block?.meta?.["view"] === view && (identity == null || paneIdentity(view, block.meta) === identity)) return blockId;
        }
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

    // Another pane magnified would keep the target hidden behind it. A plain
    // toggle requests node focus once committed; this one must not.
    const magnified = model.magnifiedNodeId;
    if (magnified && magnified !== node.id) model.magnifyNodeToggle(magnified, true, false);

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

/** What the notice says for each reason a pane could not be shown. */
const FAIL_TEXT: Record<string, string> = {
    "not-found": "it is no longer open",
    "resolve-timeout": "it took too long to find",
    "tab-timeout": "its tab did not open in time",
    "layout-timeout": "its tab is still loading",
    mismatch: "it did not come to the front",
    stuck: "it did not respond in time",
    "remote-timeout": "the other window did not answer",
    "remote-failed": "the other window could not show it",
    gone: "it was just closed",
    error: "something went wrong",
};

/** The pane's own name, for the notice. */
function blockName(blockId: string): string {
    const meta = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta;
    const name = [meta?.["agentName"], meta?.["title"], meta?.["name"]].find((v) => typeof v === "string" && v.trim());
    return typeof name === "string" ? name.trim() : "that pane";
}

/** Two animation frames: what a tab switch schedules for after its RPC (a refocus) has had its turn. */
const twoFrames = () =>
    new Promise<void>((resolve) =>
        typeof requestAnimationFrame === "function"
            ? requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
            : setTimeout(resolve, 32)
    );

const requestedTabs = createRequestedTabs();

/** The real tabs, layouts and focus the selection machine drives. */
const liveDeps: SelectionDeps = {
    async resolve(intent) {
        const tabId = await findTabHolding(intent.blockId, intent.tabHint);
        // Not in this window's tabs: another window may show it (the handoff asks srv).
        if (!tabId) return { where: "other" };
        // The active tab is the committed one: an earlier request's switch to another tab may still be
        // on its way and would take the window off this one, so this request switches too.
        return { where: "here", tabId, active: atoms.activeTabId() === tabId && !requestedTabs.pendingElsewhere(tabId) };
    },
    switchTab(tabId) {
        requestedTabs.asked(tabId);
        return setActiveTab(tabId); // no-op when already active
    },
    // The tab must be the committed active one as well: a kept-alive layout is already there when the
    // switch replies, and the workspace push that makes the tab active can land after that reply.
    layoutHas: (tabId, blockId) =>
        atoms.activeTabId() === tabId && getLayoutModelForTabById(tabId)?.getNodeByBlockId(blockId)?.id != null,
    show(tabId, blockId) {
        const model = getLayoutModelForTabById(tabId);
        const node = model?.getNodeByBlockId(blockId);
        if (!model || node?.id == null) throw new Error("the pane left its layout");
        showBlockInPane(model, node, blockId);
    },
    focus(tabId, blockId, caret) {
        const model = getLayoutModelForTabById(tabId);
        const node = model?.getNodeByBlockId(blockId);
        if (!model || node?.id == null) throw new Error("the pane left its layout");
        model.focusNode(node.id);
        // After the switch: a hidden kept-alive tab is still registered and would otherwise take the
        // caret while invisible. Also covers focusNode's early return when the pane was already focused.
        if (caret) giveBlockFocus(blockId);
    },
    settle: twoFrames,
    verify(tabId, blockId) {
        const model = getLayoutModelForTabById(tabId);
        const node = model?.getNodeByBlockId(blockId);
        if (!model || node?.id == null) return { gone: true };
        if (atoms.activeTabId() !== tabId) return { ok: false, detail: "the window tab is not the active one" };
        const active = node.data?.activeBlockId ?? node.data?.blockId;
        if (active && active !== blockId) return { ok: false, detail: "the pane shows another member" };
        if (node.minimized) return { ok: false, detail: "the pane is still minimized" };
        if (model.magnifiedNodeId && model.magnifiedNodeId !== node.id) return { ok: false, detail: "another pane is magnified" };
        if (model.treeState.focusedNodeId !== node.id) return { ok: false, detail: "the pane is not the focused one" };
        return { ok: true };
    },
    handoff: (blockId) => revealInOtherWindow(blockId),
    flash: (blockId) => emitPaneLocate(blockId),
    notify(intent: Intent, reason: string) {
        // A block that is not in this window, asked for in this window only, is not a failure to tell the
        // user about: the caller has the answer (false) and may look elsewhere.
        if (intent.quiet || (intent.localOnly && reason === "not-found")) return;
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: `Couldn't show ${blockName(intent.blockId)}`,
            message: `${FAIL_TEXT[reason] ?? reason}`,
            timestamp: new Date().toISOString(),
            type: "info",
            expiration: Date.now() + 5000,
        });
    },
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    now: () => Date.now(),
};

const selectionTrace = createSelectionTrace();
const selection = createSelectionRunner(liveDeps, selectionTrace);
(globalThis as { __selectionTrace?: () => string[] }).__selectionTrace = () =>
    selectionTrace.entries().map((e) => `${new Date(e.at).toISOString()} ${e.line}`);

// The user switched window tab on their own while a request was in flight: that request ends, not
// retried. Started with the first request, not at import: this module is loaded early, inside import
// cycles, where the window's atoms are not yet there to read.
let watchingUserMoves = false;
function watchUserMoves(): void {
    if (watchingUserMoves) return;
    watchingUserMoves = true;
    createRoot(() => {
        createEffect(() => {
            const active = atoms.activeTabId();
            const state = selection.state();
            // A tab this runner asked for (an earlier request's, committed late) is not the user moving.
            if ("tabId" in state && state.tabId !== active && !requestedTabs.isOurs(active)) selection.userMoved();
        });
    });
}

/**
 * Show `blockId` wherever it is, and resolve with whether it was shown: in this window if it's here,
 * else in the window that shows it. The selection machine (`app/selection`) owns it from the request to
 * a verified result: the newest request wins, a layout that is slow to arrive is waited for, what
 * was done is read back (one retry), the locate flash plays once it is settled, and a failure says
 * why in a quiet notice. A renderer can't drive another window's layout, so srv (`block.reveal`) tells
 * THAT window's renderer to reveal it (`block:reveal`, reveal-block-events.ts) and this one raises
 * that window. Spec §4.3.
 */
export async function revealBlock(blockId: string, opts: RevealOptions = {}): Promise<SelectionResult> {
    watchUserMoves();
    const done = selectTrace.revealStarted(opts.source ?? "unknown", blockId);
    const result = await selection.select({
        blockId,
        source: opts.source,
        tabHint: opts.tabId,
        focusCaret: opts.focusCaret,
        localOnly: opts.localOnly,
        quiet: opts.quiet,
    });
    done(!isFailure(result) || result.reason === "superseded", isFailure(result) ? result.reason : undefined);
    return result;
}

/**
 * Hand the block to the window that shows it and wait for that window's answer (`block:revealed`, through srv):
 * it showed the pane, could not, or did not say in time.
 */
export async function revealInOtherWindow(blockId: string): Promise<{ outcome: RemoteOutcome; reason?: string; raise?: FocusWindowResult }> {
    try {
        const r = await RpcApi.BlockRevealCommand(TabRpcClient, { block_id: blockId, await_ack: true });
        if (r?.found && r.window_id) {
            const raise = await raiseWindowById(r.window_id);
            if (r.revealed === true) return { outcome: "revealed", raise };
            if (r.revealed === false) return { outcome: "failed", reason: r.reason, raise };
            return { outcome: "timeout", raise };
        }
    } catch (e) {
        // An older srv without block.reveal: fall back below.
        console.warn("[reveal-block] block.reveal failed", { blockId, error: String(e) });
    }
    const moved = await activateTabInOtherWorkspace(blockId);
    return moved ? { outcome: "revealed", raise: moved.raise } : { outcome: "not-found" };
}

/**
 * Raise the window whose `Window` oid is `windowId`, and say what the host made of it (`undefined`: no
 * such window instance, or the host could not be asked).
 */
async function raiseWindowById(windowId: string): Promise<FocusWindowResult | undefined> {
    try {
        const instances = await getApi().listWindowInstances();
        const instance = instances.find((i) => i.windowId === windowId);
        return instance?.label ? await getApi().focusWindow(instance.label) : undefined;
    } catch (e) {
        console.warn("[reveal-block] focusWindow failed", { windowId, error: String(e) });
        return undefined;
    }
}

/** The object at `oref`, cached or fetched; `null` when srv has no such row
 *  (`Client.windowids` can hold stale ids), so one bad id doesn't end a scan. */
export async function loadObject<T extends MuxObj>(otype: string, oid: string): Promise<T | null> {
    const oref = MOS.makeORef(otype, oid);
    try {
        return MOS.getObjectValue<T>(oref) ?? (await MOS.reloadMuxObject<T>(oref)) ?? null;
    } catch {
        return null;
    }
}

/** Fallback when srv can't name an open window for the block: activate its
 *  tab in the workspace that holds it and raise that workspace's window.
 *  Selects the window tab, not the pane or its tab. */
async function activateTabInOtherWorkspace(blockId: string): Promise<{ raise?: FocusWindowResult } | null> {
    // We need Tab.blockids to find which tab holds the block: layout models for
    // other windows' tabs aren't available in this renderer.
    // Walk the open windows, not WorkspaceService.ListWorkspaces: that skips
    // unnamed workspaces, which is what an ordinary or torn-off window holds.
    const ws = workspace();
    const client = await ClientService.GetClientData();
    for (const windowId of client?.windowids ?? []) {
        const win = await loadObject<MuxWindow>("window", windowId);
        if (!win?.workspaceid || win.workspaceid === ws?.oid) continue;
        const wsData = await loadObject<Workspace>("workspace", win.workspaceid);
        if (!wsData) continue;
        const allTabIds = [...(wsData.pinnedtabids ?? []), ...(wsData.tabids ?? [])];
        for (const tabId of allTabIds) {
            const tab = await loadObject<Tab>("tab", tabId);
            if (!tab?.blockids?.includes(blockId)) continue;
            await WorkspaceService.SetActiveTab(wsData.oid, tabId);
            return { raise: await raiseWindowById(windowId) };
        }
    }
    return null;
}
