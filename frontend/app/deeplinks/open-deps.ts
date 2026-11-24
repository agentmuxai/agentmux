// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The executor's hands in a window (`open.ts` `ExecutorDeps`). A person's open goes through the window's own
 * `openOrFocusPaneIdByView`; an agent's is found in or added to its own tab without moving focus. Loaded on the first
 * open, so the parser and the registry stay free of the store.
 */

import { openOrFocusPaneIdByView } from "@/app/store/block-component-registry";
import { setBlockMeta } from "@/app/store/block-meta";
import { atoms, MOS, workspace } from "@/app/store/global";
import { openLink } from "@/app/store/misc-utils";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { t } from "@/app/i18n/i18n";
import { nextAnnouncement } from "@/app/notification/toast-announcer";
import { emitPaneLocate } from "@/app/util/pane-locate";
import { paneIdentity } from "@/app/util/pane-identity";
import { findTabHolding, showBlockWithoutFocus } from "@/app/util/reveal-block";
import type { PanePlan } from "./destination-meta";
import type { ExecutorDeps, PaneFacts, PersonInput } from "./open";

/** Panes the person has pressed or typed in, and when the person last typed: read from the document, once. */
const used = new Set<string>();
const input: PersonInput = { lastKeyAt: Number.NEGATIVE_INFINITY, recording: false, composing: false };
let watching = false;

function watchPerson(): void {
    if (watching || typeof document === "undefined") return;
    watching = true;
    const mark = (ev: Event) => {
        const el = ev.target instanceof Element ? ev.target.closest("[data-blockid]") : null;
        const id = el?.getAttribute("data-blockid");
        if (id) used.add(id);
    };
    document.addEventListener("pointerdown", mark, true);
    document.addEventListener(
        "keydown",
        (ev) => {
            input.lastKeyAt = performance.now();
            mark(ev);
        },
        true
    );
    document.addEventListener("compositionstart", () => (input.composing = true), true);
    document.addEventListener("compositionend", () => (input.composing = false), true);
}

function blockOf(blockId: string): Block | undefined {
    return MOS.getObjectValue<Block>(MOS.makeORef("block", blockId));
}

function tabOfSync(blockId: string): string | undefined {
    const ws = workspace();
    for (const tabId of [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])]) {
        if (MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabId))?.blockids?.includes(blockId)) return tabId;
    }
    return undefined;
}

async function blocksOfTab(tabId: string): Promise<string[]> {
    const ref = MOS.makeORef("tab", tabId);
    const tab = MOS.getObjectValue<Tab>(ref) ?? (await MOS.reloadMuxObject<Tab>(ref));
    return tab?.blockids ?? [];
}

function identityOf(plan: PanePlan): string | undefined {
    return paneIdentity(plan.view, plan.meta);
}

export function windowDeps(): ExecutorDeps {
    watchPerson();
    return {
        now: () => performance.now(),
        sleep: (ms) => new Promise((r) => setTimeout(r, ms)),
        translate: (key) => t(key),
        async openForPerson(plan, how) {
            const def: BlockDef = { meta: plan.meta as MetaType };
            if (how.floating || how.newPane) {
                const res = await RpcApi.PaneOpenCommand(
                    TabRpcClient,
                    { view: plan.view, meta: plan.meta as MetaType, tab_id: atoms.activeTabId(), ...(how.floating ? { floating: true } : { new: true }) },
                    {}
                );
                return res?.block_id;
            }
            return openOrFocusPaneIdByView(plan.view, def, plan.focusMeta as MetaType);
        },
        async findPane(plan, tabId) {
            const tab = tabId ?? atoms.activeTabId();
            const identity = identityOf(plan);
            for (const blockId of await blocksOfTab(tab)) {
                const meta = blockOf(blockId)?.meta;
                if (meta?.view === plan.view && (identity == null || paneIdentity(plan.view, meta) === identity)) return blockId;
            }
            return null;
        },
        setMeta: (blockId, meta) => setBlockMeta(blockId, meta as MetaType),
        async showWithoutFocus(blockId) {
            await showBlockWithoutFocus(blockId, { tabId: (await findTabHolding(blockId)) ?? undefined });
        },
        async openWithoutFocus(plan, at) {
            const res = await RpcApi.PaneOpenCommand(
                TabRpcClient,
                {
                    view: plan.view,
                    meta: plan.meta as MetaType,
                    focus: false,
                    new: true,
                    ...(at.tabId ? { tab_id: at.tabId } : {}),
                    ...(at.besideBlockId ? { split_reference_block_id: at.besideBlockId, split_direction: "right" } : {}),
                },
                {}
            );
            return res?.block_id;
        },
        async stackOnto(plan, blockId) {
            const res = await RpcApi.PaneOpenCommand(
                TabRpcClient,
                { view: plan.view, meta: plan.meta as MetaType, focus: false, new: true, stack_onto_block_id: blockId },
                {}
            );
            return res?.block_id;
        },
        paneFacts(blockId): PaneFacts | undefined {
            const meta = blockOf(blockId)?.meta;
            if (!meta) return undefined;
            const view = String(meta.view ?? "");
            // Read as the strictest case: an agent pane, a terminal and an editor are each taken to hold work.
            return {
                view,
                agentRunning: view === "agent",
                termProcess: view === "term",
                editorDirty: view === "editor",
                usedByPerson: used.has(blockId),
            };
        },
        tabOf: tabOfSync,
        tabExists: (tabId) => {
            const ws = workspace();
            return [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])].includes(tabId);
        },
        async settingExists(id) {
            const { SETTINGS_INDEX } = await import("@/app/view/settings/settings-index");
            return SETTINGS_INDEX.some((e) => e.id === id);
        },
        async vaultEntryExists(linkId) {
            const { refreshVault, vaultEntries } = await import("@/app/view/vault/vault-store");
            await refreshVault();
            return (vaultEntries() ?? []).some((e) => e.link_id === linkId);
        },
        locate: (blockId) => emitPaneLocate(blockId, { focus: false }),
        announce: announcePolite,
        personInput: () => ({ ...input }),
        openExternal: (url) => openLink(url),
        async openBrowserPane(url, floating) {
            const res = await RpcApi.PaneOpenCommand(TabRpcClient, { view: "browser", url, tab_id: atoms.activeTabId(), ...(floating ? { floating: true } : {}) }, {});
            return res?.block_id;
        },
    };
}

let region: HTMLElement | undefined;

/** The executor's polite live region: one, made on first use, read by screen readers, never focused (G13). */
export function announcePolite(text: string): void {
    if (typeof document === "undefined") return;
    if (!region || !region.isConnected) {
        region = document.createElement("div");
        region.className = "toast-announcer";
        region.setAttribute("role", "status");
        region.setAttribute("aria-live", "polite");
        region.setAttribute("aria-atomic", "true");
        region.setAttribute("data-testid", "deeplink-announcer");
        document.body.appendChild(region);
    }
    // The same words twice are read twice.
    region.textContent = nextAnnouncement(region.textContent ?? "", text);
}
