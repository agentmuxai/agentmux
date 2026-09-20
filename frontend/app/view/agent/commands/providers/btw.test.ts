// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { SlashCommandContext } from "../types";
import { btwCommand } from "./btw";

function makeCtx(askSideQuestion: (question: string) => Promise<{ requestId: string }>): SlashCommandContext {
    return {
        blockId: "block-1",
        provider: () => undefined,
        block: () => undefined,
        documentNodes: () => [],
        log: vi.fn(),
        setAuthUrl: vi.fn(),
        notifyControllerHealthy: vi.fn(),
        clearAuthFailure: vi.fn(),
        forceControllerRefresh: vi.fn().mockResolvedValue(true),
        isTurnActive: () => false,
        deferControllerRefreshUntilIdle: vi.fn(),
        beginRecoveryFlow: vi.fn(),
        endRecoveryFlow: vi.fn(),
        isCancelled: () => false,
        resetCancelled: vi.fn(),
        openPicker: vi.fn(),
        openHelp: vi.fn(),
        quickFork: vi.fn(),
        askSideQuestion,
    };
}

describe("/btw", () => {
    it("calls ctx.askSideQuestion with the trimmed question and returns ok", async () => {
        const askSideQuestion = vi.fn().mockResolvedValue({ requestId: "req-1" });
        const ctx = makeCtx(askSideQuestion);

        const result = await btwCommand.handler(ctx, "  what does this function do?  ");

        expect(askSideQuestion).toHaveBeenCalledOnce();
        expect(askSideQuestion).toHaveBeenCalledWith("what does this function do?");
        expect(result).toEqual({ kind: "ok" });
    });

    it("returns an error without calling ctx.askSideQuestion when the argument is blank", async () => {
        const askSideQuestion = vi.fn().mockResolvedValue({ requestId: "req-1" });
        const ctx = makeCtx(askSideQuestion);

        const result = await btwCommand.handler(ctx, "   ");

        expect(askSideQuestion).not.toHaveBeenCalled();
        expect(result).toEqual({ kind: "error", message: "/btw: a question is required" });
    });

    it("returns an error when ctx.askSideQuestion rejects", async () => {
        const askSideQuestion = vi.fn().mockRejectedValue(new Error("backend not available"));
        const ctx = makeCtx(askSideQuestion);

        const result = await btwCommand.handler(ctx, "why is the sky blue?");

        expect(result).toEqual({ kind: "error", message: "/btw: backend not available" });
    });
});
