// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Browser › Agents on sites › Page permissions: the list over `browsergrants.permission*`. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const list = vi.fn();
const permissionSet = vi.fn();
const permissionRevoke = vi.fn();
const listAgents = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BrowserGrantsListCommand: (...a: unknown[]) => list(...a),
        BrowserGrantsPermissionSetCommand: (...a: unknown[]) => permissionSet(...a),
        BrowserGrantsPermissionRevokeCommand: (...a: unknown[]) => permissionRevoke(...a),
        ListAgentDefinitionsCommand: (...a: unknown[]) => listAgents(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/ws", () => ({ addWSReconnectHandler: () => {} }));

// The section keeps its rows in a store shared by every mount, so each test loads a fresh copy of it.
let AgentSiteGrants: typeof import("./agent-site-grants").AgentSiteGrants;

const agent = (slug: string, name: string) => ({ id: `id-${slug}`, slug, name });
const permission = (over: Record<string, unknown> = {}) => ({
    id: "p1",
    agent: "naki-2",
    site: "https://meet.example.com",
    kinds: ["camera", "microphone"],
    granted_at_ms: Date.now() - 2 * 3600_000,
    ...over,
});
const result = (permissions: unknown[] = [permission()]) => ({
    rows: [],
    presets: [],
    powers: ["terminal"],
    permission_kinds: ["camera", "microphone", "location", "notifications"],
    permissions,
});

describe("Page permissions", () => {
    beforeEach(async () => {
        vi.resetModules();
        ({ AgentSiteGrants } = await import("./agent-site-grants"));
        for (const f of [list, permissionSet, permissionRevoke, listAgents]) f.mockReset();
        listAgents.mockResolvedValue([agent("naki-2", "Naki-2"), agent("agent1", "Agent1")]);
        list.mockResolvedValue(result());
    });
    afterEach(() => cleanup());

    it("lists each site permission with the agent, the site, what it carries and when", async () => {
        render(() => <AgentSiteGrants />);
        const li = (await screen.findByText("meet.example.com")).closest("li")!;
        expect(within(li).getByText("Naki-2")).toBeInTheDocument();
        expect(within(li).getByText(/allowed 2 h ago/)).toBeInTheDocument();
        expect(within(li).getByRole("checkbox", { name: /Camera/ })).toBeChecked();
        expect(within(li).getByRole("checkbox", { name: /Microphone/ })).toBeChecked();
        expect(within(li).getByRole("checkbox", { name: /Location/ })).not.toBeChecked();
    });

    it("says there is none when the list is empty", async () => {
        list.mockResolvedValue(result([]));
        render(() => <AgentSiteGrants />);
        expect(await screen.findByText(/No site has a standing page permission/)).toBeInTheDocument();
    });

    it("turns a kind on by sending the kinds that stay, in srv's order", async () => {
        permissionSet.mockResolvedValue(result());
        render(() => <AgentSiteGrants />);
        fireEvent.click(await screen.findByRole("checkbox", { name: /Notifications.*Naki-2/ }));
        await waitFor(() =>
            expect(permissionSet).toHaveBeenCalledWith({}, { agent: "naki-2", site: "https://meet.example.com", kinds: ["camera", "microphone", "notifications"] })
        );
    });

    it("revokes the row when the last kind is switched off", async () => {
        list.mockResolvedValue(result([permission({ kinds: ["camera"] })]));
        permissionRevoke.mockResolvedValue(result([]));
        render(() => <AgentSiteGrants />);
        fireEvent.click(await screen.findByRole("checkbox", { name: /Camera.*Naki-2/ }));
        await waitFor(() => expect(permissionRevoke).toHaveBeenCalledWith({}, { id: "p1" }));
        expect(permissionSet).not.toHaveBeenCalled();
        expect(await screen.findByText(/No site has a standing page permission/)).toBeInTheDocument();
    });

    it("revokes a row with its Revoke button", async () => {
        permissionRevoke.mockResolvedValue(result([]));
        render(() => <AgentSiteGrants />);
        fireEvent.click(await screen.findByRole("button", { name: /Revoke the page permissions of Naki-2/ }));
        await waitFor(() => expect(permissionRevoke).toHaveBeenCalledWith({}, { id: "p1" }));
    });

    it("adds a site permission for the chosen agent once a site and a kind are given", async () => {
        permissionSet.mockResolvedValue(result());
        render(() => <AgentSiteGrants />);
        await screen.findByText("meet.example.com");
        const add = screen.getByRole("button", { name: "Allow on this site" });
        expect(add).toBeDisabled();
        fireEvent.change(screen.getByRole("combobox", { name: "Agent to give a standing grant" }), { target: { value: "agent1" } });
        fireEvent.input(screen.getByLabelText("Site for page permissions"), { target: { value: "talk.example.com" } });
        expect(add).toBeDisabled();
        fireEvent.click(screen.getByRole("checkbox", { name: /Microphone: the new site/ }));
        expect(add).not.toBeDisabled();
        fireEvent.click(add);
        await waitFor(() => expect(permissionSet).toHaveBeenCalledWith({}, { agent: "agent1", site: "talk.example.com", kinds: ["microphone"] }));
    });
});
