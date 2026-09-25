// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const quitAgent = vi.fn();
const pushNotification = vi.fn();
vi.mock("@/store/services", () => ({ ObjectService: { QuitAgent: (...a: unknown[]) => quitAgent(...a) } }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: (n: unknown) => pushNotification(n) }));
const beginShutdownLog = vi.fn();
const endShutdownLog = vi.fn();
vi.mock("@/app/view/agent/shutdown/shutdown-log", () => ({
    beginShutdownLog: (id: string) => beginShutdownLog(id),
    endShutdownLog: (id: string) => endShutdownLog(id),
}));

import type { SlashCommandContext } from "../types";
import { quitCommand, quitNoticeMessage, type QuitSummary } from "./quit";

const ctx = { blockId: "block-1" } as unknown as SlashCommandContext;
const summary = (over: Partial<QuitSummary> = {}): QuitSummary => ({
    status: "quit",
    agent: "Camper",
    released_claims: 0,
    stopped_shells: 0,
    crons_targeting: [],
    ...over,
});

describe("/quit", () => {
    beforeEach(() => {
        quitAgent.mockReset();
        pushNotification.mockReset();
        beginShutdownLog.mockReset();
        endShutdownLog.mockReset();
    });

    it("is /quit with the /exit alias, never /q, and only on a launched agent", () => {
        // The registry resolves aliases generically (registry.ts `lookup`).
        expect(quitCommand.name).toBe("quit");
        expect(quitCommand.aliases).toEqual(["exit"]);
        expect(quitCommand.availability).toBe("any-agent");
    });

    it("asks the server to quit this block once and shows the notice", async () => {
        quitAgent.mockResolvedValue(summary());
        expect(await quitCommand.handler(ctx, "")).toEqual({ kind: "ok" });
        expect(quitAgent).toHaveBeenCalledTimes(1);
        expect(quitAgent).toHaveBeenCalledWith("block-1");
        expect(pushNotification).toHaveBeenCalledWith(expect.objectContaining({ title: "Camper quit", type: "info" }));
        expect(beginShutdownLog).toHaveBeenCalledWith("block-1"); // the pane shows the shutdown log (§5.5)
    });

    it("a quit already under way is fine and shows no second notice", async () => {
        quitAgent.mockResolvedValue(summary({ status: "already_quitting" }));
        expect(await quitCommand.handler(ctx, "")).toEqual({ kind: "ok", message: "already quitting" });
        expect(pushNotification).not.toHaveBeenCalled();
    });

    it("a server failure is a command error, and nothing claims the agent quit", async () => {
        quitAgent.mockRejectedValue(new Error("QuitAgent: block not found: block-1"));
        const r = await quitCommand.handler(ctx, "");
        expect(r).toEqual({ kind: "error", message: "Couldn't quit: QuitAgent: block not found: block-1" });
        expect(endShutdownLog).toHaveBeenCalledWith("block-1"); // nothing to show: the pane stays as it was
        expect(pushNotification).not.toHaveBeenCalled();
    });
});

describe("quit notice", () => {
    it("says the conversation is kept, and only mentions what happened", () => {
        expect(quitNoticeMessage(summary())).toBe("Conversation kept — reopen the agent to resume it.");
        const m = quitNoticeMessage(summary({ released_claims: 2, stopped_shells: 1, crons_targeting: ["nightly"] }));
        expect(m).toContain("Released 2 work claims.");
        expect(m).toContain("Stopped 1 shell.");
        expect(m).toContain("1 cron job still targets it (nightly)");
    });
});
