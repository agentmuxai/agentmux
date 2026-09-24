// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Renderer side of the OS notification system —
 * docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §3.
 *
 * The renderer is the authority for three things the srv Router can't see:
 *
 * 1. **Pane events** — turn outcome and AskUserQuestion waiting state exist
 *    only in the agent-pane reducer. Forwarded as `notify.emit`.
 * 2. **Focus** — whether this window is foreground and which block is
 *    focused in it. Forwarded as `notify.focus` on change, plus a heartbeat so
 *    a reconnected WS (new srv-side conn id) is re-registered promptly.
 * 3. **Activation** — when the user clicks a toast, the Router publishes
 *    `notification:activate`; the window that holds the block switches to its
 *    tab, focuses the pane and raises itself.
 *
 * All failures are swallowed: notifications are best-effort and must never
 * break the pane or window init.
 */

import { createEffect, createRoot, on } from "solid-js";

import { addEventListener as addPaneListener } from "@/app/store/agent-pane-state-store";
import type { AgentPaneEvent } from "@/app/store/agent-pane-state/types";
import { focusManager } from "@/app/store/focusManager";
import { getApi, getSettingsKeyAtom, MOS, setActiveTab, workspace } from "@/app/store/global";
import { muxEventSubscribe } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import type { NotifyPaneEvent } from "@/types/rpc/NotifyPaneEvent";

export const EVENT_NOTIFICATION_ACTIVATE = "notification:activate";
export const EVENT_NOTIFICATION_STATE = "notification:state";

/** Clicks older than this are ignored (stale replay / slow window). */
const ACTIVATION_MAX_AGE_MS = 15_000;
const FOCUS_HEARTBEAT_MS = 20_000;

/**
 * Pure mapping from a reducer event to what the Router cares about.
 * `stopped` / `interrupted` are user-initiated — nothing to notify about.
 * `waiting-ended` only resolves on "submitted": "closed" fires when the pane
 * unmounts (e.g. its window closed into background mode) while the agent is
 * still blocked on the question, which is exactly when the toast matters.
 */
export function paneEventToNotify(ev: AgentPaneEvent): { event: NotifyPaneEvent; question?: string } | null {
    switch (ev.type) {
        case "turn-started":
            return { event: "turn_started" };
        case "turn-ended":
            if (ev.outcome === "completed") return { event: "turn_completed" };
            if (ev.outcome === "errored") return { event: "turn_errored" };
            return null;
        case "waiting-for-input":
            return { event: "input_waiting", question: ev.question };
        case "waiting-ended":
            return ev.reason === "submitted" ? { event: "input_resolved" } : null;
        default:
            return null;
    }
}

function emit(blockId: string, ev: AgentPaneEvent): void {
    const m = paneEventToNotify(ev);
    if (!m) return;
    RpcApi.NotifyEmitCommand(TabRpcClient, { block_id: blockId, event: m.event, question: m.question }).catch(() => {});
}

async function raiseThisWindow(): Promise<void> {
    try {
        const label = await getApi().getWindowLabel();
        if (label) await getApi().focusWindow(label);
    } catch {
        /* best-effort */
    }
}

/**
 * Focus `blockId` if it lives in THIS window's workspace. Unlike
 * `focusBlock()`'s fast path this also finds tabs whose layout model hasn't
 * been created yet (never visited in this window), via `Tab.blockids`.
 * Returns whether the block was found here.
 */
export async function activateBlockLocally(blockId: string): Promise<boolean> {
    const ws = workspace();
    if (!ws) return false;
    const tabIds = [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
    for (const tabId of tabIds) {
        const oref = MOS.makeORef("tab", tabId);
        const tab = MOS.getObjectValue<Tab>(oref) ?? (await MOS.reloadMuxObject<Tab>(oref));
        if (!tab?.blockids?.includes(blockId)) continue;
        await setActiveTab(tabId);
        // The layout model for a not-yet-visited tab exists only after the
        // switch renders — retry briefly.
        for (let i = 0; i < 10; i++) {
            const node = getLayoutModelForTabById(tabId)?.getNodeByBlockId(blockId);
            if (node?.id != null) {
                getLayoutModelForTabById(tabId)!.focusNode(node.id);
                break;
            }
            await new Promise((r) => setTimeout(r, 50));
        }
        await raiseThisWindow();
        return true;
    }
    return false;
}

/** Count of on-screen "needs you" notifications in a Router snapshot. */
export function attentionCount(data: unknown): number {
    const a = (data as { attention?: unknown[] } | undefined)?.attention;
    return Array.isArray(a) ? a.length : 0;
}

/**
 * Taskbar badge + flash (spec Phase 4). Each window badges its own taskbar
 * button; the host ignores the call on platforms without an implementation.
 */
async function applyTaskbarAttention(count: number): Promise<void> {
    try {
        const label = await getApi().getWindowLabel();
        if (!label) return;
        const { invokeCommand } = await import("@/app/platform/ipc");
        await invokeCommand("set_taskbar_attention", { label, count });
    } catch {
        /* older host without the verb */
    }
}

let installed = false;

export function installOsNotifyBridge(): () => void {
    if (installed) return () => {};
    installed = true;

    const paneUnsub = addPaneListener((blockId, event) => {
        try {
            emit(blockId, event);
        } catch {
            /* never break the reducer's listener fan-out */
        }
    });

    let lastSent = "";
    const report = (force = false) => {
        const windowFocused = makeWindowFocusSignal()();
        const blockId = focusManager.blockFocusAtom() ?? undefined;
        const key = `${windowFocused}|${blockId ?? ""}`;
        if (!force && key === lastSent) return;
        lastSent = key;
        RpcApi.NotifyFocusCommand(TabRpcClient, { window_focused: windowFocused, block_id: blockId }).catch(() => {
            lastSent = ""; // retry on next tick
        });
    };
    const disposeRoot = createRoot((dispose) => {
        const focused = makeWindowFocusSignal();
        createEffect(on([focused, focusManager.blockFocusAtom], () => report()));
        return dispose;
    });
    const heartbeat = setInterval(() => report(true), FOCUS_HEARTBEAT_MS);

    const activateUnsub = muxEventSubscribe({
        eventType: EVENT_NOTIFICATION_ACTIVATE,
        handler: (event) => {
            const d = event.data as { block_id?: string; at_ms?: number } | undefined;
            if (!d?.block_id) return;
            if (typeof d.at_ms === "number" && Date.now() - d.at_ms > ACTIVATION_MAX_AGE_MS) return;
            void activateBlockLocally(d.block_id).catch(() => {});
        },
    });

    let lastCount = -1;
    let latestCount = 0;
    const taskbarOn = () => (getSettingsKeyAtom("notify:taskbar:attention" as any)() as boolean | undefined) ?? true;
    const syncTaskbar = () => {
        const count = taskbarOn() ? latestCount : 0;
        if (count === lastCount) return;
        lastCount = count;
        void applyTaskbarAttention(count);
    };
    const stateUnsub = muxEventSubscribe({
        eventType: EVENT_NOTIFICATION_STATE,
        handler: (event) => {
            latestCount = attentionCount(event.data);
            syncTaskbar();
        },
    });
    const disposeTaskbarRoot = createRoot((dispose) => {
        createEffect(on(taskbarOn, () => syncTaskbar(), { defer: true }));
        return dispose;
    });

    // A click that arrived while no window was open (background mode): the
    // launcher opened this window; pick the target up once.
    void RpcApi.NotifyTakeActivationCommand(TabRpcClient)
        .then((r) => (r?.block_id ? activateBlockLocally(r.block_id) : false))
        .catch(() => {});

    return () => {
        paneUnsub();
        disposeRoot();
        clearInterval(heartbeat);
        activateUnsub();
        stateUnsub();
        disposeTaskbarRoot();
        installed = false;
    };
}
