// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const outcomesMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AmbientOutcomesCommand: (...args: unknown[]) => outcomesMock(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { AgentMuxAiSection } from "./swarm-agentmux-ai";

afterEach(() => {
    cleanup();
    outcomesMock.mockReset();
});

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("AgentMux AI section", () => {
    it("starts collapsed with the call total, and lists every purpose when opened", async () => {
        outcomesMock.mockResolvedValue({
            activity_summary: { accepted: 10, kept: 30 },
            next_prompt_suggestion: { accepted: 5, timeout: 1 },
        });
        const { container, getByText, queryByText } = render(() => <AgentMuxAiSection />);
        await settle();
        expect(getByText("AgentMux AI")).toBeTruthy();
        expect(getByText("46 calls")).toBeTruthy();
        expect(queryByText("Session titles")).toBeNull();

        fireEvent.click(container.querySelector(".swarm-agentmux-ai-header")!);
        await settle();
        expect(getByText("Session titles")).toBeTruthy();
        expect(getByText("10 accepted · 30 kept")).toBeTruthy();
        expect(getByText("Prompt suggestions")).toBeTruthy();
        expect(getByText("5 accepted · 0 kept · 1 failed")).toBeTruthy();
    });

    it("marks a failing purpose in the header and on its row", async () => {
        outcomesMock.mockResolvedValue({ activity_summary_pushed: { accepted: 1, cli_failed: 4 } });
        const { container, getByText } = render(() => <AgentMuxAiSection />);
        await settle();
        expect(getByText("1 failing")).toBeTruthy();
        fireEvent.click(container.querySelector(".swarm-agentmux-ai-header")!);
        await settle();
        expect(container.querySelector(".swarm-agentmux-ai-counts--warn")?.textContent).toBe("1 accepted · 0 kept · 4 failed");
    });

    it("stays hidden before any call has ended, or on a server without the command", async () => {
        outcomesMock.mockResolvedValue({});
        const first = render(() => <AgentMuxAiSection />);
        await settle();
        expect(first.container.querySelector(".swarm-agentmux-ai")).toBeNull();
        cleanup();

        outcomesMock.mockRejectedValue(new Error("unknown command ambient.outcomes"));
        const second = render(() => <AgentMuxAiSection />);
        await settle();
        expect(second.container.querySelector(".swarm-agentmux-ai")).toBeNull();
    });
});
