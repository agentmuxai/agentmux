// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory → Skills: the agents × skills grid says how each agent gets each skill and changes it.

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cellAction, cellState } from "./skill-matrix-model";

const rpc = vi.hoisted(() => ({
    SkillCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    SkillCatalogListForAgentCommand: vi.fn(),
    SkillCatalogBindCommand: vi.fn(),
    SkillCatalogUnbindCommand: vi.fn(),
    SkillCatalogSetOptOutCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}), muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: vi.fn() }));
vi.mock("@/app/element/markdown", () => ({ Markdown: () => null }));

import { SkillManager } from "./skill-manager";

function skill(id: string, over: Record<string, unknown> = {}) {
    return {
        id,
        name: id,
        trigger: id,
        skill_type: "agent-skill",
        description: "",
        content: "c",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 0,
        all_agents: false,
        new_agents: false,
        source: null,
        ...over,
    };
}

const facts = (bound: boolean, reached: boolean, opted_out = false) => ({ bound_to_agent: bound, reached, opted_out });

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    rpc.SkillCatalogListCommand.mockResolvedValue([skill("pdf"), skill("deploy", { all_agents: true })]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([
        { id: "a1", name: "Korp" },
        { id: "a2", name: "AgentY" },
    ]);
    // Korp: pdf bound; deploy reaches it. AgentY: pdf nothing; deploy opted out.
    rpc.SkillCatalogListForAgentCommand.mockImplementation((_c: unknown, { agent_id }: { agent_id: string }) =>
        Promise.resolve(
            agent_id === "a1"
                ? [{ ...skill("pdf"), ...facts(true, false) }, { ...skill("deploy"), ...facts(false, true) }]
                : [{ ...skill("pdf"), ...facts(false, false) }, { ...skill("deploy"), ...facts(false, false, true) }],
        ),
    );
});
afterEach(() => cleanup());

const cell = (agent: string, skillName: string) =>
    screen.getByRole("button", { name: new RegExp(`^${agent}, ${skillName}:`) });

describe("cell state", () => {
    it("a bind wins over a reach, an opt-out over nothing, and the action follows", () => {
        const all = skill("s", { all_agents: true }) as never;
        const fresh = skill("s") as never;
        expect(cellState(all, { bound: true, reached: true, optedOut: false })).toBe("bound");
        expect(cellState(all, { bound: false, reached: true, optedOut: false })).toBe("reach-all");
        expect(cellState(fresh, { bound: false, reached: true, optedOut: false })).toBe("reach-new");
        expect(cellState(all, { bound: false, reached: false, optedOut: true })).toBe("opted-out");
        expect(cellState(all, undefined)).toBe("none");
        expect(["bound", "reach-all", "opted-out", "none"].map((s) => cellAction(s as never))).toEqual([
            "Unbind",
            "Opt out",
            "Opt back in",
            "Bind",
        ]);
    });
});

describe("Agents and skills grid", () => {
    async function open() {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Agents and skills" }));
        await screen.findByTestId("skill-matrix");
    }

    it("shows how each agent gets each skill", async () => {
        await open();
        await waitFor(() => expect(cell("Korp", "pdf").textContent).toBe("Bound"));
        expect(cell("Korp", "deploy").textContent).toBe("Every agent");
        expect(cell("AgentY", "pdf").textContent).toBe("—");
        expect(cell("AgentY", "deploy").textContent).toBe("Opted out");
    });

    it("clicking a cell binds, unbinds, opts out or opts back in, through srv", async () => {
        await open();
        await waitFor(() => expect(cell("Korp", "pdf").textContent).toBe("Bound"));

        fireEvent.click(cell("AgentY", "pdf"));
        await waitFor(() => expect(rpc.SkillCatalogBindCommand).toHaveBeenCalledWith({}, { agent_id: "a2", skill_id: "pdf" }));
        fireEvent.click(cell("Korp", "pdf"));
        await waitFor(() => expect(rpc.SkillCatalogUnbindCommand).toHaveBeenCalledWith({}, { agent_id: "a1", skill_id: "pdf" }));
        fireEvent.click(cell("Korp", "deploy"));
        await waitFor(() =>
            expect(rpc.SkillCatalogSetOptOutCommand).toHaveBeenCalledWith({}, { agent_id: "a1", skill_id: "deploy", opted_out: true }),
        );
        fireEvent.click(cell("AgentY", "deploy"));
        await waitFor(() =>
            expect(rpc.SkillCatalogSetOptOutCommand).toHaveBeenCalledWith({}, { agent_id: "a2", skill_id: "deploy", opted_out: false }),
        );
    });

    it("says why a change failed and keeps the grid", async () => {
        await open();
        await waitFor(() => expect(cell("Korp", "pdf").textContent).toBe("Bound"));
        rpc.SkillCatalogUnbindCommand.mockRejectedValue(new Error("boom"));
        fireEvent.click(cell("Korp", "pdf"));
        expect(await screen.findByText(/Couldn't change Korp: boom/)).toBeTruthy();
        expect(cell("Korp", "pdf").textContent).toBe("Bound");
    });
});
