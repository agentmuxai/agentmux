// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentStoppedBanner: shown for an agent whose block says it should be stopped, with a Start that
 * calls `agent.start` and says why srv refused when it does.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AgentStoppedBanner, isAgentStopped } from "./AgentStoppedBanner";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AgentStartCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

const block = (meta: Record<string, unknown>) => () => ({ meta });

describe("AgentStoppedBanner", () => {
    afterEach(cleanup);

    it("reads only an explicit stop as stopped", () => {
        expect(isAgentStopped(block({ "agent:desired": "stopped" }))).toBe(true);
        expect(isAgentStopped(block({ "agent:desired": "running" }))).toBe(false);
        expect(isAgentStopped(block({}))).toBe(false);
        expect(isAgentStopped(() => undefined)).toBe(false);
    });

    it("renders nothing for a running agent", () => {
        const { container } = render(() => (
            <AgentStoppedBanner block={block({ "agent:desired": "running" })} agentId="a1" />
        ));
        expect(container.querySelector(".agent-stopped-banner")).toBeNull();
    });

    it("says the agent is stopped and starts it by its id", async () => {
        const start = vi.fn(async () => undefined);
        render(() => <AgentStoppedBanner block={block({ "agent:desired": "stopped" })} agentId="a1" start={start} />);
        expect(screen.getByText(/This agent is stopped/)).toBeInTheDocument();
        await userEvent.click(screen.getByRole("button", { name: "Start" }));
        await waitFor(() => expect(start).toHaveBeenCalledWith("a1"));
    });

    it("shows why a start was refused, and lets the user try again", async () => {
        const start = vi.fn(async () => "Aider can't run without a pane");
        render(() => <AgentStoppedBanner block={block({ "agent:desired": "stopped" })} agentId="a1" start={start} />);
        await userEvent.click(screen.getByRole("button", { name: "Start" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Aider can't run without a pane");
        expect(screen.getByRole("button", { name: "Start" })).not.toBeDisabled();
    });

    it("renders nothing without an agent id to start", () => {
        const { container } = render(() => <AgentStoppedBanner block={block({ "agent:desired": "stopped" })} agentId="" />);
        expect(container.querySelector(".agent-stopped-banner")).toBeNull();
    });
});
