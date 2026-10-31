// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shut down an agent from My agents or its pane's menu: the one act that ends it
 * (SPEC_CLOSING_PARKS_AGENTS_SHUTDOWN_IS_EXPLICIT §4.4). Closing a pane, tab or window
 * only detaches it.
 *
 * srv does the work (`ObjectService.ShutdownAgent`, the same teardown as `/shutdown`).
 * It asks first only when something would be lost: a turn in progress, work the agent
 * has claimed, or cron jobs that target it. An idle agent shuts down on one click, since
 * its conversation is kept.
 */

import { openModal } from "@/app/store/modalmodel";
import { pushNotification } from "@/app/store/flash-notifications";
import * as services from "@/store/services";
import { quitNoticeMessage, type QuitSummary } from "../commands/global/quit";
import { beginShutdownLog, endShutdownLog } from "./shutdown-log";
import { ShutdownConfirm } from "./shutdown-confirm";

/** What srv says a shutdown would end (`self_quit::ShutdownPreview`). */
export type ShutdownPreview = { working: boolean; claims: number; crons_targeting: string[] };

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Whether the shutdown asks first. */
export function needsConfirm(p: ShutdownPreview): boolean {
    return p.working || p.claims > 0 || p.crons_targeting.length > 0;
}

/** The confirm's title and what ends, named. */
export function shutdownConfirmText(name: string, p: ShutdownPreview): { title: string; description: string } {
    const parts: string[] = [];
    if (p.working) parts.push("It is in the middle of a turn, which ends.");
    if (p.claims > 0) parts.push(`${plural(p.claims, "work item it has claimed goes", "work items it has claimed go")} back to the pool.`);
    if (p.crons_targeting.length > 0) {
        parts.push(
            `${plural(p.crons_targeting.length, "cron job targets", "cron jobs target")} it (${p.crons_targeting.join(", ")}); ` +
                `${p.crons_targeting.length === 1 ? "it is" : "they are"} kept.`
        );
    }
    parts.push("The conversation is kept.");
    return { title: `Shut down ${name}?`, description: parts.join(" ") };
}

async function shutDown(blockId: string, name: string, stopBackground: boolean): Promise<void> {
    beginShutdownLog(blockId);
    try {
        const summary = (await services.ObjectService.ShutdownAgent(blockId, stopBackground)) as QuitSummary;
        if (summary?.status === "already_quitting") return;
        pushNotification({
            icon: "fa-power-off",
            title: `${summary?.agent || name} shut down`,
            message: quitNoticeMessage(summary),
            timestamp: new Date().toISOString(),
            type: "info",
            expiration: Date.now() + (summary?.crons_targeting?.length ? 15_000 : 6_000),
        });
    } catch (e) {
        endShutdownLog(blockId);
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: `Couldn't shut down ${name}`,
            message: e instanceof Error ? e.message : String(e),
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 8000,
        });
    }
}

/** Shut down the agent running in `blockId`, asking first when something would be lost. */
export async function requestShutdown(blockId: string, name: string): Promise<void> {
    let preview: ShutdownPreview;
    try {
        preview = (await services.ObjectService.ShutdownPreview(blockId)) as ShutdownPreview;
    } catch {
        // No preview: ask, as if something were running.
        preview = { working: true, claims: 0, crons_targeting: [] };
    }
    if (!needsConfirm(preview)) {
        await shutDown(blockId, name, false);
        return;
    }
    openModal(ShutdownConfirm, {
        ...shutdownConfirmText(name, preview),
        onConfirm: (stopBackground: boolean) => void shutDown(blockId, name, stopBackground),
    });
}
