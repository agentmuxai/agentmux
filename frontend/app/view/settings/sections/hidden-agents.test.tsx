// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Paired devices › Hidden from devices: the list over `viewer.agent-hidden`. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listAgents = vi.fn();
const agentHidden = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgents(...args),
        ViewerAgentHiddenCommand: (...args: unknown[]) => agentHidden(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { HiddenAgents } from "./hidden-agents";

const agent = (id: string, name: string) => ({ id, slug: id, name });

// The srv side: a flag per agent id, read when `hidden` is absent, set when given.
function serveFlags(initial: Record<string, boolean>) {
    const flags = { ...initial };
    agentHidden.mockImplementation(async (_c: unknown, data: { agent_id: string; hidden?: boolean }) => {
        if (data.hidden !== undefined) flags[data.agent_id] = data.hidden;
        return { hidden: !!flags[data.agent_id] };
    });
}

describe("HiddenAgents", () => {
    beforeEach(() => {
        listAgents.mockReset();
        agentHidden.mockReset();
        listAgents.mockResolvedValue([agent("a1", "Alpha"), agent("a2", "Bravo"), agent("a3", "Charlie")]);
    });
    afterEach(() => cleanup());

    it("lists only the agents whose flag is set, and offers the rest to add", async () => {
        serveFlags({ a2: true });
        render(() => <HiddenAgents />);
        const row = (await screen.findByText("Bravo")).closest("li")!;
        expect(within(row).getByRole("button", { name: "Remove Bravo from the hidden list" })).toBeInTheDocument();
        expect(screen.queryByRole("listitem", { name: /Alpha/ })).not.toBeInTheDocument();
        const options = Array.from(screen.getByRole("combobox").querySelectorAll("option")).map((o) => o.textContent);
        expect(options).toEqual(["Choose an agent…", "Alpha", "Charlie"]);
        expect(listAgents).toHaveBeenCalledWith({}, { is_seeded: 0 });
    });

    it("says nothing is hidden when no flag is set", async () => {
        serveFlags({});
        render(() => <HiddenAgents />);
        expect(await screen.findByText(/No agent is hidden/)).toBeInTheDocument();
    });

    it("Add agent hides the chosen agent through the RPC and lists it", async () => {
        serveFlags({});
        render(() => <HiddenAgents />);
        const select = await screen.findByRole("combobox");
        const add = screen.getByRole("button", { name: "Add agent" });
        expect(add).toBeDisabled();
        fireEvent.change(select, { target: { value: "a3" } });
        expect(add).toBeEnabled();
        fireEvent.click(add);
        await waitFor(() => expect(agentHidden).toHaveBeenCalledWith({}, { agent_id: "a3", hidden: true }));
        const row = (await screen.findByText("Charlie", { selector: ".setting-hidden-agent-name" })).closest("li")!;
        expect(within(row).getByRole("button", { name: /Remove Charlie/ })).toBeInTheDocument();
        expect(screen.queryByText(/No agent is hidden/)).not.toBeInTheDocument();
        // It leaves the picker, so it can't be added twice.
        const options = Array.from(screen.getByRole("combobox").querySelectorAll("option")).map((o) => o.textContent);
        expect(options).toEqual(["Choose an agent…", "Alpha", "Bravo"]);
    });

    it("Remove shows the agent again through the RPC and puts it back in the picker", async () => {
        serveFlags({ a1: true });
        render(() => <HiddenAgents />);
        fireEvent.click(await screen.findByRole("button", { name: "Remove Alpha from the hidden list" }));
        await waitFor(() => expect(agentHidden).toHaveBeenCalledWith({}, { agent_id: "a1", hidden: false }));
        await waitFor(() => expect(screen.getByText(/No agent is hidden/)).toBeInTheDocument());
        const options = Array.from(screen.getByRole("combobox").querySelectorAll("option")).map((o) => o.textContent);
        expect(options).toContain("Alpha");
    });

    it("keeps the list and says why when a change is refused", async () => {
        serveFlags({ a1: true });
        render(() => <HiddenAgents />);
        const remove = await screen.findByRole("button", { name: "Remove Alpha from the hidden list" });
        agentHidden.mockRejectedValueOnce(new Error("viewer.agent-hidden: no agent a1"));
        fireEvent.click(remove);
        expect(await screen.findByRole("alert")).toHaveTextContent(
            "Couldn't change Alpha: viewer.agent-hidden: no agent a1"
        );
        expect(screen.getByRole("button", { name: "Remove Alpha from the hidden list" })).toBeInTheDocument();
    });

    it("says so when the agents can't be listed", async () => {
        listAgents.mockRejectedValue(new Error("srv is down"));
        render(() => <HiddenAgents />);
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't list hidden agents: srv is down");
    });
});
