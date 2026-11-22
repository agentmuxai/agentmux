// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's side of an agent's deep-link open (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §4.2, R3,
 * G9): srv's guard lets the open through and publishes `deeplink:open`; the window it names carries it out with the
 * executor (`open.ts` `handleAgentOpen`) and answers `deeplink.open.done` with the outcome. An event past its
 * expiry, or for another window, is not carried out.
 */

import { atoms, workspace } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DeepLinkOpenEvent } from "@/types/rpc/DeepLinkOpenEvent";
import type { DeepLinkOutcome } from "@/types/rpc/DeepLinkOutcome";
import type { OpenOutcome, OpenTarget } from "./open-types";

/** What the bridge reads of the window and does, so it can be tested without one. */
export interface BridgeDeps {
    now(): number;
    /** This window's id. */
    windowId(): string | undefined;
    /** Whether `tabId` is a tab of this window. */
    holdsTab(tabId: string): boolean;
    /** Whether the person last used this window (it has focus). */
    lastUsed(): boolean;
    open(req: { link: string; target: OpenTarget; agentId: string; pane?: string; requestId: string }): Promise<OpenOutcome>;
    answer(requestId: string, outcome: DeepLinkOutcome): Promise<void>;
}

/** srv's target, as the executor names it: srv resolved a tab or a window target into `tab_id` already. */
export function targetOf(ev: Pick<DeepLinkOpenEvent, "target" | "tab_id" | "caller_block_id">): OpenTarget {
    if (ev.target.startsWith("pane:")) return { kind: "pane", blockId: ev.target.slice("pane:".length) };
    if (ev.target === "new") return { kind: "new" };
    // Beside the caller's conversation; a caller with no pane opens in the tab srv chose for it.
    return !ev.caller_block_id && ev.tab_id ? { kind: "tab", tabId: ev.tab_id } : { kind: "beside" };
}

/** Whether this window is the one to carry `ev` out. */
export function isMine(ev: DeepLinkOpenEvent, deps: Pick<BridgeDeps, "now" | "windowId" | "holdsTab" | "lastUsed">): boolean {
    if (Number(ev.expires_at_ms) <= deps.now()) return false;
    if (ev.window_id) return ev.window_id === deps.windowId();
    if (ev.tab_id) return deps.holdsTab(ev.tab_id);
    return deps.lastUsed();
}

/** The executor's outcome as srv records it. */
export function wireOutcome(out: OpenOutcome): DeepLinkOutcome {
    return {
        outcome: out.outcome,
        label: out.label.slice(0, 200),
        ...(out.blockId ? { block_id: out.blockId } : {}),
        ...(out.place ? { place: out.place } : {}),
        ...(out.reason ? { reason_code: out.reason.code, reason: out.reason.text } : {}),
        ...(out.remedy ? { remedy: out.remedy.text } : {}),
    };
}

/** Carries `ev` out when it is this window's, and answers srv. Each request is carried out once. */
export async function carryOut(ev: DeepLinkOpenEvent, deps: BridgeDeps, seen: Set<string>): Promise<void> {
    if (seen.has(ev.request_id) || !isMine(ev, deps)) return;
    seen.add(ev.request_id);
    let out: OpenOutcome;
    try {
        out = await deps.open({ link: ev.link, target: targetOf(ev), agentId: ev.agent_id, pane: ev.caller_block_id, requestId: ev.request_id });
    } catch (e) {
        out = { outcome: "refused", label: "", reason: { code: "not_open", text: e instanceof Error ? e.message.slice(0, 200) : "The pane couldn't be opened." } };
    }
    await deps.answer(ev.request_id, wireOutcome(out)).catch(() => {});
}

let started = false;

/** Carry out the agents' deep-link opens srv sends to this window. Call once per window. */
export function startDeepLinkOpens(deps?: BridgeDeps): void {
    if (started) return;
    started = true;
    const seen = new Set<string>();
    const d = deps ?? windowBridgeDeps();
    onMuxEvent(WpsEvent.DeepLinkOpen, (ev: DeepLinkOpenEvent) => void carryOut(ev, d, seen));
}

function windowBridgeDeps(): BridgeDeps {
    return {
        now: () => Date.now(),
        windowId: () => atoms.muxWindow()?.oid,
        holdsTab: (tabId) => {
            const ws = workspace();
            return [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])].includes(tabId);
        },
        lastUsed: () => typeof document !== "undefined" && document.hasFocus(),
        async open(req) {
            const { handleAgentOpen } = await import("./open");
            return handleAgentOpen({ link: req.link, target: req.target, source: { kind: "agent", agentId: req.agentId, pane: req.pane, requestId: req.requestId }, requestId: req.requestId });
        },
        async answer(requestId, outcome) {
            await RpcApi.DeepLinkOpenDoneCommand(TabRpcClient, { request_id: requestId, outcome });
        },
    };
}
