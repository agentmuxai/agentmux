// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /quit (alias /exit) — end this agent gracefully and close its own tab.
 * SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.
 *
 * The server does the work (`ObjectService.QuitAgent` → `sagas/self_quit.rs`):
 * releases the agent's work claims, stops the shells it started, shuts it
 * down gracefully and closes only its tab; sibling tabs keep running. The
 * conversation is kept, so reopening the agent resumes it.
 *
 * `/exit` is an alias because Claude Code users type it by habit; without it
 * the text went to the model as a question. `/q` is deliberately NOT an alias:
 * a one-letter command that ends an agent is too easy to hit by accident.
 */

import { pushNotification } from "@/app/store/flash-notifications";
import { beginShutdownLog, endShutdownLog } from "@/app/view/agent/shutdown/shutdown-log";
import * as services from "@/store/services";
import type { SlashCommand, SlashResult } from "../types";

/** The server's answer (`sagas/self_quit.rs` `QuitSummary`). */
export type QuitSummary = {
    status: "quit" | "already_quitting";
    agent: string;
    released_claims: number;
    stopped_shells: number;
    crons_targeting: string[];
};

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** The notice shown after the tab is gone (spec §5.3). */
export function quitNoticeMessage(s: QuitSummary): string {
    const parts = ["Conversation kept — reopen the agent to resume it."];
    if (s.released_claims > 0) parts.push(`Released ${plural(s.released_claims, "work claim", "work claims")}.`);
    if (s.stopped_shells > 0) parts.push(`Stopped ${plural(s.stopped_shells, "shell", "shells")}.`);
    if (s.crons_targeting.length > 0) {
        parts.push(
            `${plural(s.crons_targeting.length, "cron job still targets", "cron jobs still target")} it ` +
                `(${s.crons_targeting.join(", ")}); pause or delete them if you meant to stop everything.`
        );
    }
    return parts.join(" ");
}

export const quitCommand: SlashCommand = {
    name: "quit",
    aliases: ["exit"],
    category: "session",
    description: "End this agent gracefully and close its tab (conversation kept; reopen to resume)",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        // The pane shows what srv stops while it winds the agent down (§5.5).
        beginShutdownLog(ctx.blockId);
        let summary: QuitSummary;
        try {
            summary = (await services.ObjectService.QuitAgent(ctx.blockId)) as QuitSummary;
        } catch (e) {
            endShutdownLog(ctx.blockId);
            return { kind: "error", message: `Couldn't quit: ${e instanceof Error ? e.message : String(e)}` };
        }
        if (summary?.status === "already_quitting") {
            return { kind: "ok", message: "already quitting" };
        }
        pushNotification({
            icon: "fa-right-from-bracket",
            title: `${summary?.agent || "Agent"} quit`,
            message: quitNoticeMessage(summary),
            timestamp: new Date().toISOString(),
            type: "info",
            expiration: Date.now() + (summary?.crons_targeting?.length ? 15_000 : 6_000),
        });
        return { kind: "ok" };
    },
};
