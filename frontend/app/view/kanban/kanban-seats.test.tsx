// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Seats on the Kanban (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4): a card's owner shows its seat
 * on hover, and "Reports to …" keeps the tasks of agents that report to the operator or one lead.
 */
import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentSeat } from "@/types/rpc/AgentSeat";

const listAgents = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ListAgentDefinitionsCommand: (...args: unknown[]) => listAgents(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import { AgentChip } from "./kanban-card";
import {
    REPORTS_TO_OPERATOR,
    reportsTo,
    reportsToOptions,
    ReportsToSelect,
    seatIndex,
    seatOf,
    taskReportsTo,
} from "./kanban-seats";

const def = (id: string, name: string, seat?: AgentSeat, is_seeded = 0) => ({ id, name, is_seeded, seat });
const DEFS = [
    def("uid-camper", "Camper", { role: "leader", qualifier: "product", area: "panes, layout", set_at_ms: 1 }),
    def("uid-naki", "Naki-2", { role: "leader", set_at_ms: 1 }),
    def("uid-w1", "W1", { role: "worker", qualifier: "spawned", lead: "uid-camper", set_at_ms: 0 }),
    def("uid-w2", "W2", { role: "specialist", lead: "uid-naki", set_at_ms: 1 }),
    def("uid-free", "Free"),
    def("tpl", "Claude Code", { role: "leader", set_at_ms: 1 }, 1),
];
const index = seatIndex(DEFS);

const task = (claimed_by = "", claimed_by_uid = "", target_agent = "", target_agent_uid = "") => ({
    claimed_by,
    claimed_by_uid,
    target_agent,
    target_agent_uid,
});

describe("Kanban seats", () => {
    it("finds an agent's seat by uid, or by name when the task names no uid, and leaves templates out", () => {
        expect(seatOf(index, "uid-w1", "W1")?.lead).toEqual({ id: "uid-camper", name: "Camper" });
        expect(seatOf(index, "", "w1")?.role).toBe("worker");
        expect(seatOf(index, "uid-free", "Free")).toBeNull();
        expect(seatOf(index, "", "Claude Code")).toBeNull();
        expect(seatOf(index, "", "")).toBeNull();
    });

    it("filters by whom the owner reports to: the holder first, else the agent it waits for", () => {
        expect(taskReportsTo(index, task("W1", "uid-w1"), "lead:uid-camper")).toBe(true);
        expect(taskReportsTo(index, task("", "", "W2", "uid-w2"), "lead:uid-camper")).toBe(false);
        expect(taskReportsTo(index, task("", "", "W2", "uid-w2"), "lead:uid-naki")).toBe(true);
        expect(taskReportsTo(index, task("Camper", "uid-camper", "W2", "uid-w2"), REPORTS_TO_OPERATOR)).toBe(true);
        // A task for no one, or an owner with no seat, matches only "anyone".
        expect(taskReportsTo(index, task(), REPORTS_TO_OPERATOR)).toBe(false);
        expect(taskReportsTo(index, task(), "")).toBe(true);
        expect(reportsTo(null, "lead:uid-camper")).toBe(false);
        expect(reportsTo(null, "")).toBe(true);
    });

    it("offers anyone, the operator, then each lead someone reports to, by name", () => {
        expect(reportsToOptions(index).map((o) => o.label)).toEqual([
            "Reports to anyone",
            "Reports to the operator",
            "Reports to Camper",
            "Reports to Naki-2",
        ]);
        expect(reportsToOptions(seatIndex([])).map((o) => o.value)).toEqual([""]);
    });
});

describe("Kanban seats on screen", () => {
    beforeEach(() => listAgents.mockReset());
    afterEach(() => cleanup());

    it("shows a card owner's seat on hover", async () => {
        listAgents.mockResolvedValue(DEFS);
        const { container } = render(() => <AgentChip name="W1" uid="uid-w1" showSeat />);
        const chip = container.querySelector(".kanban-agent-chip")!;
        await waitFor(() => expect(chip.getAttribute("title")).toBe("W1: Worker · spawned — reports to Camper"));
    });

    it("finds the owner by name when the task has no uid, and leaves other chips without a seat", async () => {
        listAgents.mockResolvedValue(DEFS);
        const { container } = render(() => (
            <>
                <AgentChip name="Camper" showSeat />
                <AgentChip name="W1" uid="uid-w1" />
            </>
        ));
        const [owner, plain] = [...container.querySelectorAll(".kanban-agent-chip")];
        await waitFor(() =>
            expect(owner.getAttribute("title")).toBe(
                "Camper: Leader · product — reports to the operator — panes, layout"
            )
        );
        expect(plain.hasAttribute("title")).toBe(false);
    });

    it("offers the filter once an agent has a seat", async () => {
        listAgents.mockResolvedValue(DEFS);
        const onChange = vi.fn();
        const { findByLabelText } = render(() => <ReportsToSelect value="" onChange={onChange} />);
        const select = (await findByLabelText("Reports to")) as HTMLSelectElement;
        fireEvent.change(select, { target: { value: "lead:uid-naki" } });
        expect(onChange).toHaveBeenCalledWith("lead:uid-naki");
    });

    it("hides the filter when no agent has a seat", async () => {
        listAgents.mockResolvedValue([def("uid-free", "Free")]);
        const { container } = render(() => <ReportsToSelect value="" onChange={() => {}} />);
        await waitFor(() => expect(listAgents).toHaveBeenCalled());
        await Promise.resolve();
        expect(container.querySelector("select")).toBeNull();
    });
});
