// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory → Skills: a skill's details list the agents it is bound to, each with
// Unbind (docs/specs/SPEC_DROP_THE_AGENT_STASH_2026_10_24.md §3).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    SkillCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    SkillCatalogListForAgentCommand: vi.fn(),
    SkillCatalogUnbindCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}), muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: vi.fn() }));
vi.mock("@/app/element/markdown", () => ({ Markdown: () => null }));

import { SkillManager } from "./skill-manager";

function skill(over: Record<string, unknown> = {}) {
    return {
        id: "k1",
        name: "pdf-extraction",
        trigger: "",
        skill_type: "prompt",
        description: "",
        content: "",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 2,
        ...over,
    };
}

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    rpc.SkillCatalogListCommand.mockResolvedValue([skill()]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([
        { id: "a1", name: "Korp" },
        { id: "a2", name: "AgentY" },
        { id: "a3", name: "Idle" },
    ]);
    rpc.SkillCatalogListForAgentCommand.mockImplementation((_c: unknown, { agent_id }: { agent_id: string }) =>
        Promise.resolve(agent_id === "a3" ? [] : [{ ...skill(), bound_to_agent: true }])
    );
});
afterEach(() => cleanup());

const names = (list: HTMLElement) =>
    within(list)
        .getAllByRole("listitem")
        .map((li) => li.firstElementChild?.textContent);

describe("SkillManager", () => {
    it("lists the agents a skill is bound to in its details", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("pdf-extraction"));
        const list = await screen.findByRole("list", { name: "Agents it is bound to" });
        expect(names(list)).toEqual(["AgentY", "Korp"]);
    });

    it("Unbind removes the bind through srv and the list follows", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("pdf-extraction"));
        const list = await screen.findByRole("list", { name: "Agents it is bound to" });

        rpc.SkillCatalogUnbindCommand.mockResolvedValue({ unbound: true });
        rpc.SkillCatalogListCommand.mockResolvedValue([skill({ bound_count: 1 })]);
        rpc.SkillCatalogListForAgentCommand.mockImplementation((_c: unknown, { agent_id }: { agent_id: string }) =>
            Promise.resolve(agent_id === "a2" ? [{ ...skill(), bound_to_agent: true }] : [])
        );
        fireEvent.click(within(list).getByRole("button", { name: "Unbind Korp" }));

        await waitFor(() =>
            expect(rpc.SkillCatalogUnbindCommand).toHaveBeenCalledWith({}, { agent_id: "a1", skill_id: "k1" })
        );
        await waitFor(() =>
            expect(names(screen.getByRole("list", { name: "Agents it is bound to" }))).toEqual(["AgentY"])
        );
        expect(screen.getByText("Used by 1 agent")).toBeTruthy();
    });

    it("shows a failed unbind and keeps the agent listed", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("pdf-extraction"));
        const unbind = await screen.findByRole("button", { name: "Unbind Korp" });
        rpc.SkillCatalogUnbindCommand.mockRejectedValue(new Error("nope"));
        fireEvent.click(unbind);
        expect(await screen.findByText("Unbind failed: nope")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Unbind Korp" })).toBeTruthy();
    });

    it("lists nobody for a skill no agent is bound to", async () => {
        rpc.SkillCatalogListCommand.mockResolvedValue([skill({ bound_count: 0 })]);
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("pdf-extraction"));
        await screen.findByText("Used by 0 agents");
        expect(screen.queryByRole("list", { name: "Agents it is bound to" })).toBeNull();
        expect(rpc.SkillCatalogListForAgentCommand).not.toHaveBeenCalled();
    });
});
