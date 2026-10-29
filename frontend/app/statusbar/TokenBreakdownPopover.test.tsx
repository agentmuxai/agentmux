// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenBreakdownPopover — by-agent regroup.
 * SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md.
 *
 * Store-level aggregation (sorting, ambient collapse, cost/cache math)
 * already has thorough coverage in token-usage.test.ts — these tests
 * cover only what that can't: does the popover actually render agent
 * rows, keep the ambient bucket collapsed by default, and wire a click
 * through to focusBlock.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { autoUpdate } from "@floating-ui/dom";
import { refreshAmbient } from "@/app/store/ambient-health";
import { recordTurn, resetSession } from "@/store/token-usage";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
}));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: vi.fn(),
}));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({
        style: { position: "fixed", left: "0px", top: "0px" },
    })),
}));
const outcomesCmd = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AmbientOutcomesCommand: outcomesCmd } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const focusBlockMock = vi.fn(async (_blockId: string, _opts?: unknown) => {});
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: (blockId: string, opts?: unknown) => focusBlockMock(blockId, opts),
}));

import { TokenBreakdownPopover } from "./TokenBreakdownPopover";

describe("TokenBreakdownPopover — by-agent regroup", () => {
    beforeEach(() => {
        resetSession();
        focusBlockMock.mockClear();
        vi.mocked(autoUpdate).mockClear();
    });

    afterEach(() => {
        cleanup();
    });

    function renderPopover() {
        return render(() => (
            <TokenBreakdownPopover anchor={null} onClose={() => {}} />
        ));
    }

    it("shows the empty state when no turns have completed", () => {
        renderPopover();
        expect(screen.getByText("No turns completed yet this session.")).toBeInTheDocument();
    });

    it("renders one row per real agent, with turn count and cost, and no ambient row when nothing ambient ran", () => {
        recordTurn("claude", { input: 1000, output: 50 }, { blockId: "block-1", agentName: "Manoz", costUsd: 0.12 });
        recordTurn("claude", { input: 500, output: 20 }, { blockId: "block-1", agentName: "Manoz", costUsd: 0.05 });
        recordTurn("codex", { input: 300, output: 10 }, { blockId: "block-2", agentName: "Codex Agent" });
        renderPopover();

        expect(screen.getByText("Manoz")).toBeInTheDocument();
        expect(screen.getByText("Codex Agent")).toBeInTheDocument();
        expect(screen.getByText(/2 turns/)).toBeInTheDocument();
        expect(screen.getByText(/\$0\.170/)).toBeInTheDocument();
        expect(screen.queryByText("AgentMux internal")).not.toBeInTheDocument();
    });

    it("collapses ambient usage into a single toggleable row, expanding to per-service detail on click", () => {
        recordTurn("claude", { input: 1000, output: 50 }, { blockId: "block-1", agentName: "Manoz" });
        recordTurn("ambient:next_prompt_suggestion", { input: 100, output: 5 });
        recordTurn("ambient:activity_summary", { input: 40, output: 2 });
        renderPopover();

        expect(screen.getByText(/AgentMux internal/)).toBeInTheDocument();
        // Collapsed by default — per-service ambient rows not yet in the DOM.
        expect(screen.queryByText("Next Prompt Suggestion")).not.toBeInTheDocument();

        const toggle = screen.getByText(/AgentMux internal/).closest("button") as HTMLButtonElement;
        fireEvent.click(toggle);

        // Expanded: the ambient bucket's own service breakdown appears.
        expect(toggle.getAttribute("aria-expanded")).toBe("true");
    });

    it("shows what AgentMux's own model calls did, per purpose, inside the internal row", async () => {
        outcomesCmd.mockResolvedValue({ activity_summary: { accepted: 1, "rejected:shape": 2, cli_failed: 2 } });
        recordTurn("ambient:activity_summary", { input: 40, output: 2 });
        renderPopover();
        fireEvent.click(screen.getByText(/AgentMux internal/).closest("button") as HTMLButtonElement);
        expect(await screen.findByRole("button", { name: /Session titles/ })).toBeInTheDocument();
        expect(screen.getByText("2 failed")).toBeInTheDocument();
    });

    it("lists the purposes even when no turn has completed and no internal tokens were recorded", async () => {
        outcomesCmd.mockResolvedValue({ activity_summary: { accepted: 1, "rejected:shape": 2, cli_failed: 2 } });
        renderPopover();
        // The counts come from the indicator's ask, which is not mounted here.
        await refreshAmbient();
        expect(await screen.findByRole("button", { name: /Session titles/ })).toBeInTheDocument();
        expect(screen.queryByText("No turns completed yet this session.")).not.toBeInTheDocument();
    });

    it("clicking a real agent row calls focusBlock with that agent's blockId", () => {
        recordTurn("claude", { input: 1000, output: 50 }, { blockId: "block-1", agentName: "Manoz" });
        renderPopover();

        const row = screen.getByText("Manoz").closest("button") as HTMLButtonElement;
        fireEvent.click(row);

        expect(focusBlockMock).toHaveBeenCalledWith("block-1", { source: "tokens" });
    });

    it("keeps an agent's row element when a turn lands, so a press on it is not lost, and the click goes to its agent", () => {
        recordTurn("claude", { input: 1000, output: 50 }, { blockId: "block-1", agentName: "Manoz" });
        renderPopover();
        const before = screen.getByText("Manoz").closest("button") as HTMLButtonElement;

        recordTurn("claude", { input: 500, output: 20 }, { blockId: "block-1", agentName: "Manoz" });
        recordTurn("claude", { input: 10, output: 1 }, { blockId: "block-2", agentName: "Camper" });

        const after = screen.getByText("Manoz").closest("button") as HTMLButtonElement;
        expect(after).toBe(before);
        expect(after.textContent).toContain("2 turns");
        fireEvent.click(before);
        expect(focusBlockMock).toHaveBeenCalledWith("block-1", { source: "tokens" });
    });

    it("does not throw when clicking the ambient row (no blockId to focus)", () => {
        recordTurn("ambient:next_prompt_suggestion", { input: 100, output: 5 });
        renderPopover();

        const row = screen.getByText(/AgentMux internal/).closest("button") as HTMLButtonElement;
        expect(() => fireEvent.click(row)).not.toThrow();
        expect(focusBlockMock).not.toHaveBeenCalled();
    });
});
