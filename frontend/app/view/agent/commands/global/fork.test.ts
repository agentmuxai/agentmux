// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { forkCommand } from "./fork";
import type { SlashCommandContext } from "../types";

function makeCtx(quickFork: () => Promise<boolean>): SlashCommandContext {
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
        quickFork,
        askSideQuestion: vi.fn(),
    };
}

describe("/fork", () => {
    it("returns ok and delegates to ctx.quickFork() when the fork launches", async () => {
        const quickFork = vi.fn().mockResolvedValue(true);
        const ctx = makeCtx(quickFork);

        const result = await forkCommand.handler(ctx, "");

        expect(quickFork).toHaveBeenCalledOnce();
        expect(result).toEqual({ kind: "ok" });
    });

    it("returns an error when ctx.quickFork() reports the fork did not launch", async () => {
        const quickFork = vi.fn().mockResolvedValue(false);
        const ctx = makeCtx(quickFork);

        const result = await forkCommand.handler(ctx, "");

        expect(result).toEqual({
            kind: "error",
            message: "/fork: could not fork this conversation",
        });
    });
});
