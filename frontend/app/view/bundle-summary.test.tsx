// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `BundleSummaryPanel`'s `agentId`-driven bound-bundle
 * resolution — closes the DATA GAP documented in the module's own header
 * comment (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.3/§6):
 * when this panel's block was opened with `meta.agentId` set, it must
 * resolve and show that specific agent's own dedicated ABF bundle
 * (`AgentDefinition.memory_id`) instead of staying purely generic.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getMemory = vi.fn();
let agentsList: any[] = [];

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        GetMemoryCommand: (...args: unknown[]) => getMemory(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({
    openOrFocusPaneByView: vi.fn(),
}));
vi.mock("@/app/view/agent/components/AgentPicker", () => ({
    useAgentDefinitions: () => () => agentsList,
}));

import { BundleSummaryPanel } from "./bundle-summary";

describe("BundleSummaryPanel", () => {
    beforeEach(() => {
        getMemory.mockReset();
        agentsList = [];
    });

    afterEach(() => {
        cleanup();
    });

    it("renders the generic pointer-only form when agentId is absent", () => {
        render(() => <BundleSummaryPanel kind="Bundle" />);
        expect(screen.getByText(/Manage in Identity & Memory/)).toBeInTheDocument();
        expect(screen.queryByText("This agent's own ABF")).not.toBeInTheDocument();
    });

    it("shows the bound bundle's name and provider when the agent has one", async () => {
        agentsList = [{ id: "agent-1", name: "Agent One", provider: "claude", memory_id: "mem-1" }];
        getMemory.mockResolvedValue({
            id: "mem-1",
            name: "Agent One — ABF",
            provider: "claude",
            model: "anthropic",
            created_at: 0,
            updated_at: 0,
        });

        render(() => <BundleSummaryPanel kind="Bundle" agentId="agent-1" />);

        await waitFor(() => {
            expect(screen.getByText("Agent One — ABF")).toBeInTheDocument();
        });
        expect(getMemory).toHaveBeenCalledWith({}, { id: "mem-1" });
        expect(screen.getByText(/Edit in Identity & Memory/)).toBeInTheDocument();
    });

    it("shows a hint when the agent has no bundle of its own yet", async () => {
        agentsList = [{ id: "agent-1", name: "Agent One", provider: "claude", memory_id: "" }];

        render(() => <BundleSummaryPanel kind="Bundle" agentId="agent-1" />);

        await waitFor(() => {
            expect(screen.getByText(/has no ABF bundle of its own yet/)).toBeInTheDocument();
        });
        expect(getMemory).not.toHaveBeenCalled();
    });

    it("shows a hint when the bound bundle id fails to resolve", async () => {
        agentsList = [{ id: "agent-1", name: "Agent One", provider: "claude", memory_id: "mem-deleted" }];
        getMemory.mockRejectedValue(new Error("not found"));

        render(() => <BundleSummaryPanel kind="Bundle" agentId="agent-1" />);

        await waitFor(() => {
            expect(screen.getByText(/couldn't be loaded/)).toBeInTheDocument();
        });
    });

    it("does not resolve a bundle for an unrelated agentId not in the list", () => {
        agentsList = [{ id: "agent-2", name: "Someone Else", provider: "claude", memory_id: "mem-2" }];

        render(() => <BundleSummaryPanel kind="Bundle" agentId="agent-1" />);

        expect(screen.queryByText("This agent's own ABF")).not.toBeInTheDocument();
        expect(getMemory).not.toHaveBeenCalled();
    });
});
