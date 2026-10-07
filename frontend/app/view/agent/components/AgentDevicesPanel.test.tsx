// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Stash's Devices tab: "Hide from paired devices", saved on change. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const agentHidden = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ViewerAgentHiddenCommand: (...args: unknown[]) => agentHidden(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { AgentDevicesPanel } from "./AgentDevicesPanel";

describe("AgentDevicesPanel", () => {
    beforeEach(() => agentHidden.mockReset());
    afterEach(() => cleanup());

    it("reads the flag, then saves a change", async () => {
        agentHidden.mockImplementation(async (_client: unknown, data?: { hidden?: boolean }) => ({
            hidden: data?.hidden ?? false,
        }));
        render(() => <AgentDevicesPanel agentId="agent-1" />);
        const box = screen.getByRole("checkbox", { name: "Hide from paired devices" }) as HTMLInputElement;
        await waitFor(() => expect(box).toBeEnabled());
        expect(box.checked).toBe(false);
        expect(agentHidden).toHaveBeenCalledWith({}, { agent_id: "agent-1" });

        fireEvent.click(box);
        await waitFor(() => expect(agentHidden).toHaveBeenCalledWith({}, { agent_id: "agent-1", hidden: true }));
        await waitFor(() => expect(box.checked).toBe(true));
    });

    it("shows an already hidden agent as checked", async () => {
        agentHidden.mockResolvedValue({ hidden: true });
        render(() => <AgentDevicesPanel agentId="agent-2" />);
        const box = screen.getByRole("checkbox", { name: "Hide from paired devices" }) as HTMLInputElement;
        await waitFor(() => expect(box.checked).toBe(true));
    });

    it("says why a change didn't save", async () => {
        agentHidden.mockResolvedValueOnce({ hidden: false }).mockRejectedValueOnce("viewer.agent-hidden: no agent agent-3");
        render(() => <AgentDevicesPanel agentId="agent-3" />);
        const box = screen.getByRole("checkbox", { name: "Hide from paired devices" });
        await waitFor(() => expect(box).toBeEnabled());
        fireEvent.click(box);
        expect(await screen.findByText(/no agent agent-3/)).toBeInTheDocument();
    });
});
