// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const outcomesCmd = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AmbientOutcomesCommand: outcomesCmd } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./TokenBreakdownPopover", () => ({ TokenBreakdownPopover: () => null }));

import { TokenUsageIndicator } from "./TokenUsageIndicator";

describe("TokenUsageIndicator — AgentMux's own calls", () => {
    beforeEach(() => {
        outcomesCmd.mockReset();
    });
    afterEach(cleanup);

    it("shows a dot when a purpose is failing", async () => {
        outcomesCmd.mockResolvedValue({ activity_summary: { accepted: 1, "rejected:shape": 2, cli_failed: 2 } });
        render(() => <TokenUsageIndicator />);
        const dot = await screen.findByTestId("ambient-failing-dot");
        expect(dot.getAttribute("title")).toContain("1 purpose failing");
    });

    it("shows no dot when every purpose is healthy", async () => {
        outcomesCmd.mockResolvedValue({ activity_summary: { accepted: 10, kept: 30 } });
        render(() => <TokenUsageIndicator />);
        // The counts of the previous test are still in the shared store until this ask answers.
        await waitFor(() => expect(screen.queryByTestId("ambient-failing-dot")).toBeNull());
    });
});
