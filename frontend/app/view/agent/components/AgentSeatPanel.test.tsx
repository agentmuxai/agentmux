// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Seat tab (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3): edit and save,
 * clear, a spawned worker's seat "from spawn", srv's refusals shown, and the
 * roster import's preview with its unresolved names.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSeat } from "@/types/rpc/AgentSeat";

const listAgents = vi.fn();
const setSeat = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgents(...args),
        AgentSetSeatCommand: (...args: unknown[]) => setSeat(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));

import { AgentSeatPanel, NO_SHARED_DEFINITION_NOTE } from "./AgentSeatPanel";

const def = (id: string, name: string, seat?: AgentSeat, is_seeded = 0) => ({ id, name, is_seeded, seat });

const AGENTS = (seat?: AgentSeat) => [
    def("uid-x", "AgentX", seat),
    def("uid-c", "Camper"),
    def("uid-o", "Opaz"),
    def("tpl-claude", "Claude Code", undefined, 1),
];

const roleSelect = () => screen.getByLabelText("Role") as HTMLSelectElement;
const leadSelect = () => screen.getByLabelText("Reports to") as HTMLSelectElement;

describe("AgentSeatPanel", () => {
    beforeEach(() => {
        listAgents.mockReset();
        setSeat.mockReset();
    });
    afterEach(() => cleanup());

    it("fills the form from the seat, and saves an edit", async () => {
        listAgents.mockResolvedValue(
            AGENTS({ role: "leader", qualifier: "product", area: "agent pane", set_at_ms: 5 })
        );
        setSeat.mockResolvedValue({ seat: null });
        render(() => <AgentSeatPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(roleSelect().value).toBe("leader"));
        expect((screen.getByLabelText("Qualifier") as HTMLInputElement).value).toBe("product");
        expect(leadSelect().value).toBe("");
        // The lead picker: the operator, then the other agents; not itself, not a template.
        expect([...leadSelect().options].map((o) => o.textContent)).toEqual(["The operator", "Camper", "Opaz"]);

        fireEvent.change(leadSelect(), { target: { value: "uid-c" } });
        fireEvent.input(screen.getByLabelText("Area"), { target: { value: " agent pane, identity " } });
        fireEvent.click(screen.getByRole("button", { name: "Save" }));
        await waitFor(() =>
            expect(setSeat).toHaveBeenCalledWith(
                {},
                {
                    agent_id: "uid-x",
                    seat: { role: "leader", qualifier: "product", lead: "uid-c", area: "agent pane, identity" },
                }
            )
        );
        expect(await screen.findByText("Seat saved.")).toBeInTheDocument();
    });

    it("clears a seat", async () => {
        listAgents.mockResolvedValue(AGENTS({ role: "specialist", set_at_ms: 5 }));
        setSeat.mockResolvedValue({ seat: null });
        render(() => <AgentSeatPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(roleSelect().value).toBe("specialist"));
        fireEvent.click(screen.getByRole("button", { name: "Clear" }));
        await waitFor(() => expect(setSeat).toHaveBeenCalledWith({}, { agent_id: "uid-x", seat: null }));
    });

    it("needs a role before it saves, and has nothing to clear without a seat", async () => {
        listAgents.mockResolvedValue(AGENTS());
        render(() => <AgentSeatPanel agentId="uid-x" agentName="AgentX" />);
        expect(await screen.findByText("No seat yet.")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
        expect(screen.getByRole("button", { name: "Clear" })).toBeDisabled();
    });

    it("marks a spawned worker's seat as from spawn, and lets it be set for real", async () => {
        listAgents.mockResolvedValue(AGENTS({ role: "worker", qualifier: "spawned", lead: "uid-c", set_at_ms: 0 }));
        render(() => <AgentSeatPanel agentId="uid-x" agentName="AgentX" />);
        expect(await screen.findByTestId("agent-seat-from-spawn")).toHaveTextContent(/From spawn/);
        expect(leadSelect().value).toBe("uid-c");
        expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    });

    it("shows the note for a channel-local agent, and srv's own words for anything else", async () => {
        listAgents.mockResolvedValue(AGENTS({ role: "worker", lead: "uid-c", set_at_ms: 0 }));
        setSeat
            .mockRejectedValueOnce(new Error("agent.setSeat: agent uid-x has no shared definition, so it can't have a seat"))
            .mockRejectedValueOnce(
                new Error("INVALID_ARG: uid-c already reports to this agent, so it can't be its lead (a cycle)")
            );
        render(() => <AgentSeatPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(roleSelect().value).toBe("worker"));
        fireEvent.click(screen.getByRole("button", { name: "Save" }));
        expect(await screen.findByText(NO_SHARED_DEFINITION_NOTE)).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Clear" }));
        expect(await screen.findByText(/already reports to this agent/)).toBeInTheDocument();
    });

    it("previews a roster, lists the names it can't resolve, then applies the rest leads first", async () => {
        listAgents.mockResolvedValue(AGENTS());
        setSeat.mockResolvedValueOnce({ seat: null }).mockRejectedValueOnce(new Error("INVALID_ARG: nope"));
        render(() => <AgentSeatPanel agentId="uid-x" agentName="AgentX" />);
        await screen.findByText("No seat yet.");
        fireEvent.click(screen.getByRole("button", { name: "Import roster…" }));
        const roster = {
            Camper: { role: "worker", lead: "AgentX", area: "panes, layout, tabs" },
            AgentX: { role: "leader", qualifier: "product", lead: null },
            Ghost: { role: "worker" },
            "Claude Code": { role: "analyst" },
        };
        fireEvent.input(screen.getByLabelText("Roster JSON"), { target: { value: JSON.stringify(roster) } });
        fireEvent.click(screen.getByRole("button", { name: "Preview" }));

        const preview = await screen.findByTestId("agent-seat-roster-preview");
        expect(preview).toHaveTextContent("Camper — Worker — reports to AgentX — panes, layout, tabs");
        expect(preview).toHaveTextContent("AgentX — Leader · product — reports to the operator");
        const unresolved = screen.getByTestId("agent-seat-roster-unresolved");
        expect(unresolved).toHaveTextContent('Ghost: no agent is named "Ghost" or has that uid');
        // A template is not an agent a roster can seat.
        expect(unresolved).toHaveTextContent('Claude Code: no agent is named "Claude Code" or has that uid');

        fireEvent.click(screen.getByRole("button", { name: "Apply 2 seats" }));
        const outcomes = await screen.findByTestId("agent-seat-roster-outcomes");
        await waitFor(() => expect(setSeat).toHaveBeenCalledTimes(2));
        expect(setSeat.mock.calls.map((c) => (c[1] as { agent_id: string }).agent_id)).toEqual(["uid-x", "uid-c"]);
        await waitFor(() => expect(outcomes).toHaveTextContent("Camper: INVALID_ARG: nope"));
        expect(outcomes).toHaveTextContent("AgentX: seat set");
    });

    it("says so for a pane with no saved agent", async () => {
        listAgents.mockResolvedValue(AGENTS());
        render(() => <AgentSeatPanel agentId="quick-launch" agentName="Claude" />);
        expect(await screen.findByText(/no saved agent/)).toBeInTheDocument();
    });
});
