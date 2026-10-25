// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Warden > Host: a row expands to the agent's other-instance claims and recent identity mismatches. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const registrations = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { GetReactiveRegistrationsCommand: (...args: unknown[]) => registrations(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/auth-headers", () => ({ authHeaders: () => ({}) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));

import { WardenHostManager } from "./warden-host-manager";

const NOW = Date.now();
const hostAgent = (agent_id: string, block_id: string) => ({
    agent_id,
    block_id,
    registered_at: NOW - 60_000,
    last_seen: NOW - 5_000,
});

const local = { agent_id: "loap2", block_id: "block-loap2", registered_at: NOW, last_seen: NOW, registration_nonce: 1 };

describe("WardenHostManager registration details", () => {
    beforeEach(() => {
        registrations.mockReset();
        vi.stubGlobal(
            "fetch",
            vi.fn().mockResolvedValue({
                ok: true,
                json: async () => [hostAgent("loap2", "block-loap2-0000"), hostAgent("agent5", "block-agent5-000")],
            })
        );
        registrations.mockImplementation(async (_c: unknown, data: { agent_id: string }) =>
            data.agent_id === "loap2"
                ? {
                      local,
                      remote: [{ channel: "dev", pid: 4242, updated_at: NOW - 120_000 }],
                      recent_mismatch: {
                          timestamp: NOW - 300_000,
                          block_id: "deadbeef-1234",
                          error_message: "live identity differs",
                      },
                  }
                : { local, remote: [], recent_mismatch: null }
        );
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("reads nothing until a row is opened", async () => {
        render(() => <WardenHostManager />);
        await screen.findByText("loap2");
        expect(registrations).not.toHaveBeenCalled();
        expect(screen.queryByText(/Also claimed by another instance/)).not.toBeInTheDocument();
    });

    it("shows a claim and a mismatch from reactive.registrations when the row opens", async () => {
        render(() => <WardenHostManager />);
        await screen.findByText("loap2");
        fireEvent.click(screen.getByRole("button", { name: "Show registration details for loap2" }));

        expect(await screen.findByText("Also claimed by another instance on this host")).toBeInTheDocument();
        expect(registrations).toHaveBeenCalledWith({}, { agent_id: "loap2" });
        const claim = screen.getByText("dev").closest("li")!;
        expect(claim).toHaveTextContent("channel dev (pid 4242), updated 2m ago");
        expect(screen.getByText("Recent identity mismatch")).toBeInTheDocument();
        const mismatch = screen.getByText(/live identity differs/).closest("li")!;
        expect(mismatch).toHaveTextContent("5m ago, block deadbeef: live identity differs");
        expect(screen.getByRole("button", { name: "Hide registration details for loap2" })).toHaveAttribute(
            "aria-expanded",
            "true"
        );
    });

    it("an agent with no claim or mismatch gets no claim or mismatch section", async () => {
        render(() => <WardenHostManager />);
        await screen.findByText("agent5");
        fireEvent.click(screen.getByRole("button", { name: "Show registration details for agent5" }));

        await waitFor(() => expect(registrations).toHaveBeenCalledWith({}, { agent_id: "agent5" }));
        expect(await screen.findByText(/No other instance claims this agent/)).toBeInTheDocument();
        expect(screen.queryByText(/Also claimed by another instance/)).not.toBeInTheDocument();
        expect(screen.queryByText("Recent identity mismatch")).not.toBeInTheDocument();
    });

    it("closing the row hides the details, and a failed read says so", async () => {
        registrations.mockRejectedValue(new Error("rpc down"));
        render(() => <WardenHostManager />);
        await screen.findByText("loap2");
        fireEvent.click(screen.getByRole("button", { name: "Show registration details for loap2" }));
        expect(await screen.findByText(/Couldn't read registration: rpc down/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Hide registration details for loap2" }));
        expect(screen.queryByText(/Couldn't read registration/)).not.toBeInTheDocument();
    });
});
