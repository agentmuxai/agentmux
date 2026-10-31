// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Browser › Agents on sites: the list over `browsergrants.*`. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const list = vi.fn();
const set = vi.fn();
const preset = vi.fn();
const revoke = vi.fn();
const powers = vi.fn();
const listAgents = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BrowserGrantsListCommand: (...a: unknown[]) => list(...a),
        BrowserGrantsSetCommand: (...a: unknown[]) => set(...a),
        BrowserGrantsPresetCommand: (...a: unknown[]) => preset(...a),
        BrowserGrantsRevokeCommand: (...a: unknown[]) => revoke(...a),
        BrowserGrantsPowersCommand: (...a: unknown[]) => powers(...a),
        ListAgentDefinitionsCommand: (...a: unknown[]) => listAgents(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { AgentSiteGrants } from "./agent-site-grants";

const agent = (slug: string, name: string) => ({ id: `id-${slug}`, slug, name });
const row = (over: Partial<{ id: string; agent: string; origins: string[]; powers: string[] }> = {}) => ({
    id: "r1",
    agent: "naki-2",
    origins: ["https://shell.cloud.google.com", "https://ssh.cloud.google.com"],
    powers: ["clicks", "terminal"],
    granted_at_ms: Date.now() - 3 * 3600_000,
    ...over,
});
const result = (rows = [row()]) => ({
    rows,
    presets: [{ id: "google-cloud-shell", label: "Google Cloud Shell", sites: ["https://shell.cloud.google.com"], note: "Not yet checked." }],
    powers: ["terminal"],
});

describe("AgentSiteGrants", () => {
    beforeEach(() => {
        for (const f of [list, set, preset, revoke, powers, listAgents]) f.mockReset();
        listAgents.mockResolvedValue([agent("naki-2", "Naki-2"), agent("agent1", "Agent1")]);
        list.mockResolvedValue(result());
    });
    afterEach(() => cleanup());

    it("lists each grant with the agent, its sites, its power and when", async () => {
        render(() => <AgentSiteGrants />);
        const li = (await screen.findByText("shell.cloud.google.com (and ssh.cloud.google.com)")).closest("li")!;
        expect(within(li).getByText("Naki-2")).toBeInTheDocument();
        expect(within(li).getByText(/granted 3 h ago/)).toBeInTheDocument();
        expect(within(li).getByRole("checkbox", { name: /Terminal.*Naki-2/ })).toBeChecked();
    });

    it("says the 2.6 second limit where the terminal power is given", async () => {
        render(() => <AgentSiteGrants />);
        expect(await screen.findByText(/2\.6 seconds/)).toBeInTheDocument();
    });

    it("revokes a grant and shows the list srv answers with", async () => {
        revoke.mockResolvedValue(result([]));
        render(() => <AgentSiteGrants />);
        fireEvent.click(await screen.findByRole("button", { name: /Revoke Naki-2/ }));
        await waitFor(() => expect(revoke).toHaveBeenCalledWith({}, { id: "r1" }));
        expect(await screen.findByText(/No agent has a standing grant/)).toBeInTheDocument();
    });

    it("turns a power off by sending the powers that stay on", async () => {
        powers.mockResolvedValue(result([row({ powers: ["clicks"] })]));
        render(() => <AgentSiteGrants />);
        fireEvent.click(await screen.findByRole("checkbox", { name: /Terminal.*Naki-2/ }));
        await waitFor(() => expect(powers).toHaveBeenCalledWith({}, { id: "r1", powers: [] }));
    });

    it("adds a grant for the chosen agent and site, with the terminal when ticked", async () => {
        set.mockResolvedValue(result());
        render(() => <AgentSiteGrants />);
        await screen.findByRole("button", { name: /Revoke/ });
        fireEvent.change(screen.getByRole("combobox", { name: "Agent to give a standing grant" }), { target: { value: "agent1" } });
        fireEvent.input(screen.getByLabelText("Site"), { target: { value: "shell.example.com" } });
        const allow = screen.getByRole("button", { name: "Allow" });
        expect(allow).not.toBeDisabled();
        fireEvent.click(screen.getAllByRole("checkbox").at(-1)!);
        fireEvent.click(allow);
        await waitFor(() => expect(set).toHaveBeenCalledWith({}, { agent: "agent1", site: "shell.example.com", powers: ["terminal"] }));
    });

    it("turns a console preset on for the chosen agent only", async () => {
        preset.mockResolvedValue(result());
        render(() => <AgentSiteGrants />);
        const button = await screen.findByRole("button", { name: "Google Cloud Shell" });
        expect(button).toBeDisabled();
        fireEvent.change(screen.getByRole("combobox", { name: "Agent to give a standing grant" }), { target: { value: "naki-2" } });
        fireEvent.click(button);
        await waitFor(() => expect(preset).toHaveBeenCalledWith({}, { preset: "google-cloud-shell", agent: "naki-2" }));
    });

    it("shows what went wrong when srv refuses", async () => {
        set.mockRejectedValue(new Error('"*.x.com" names every subdomain'));
        render(() => <AgentSiteGrants />);
        await screen.findByRole("button", { name: /Revoke/ });
        fireEvent.change(screen.getByRole("combobox", { name: "Agent to give a standing grant" }), { target: { value: "agent1" } });
        fireEvent.input(screen.getByLabelText("Site"), { target: { value: "*.x.com" } });
        fireEvent.click(screen.getByRole("button", { name: "Allow" }));
        expect(await screen.findByRole("alert")).toHaveTextContent(/names every subdomain/);
    });
});
