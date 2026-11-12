// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "keeps running without a pane" toast and what its Shutdown button does (task #2520). The toast is one
 * notification that changes in place, under one id, as the shutdown goes:
 *
 *   keeps running  [Open] [Shutdown]
 *     -> (something would be lost)  "Shut down Quen?"  [Shutdown] [Keep running]   asked in the toast, no modal
 *     -> "Shutting down Quen…"  (no buttons, no expiry while it works; "2 of 3 done" for several)
 *     -> "Quen was shut down"  (expires as any toast)  |  "Couldn't shut down Quen" with the reason  [Retry]
 *
 * Shutdown is the person's own click, so it takes the path their Shut down elsewhere takes
 * (`ObjectService.ShutdownPreview` / `ShutdownAgent`, shut-down-agent.ts) with no agent-stop countdown. An agent
 * that was opened again, or is already stopped, by the time of the click is told so instead of failing.
 */

import { errorDetail } from "@/app/notification/toast-debug";
import { setNotificationAction } from "@/app/notification/usenotification";
import { isParkedMeta } from "@/app/store/agent-parked";
import { openAgentInActiveTab, sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { pushNotification } from "@/app/store/flash-notifications";
import * as services from "@/store/services";
import * as MOS from "@/store/mos";
import { quitNoticeMessage, type QuitSummary } from "../commands/global/quit";
import { needsConfirm, shutdownConfirmText, type ShutdownPreview } from "./shut-down-agent";
import {
    KEEPS_RUNNING_NOTE,
    keepsRunningTitle,
    notShutDownTitle,
    sentence,
    shutDownTitle,
    shuttingDownProgress,
    shuttingDownTitle,
} from "./keeps-running-text";

export type ParkedAgent = { agentId: string; name: string };

/** An agent the Shutdown goes to: still parked, with the block srv shuts it down by. */
type Target = ParkedAgent & { blockId: string };

/** How long a finished toast stays (a result with crons to look at stays longer, as the shut down notice does). */
const RESULT_MS = 6000;
const RESULT_WITH_CRONS_MS = 15_000;

const toastId = (agents: ParkedAgent[]) => `keeps-running:${agents.map((a) => a.agentId).join(",")}`;

type Face = Pick<NotificationType, "icon" | "title" | "message" | "type" | "expiration" | "actions" | "debug">;

/**
 * Put `face` on the agents' toast. A click on a toast action removes the toast right after its handler runs, so a
 * face raised by the handler itself is pushed after that, not before it.
 */
function show(agents: ParkedAgent[], face: Face): void {
    queueMicrotask(() => pushNotification({ id: toastId(agents), timestamp: new Date().toISOString(), ...face }));
}

/** A button of the toast: its handler is registered under a key of this toast. */
function button(agents: ParkedAgent[], name: string, label: string, run: () => void): NotificationActionType {
    const actionKey = `${toastId(agents)}:${name}`;
    setNotificationAction(actionKey, run);
    return { label, actionKey };
}

/** The agent's row as the window has it now. */
function rowOf(agentId: string) {
    return sharedAgentRuntime().rows().find((r) => r.agent_id === agentId);
}

/** What the click finds: who can be shut down, and who cannot and why. */
function sortOut(agents: ParkedAgent[]): { targets: Target[]; left: string[] } {
    const targets: Target[] = [];
    const left: string[] = [];
    for (const a of agents) {
        const row = rowOf(a.agentId);
        const meta = row?.block_id ? MOS.getObjectValue<MuxObj>(MOS.makeORef("block", row.block_id))?.meta : undefined;
        if (!row?.block_id || row.state === "stopped") left.push(`${a.name} is already stopped.`);
        else if (!isParkedMeta(meta)) left.push(`${a.name} was opened again; shut it down from its pane.`);
        else targets.push({ ...a, blockId: row.block_id });
    }
    return { targets, left };
}

async function previewOf(blockId: string): Promise<ShutdownPreview> {
    try {
        return (await services.ObjectService.ShutdownPreview(blockId)) as ShutdownPreview;
    } catch {
        // No preview: ask, as if something were running.
        return { working: true, claims: 0, crons_targeting: [] };
    }
}

/** The toast while the shutdown works: no buttons, no expiry; several agents are counted as they finish. */
function working(all: ParkedAgent[], targets: Target[], finished: number): void {
    show(all, {
        icon: "fa-power-off",
        title: shuttingDownTitle(targets.map((t) => t.name)),
        message: targets.length > 1 ? shuttingDownProgress(finished, targets.length) : "",
        type: "info",
    });
}

/** Shut down every target at once, counting them in the toast as each finishes, then say how it went. */
async function shutDown(all: ParkedAgent[], targets: Target[], left: string[]): Promise<void> {
    const names = targets.map((t) => t.name);
    let finished = 0;
    working(all, targets, 0);
    const outcomes = await Promise.all(
        targets.map(async (t) => {
            try {
                const summary = (await services.ObjectService.ShutdownAgent(t.blockId, false)) as QuitSummary;
                return { target: t, summary };
            } catch (e) {
                return { target: t, error: e };
            } finally {
                finished += 1;
                if (targets.length > 1 && finished < targets.length) working(all, targets, finished);
            }
        })
    );
    const failed = outcomes.filter((o) => "error" in o);
    const ok = outcomes.filter((o) => "summary" in o);
    if (failed.length === 0) {
        const crons = ok.some((o) => (o.summary?.crons_targeting?.length ?? 0) > 0);
        const detail = ok.length === 1 && ok[0].summary ? quitNoticeMessage(ok[0].summary) : "Conversations kept — reopen an agent to resume it.";
        show(all, {
            icon: "fa-power-off",
            title: shutDownTitle(names),
            message: [detail, ...left].join(" "),
            type: "info",
            expiration: Date.now() + (crons ? RESULT_WITH_CRONS_MS : RESULT_MS),
        });
        return;
    }
    const why = (o: (typeof failed)[number]) => `${o.target.name}: ${o.error instanceof Error ? o.error.message : String(o.error)}`;
    const retry = failed.map((o) => o.target);
    const first = failed[0];
    show(all, {
        icon: "fa-triangle-exclamation",
        title: notShutDownTitle(retry.map((t) => t.name), targets.length),
        message: [...failed.map((o) => sentence(why(o))), ...(ok.length ? [`${ok.map((o) => o.target.name).join(", ")} ${ok.length === 1 ? "was" : "were"} shut down.`] : []), ...left].join(" "),
        type: "error",
        actions: [button(all, "retry", "Retry", () => void ask(all, retry, left))],
        debug: { blockId: first.target.blockId, agentName: first.target.name, code: "agent-shutdown", detail: { stopBackground: false, ...errorDetail(first.error) } },
    });
}

/** Ask in the toast when a shutdown would end a turn or lose claimed work, else go ahead. */
async function ask(all: ParkedAgent[], targets: Target[], left: string[]): Promise<void> {
    // The click removed the toast: say at once that it is working, until the question or the shutdown replaces it.
    working(all, targets, 0);
    const previews = await Promise.all(targets.map((t) => previewOf(t.blockId)));
    const risky = targets.map((t, i) => ({ t, p: previews[i] })).filter(({ p }) => needsConfirm(p));
    if (risky.length === 0) return shutDown(all, targets, left);
    const lines = risky.map(({ t, p }) => (risky.length === 1 ? shutdownConfirmText(t.name, p).description : `${t.name}: ${shutdownConfirmText(t.name, p).description}`));
    show(all, {
        icon: "fa-power-off",
        title: targets.length === 1 ? shutdownConfirmText(targets[0].name, previews[0]).title : `Shut down ${targets.map((t) => t.name).join(", ")}?`,
        message: lines.join(" "),
        type: "warning",
        actions: [button(all, "confirm", "Shutdown", () => void shutDown(all, targets, left)), button(all, "keep", "Keep running", () => {})],
    });
}

/** The Shutdown click: see who is still parked, then ask or shut down. */
function onShutdown(all: ParkedAgent[]): void {
    const { targets, left } = sortOut(all);
    if (targets.length === 0) {
        show(all, {
            icon: "fa-power-off",
            title: left.length === 1 ? left[0] : "Nothing to shut down",
            message: left.length === 1 ? "" : left.join(" "),
            type: "info",
            expiration: Date.now() + RESULT_MS,
        });
        return;
    }
    void ask(all, targets, left);
}

/** The note after a close that parked these agents. The Open brings back the first one; Shutdown ends all of them. */
export function noteKeepsRunning(agents: ParkedAgent[]): void {
    if (agents.length === 0) return;
    const first = agents[0];
    const actionKey = `agent-parked-open:${first.agentId}`;
    setNotificationAction(actionKey, () => void openAgentInActiveTab({ agent_id: first.agentId, name: first.name }));
    pushNotification({
        id: toastId(agents),
        icon: "fa-circle-play",
        title: keepsRunningTitle(agents.map((a) => a.name)),
        message: KEEPS_RUNNING_NOTE,
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + 8000,
        actions: [{ label: "Open", actionKey }, button(agents, "shutdown", "Shutdown", () => onShutdown(agents))],
    });
}
