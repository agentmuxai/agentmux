// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Accounts section of Connectors as cards
// (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5, §5).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    ListIdentityAccountsCommand: vi.fn(),
    ListAllAgentIdentitiesCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    UnlinkAgentIdentityCommand: vi.fn(),
    MuxBusStatusCommand: vi.fn(),
    MuxBusCloudConfigCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
const menu = vi.hoisted(() => ({ build: vi.fn(), show: vi.fn() }));
vi.mock("@/app/view/identity/bind-to-agent-menu", () => ({ buildAccountRowMenu: menu.build }));
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({ ...(await original<typeof import("@/app/element/ui/open-menu-at")>()), openMenuAt: menu.show }));

import { AccountsManager } from "./accounts-manager";

function account(id: string, provider: string, email: string, extra: Record<string, unknown> = {}) {
    return {
        id,
        name: `${provider}-${id}`,
        provider,
        kind: "oauth",
        secret_ref: { backend: "oauth_config_dir" },
        context: { email },
        assigned_agents: [],
        status: "valid",
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
        ...extra,
    };
}

const ACCOUNTS = [
    account("a1", "github", "octo@example.com", { kind: "pat", status: "expired" }),
    account("a2", "claude", "me@example.com"),
    account("a3", "aws", "ops@example.com", { kind: "role", status: "invalid" }),
];

beforeEach(() => {
    rpc.ListIdentityAccountsCommand.mockResolvedValue(ACCOUNTS);
    rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([
        { agent_id: "ag1", account_id: "a2" },
        { agent_id: "ag2", account_id: "a2" },
    ]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([]);
    rpc.MuxBusStatusCommand.mockResolvedValue({ connected: true, valid: true, email: "cloud@example.com" });
    rpc.MuxBusCloudConfigCommand.mockResolvedValue({});
    menu.build.mockReturnValue([]);
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const cards = () =>
    [...document.querySelectorAll('[aria-label="Connected accounts"] .ui-card')].map((c) =>
        c.querySelector(".ui-card-title")?.textContent,
    );

describe("Accounts as cards", () => {
    it("shows AgentMux Cloud first, then accounts in the gallery's provider order", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        // Gallery order: agentmux, github, …, aws, …, anthropic (claude's brand).
        expect(cards()).toEqual(["cloud@example.com", "octo@example.com", "ops@example.com", "me@example.com"]);
    });

    it("shows status, kind and how many agents use each account", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(screen.getByText("Used by 2 agents")).toBeTruthy());
        expect(screen.getByRole("img", { name: "Expired" })).toBeTruthy();
        expect(screen.getByRole("img", { name: "Invalid" })).toBeTruthy();
        expect(screen.getByText("Token")).toBeTruthy();
        // The stored name, which the card's title (the email) hides, is its tooltip.
        expect(screen.getByRole("button", { name: /^octo@example\.com/ }).getAttribute("title")).toBe("github-a1");
        // One fetch of the agent links for every card.
        expect(rpc.ListAllAgentIdentitiesCommand).toHaveBeenCalledTimes(1);
    });

    it("numbers cards that share a provider and an email (email ·N), and a lone one stays bare", async () => {
        const seq = (id: string, n: number) =>
            account(id, "claude", "me@example.com", { context: { email: "me@example.com", seq: n } });
        rpc.ListIdentityAccountsCommand.mockResolvedValue([seq("c1", 1), seq("c2", 2), account("c3", "claude", "solo@example.com")]);
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        expect(cards()).toEqual(expect.arrayContaining(["me@example.com ·1", "me@example.com ·2", "solo@example.com"]));
        // The filter finds a card by its number too.
        fireEvent.input(screen.getByPlaceholderText("Filter accounts"), { target: { value: "·2" } });
        await waitFor(() => expect(cards()).toEqual(["me@example.com ·2"]));
    });

    it("opens adding an account under its tile, in the caret-topped section, with no dialog over the pane", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        fireEvent.click(screen.getByRole("button", { name: /^AWS/ }));
        const section = await screen.findByRole("group", { name: "Connect AWS" });
        expect(section.closest(".ui-card-detail")).toBeTruthy();
        // AWS has one way to connect (a key), so the account form is already in the section.
        expect(section.querySelector(".identity-form--inline")).toBeTruthy();
        expect(document.querySelector(".identity-form-overlay, .accounts-chooser-overlay")).toBeNull();
        expect(screen.queryByRole("dialog")).toBeNull();
        // Close collapses it.
        fireEvent.click(within(section).getAllByRole("button", { name: "Close" })[0]);
        await waitFor(() => expect(screen.queryByRole("group", { name: "Connect AWS" })).toBeNull());
    });

    it("moves the add section to another tile when the person picks one, and keeps its form open", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        fireEvent.click(screen.getByRole("button", { name: /^AWS/ }));
        await screen.findByRole("group", { name: "Connect AWS" });
        fireEvent.click(screen.getByRole("button", { name: /^OpenAI/ }));
        const section = await screen.findByRole("group", { name: "Connect OpenAI" });
        expect(screen.queryByRole("group", { name: "Connect AWS" })).toBeNull();
        expect(section.querySelector(".identity-form--inline")).toBeTruthy();
    });

    it("signs in to AgentMux in the section under its card, not in a dialog", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        fireEvent.click(document.querySelector('[aria-label="Connected accounts"] .ui-card')!);
        expect(await screen.findByText(/Connected as/)).toBeTruthy();
        expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("filters by every word of the title and subtitle", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        fireEvent.input(screen.getByPlaceholderText("Filter accounts"), { target: { value: "octo" } });
        await waitFor(() => expect(cards()).toEqual(["octo@example.com"]));
        fireEvent.input(screen.getByPlaceholderText("Filter accounts"), { target: { value: "nobody" } });
        await waitFor(() => expect(screen.getByText(/No accounts match/)).toBeTruthy());
    });

    it("opens an account's details below its card, and closes them again", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        const card = screen.getByRole("button", { name: /^octo@example\.com/ });
        fireEvent.click(card);
        await waitFor(() => expect(document.querySelector(".ui-card-detail .identity-account-panel")).toBeTruthy());
        expect(card.getAttribute("aria-expanded")).toBe("true");
        fireEvent.click(card);
        await waitFor(() => expect(document.querySelector(".ui-card-detail")).toBeNull());
    });

    it("builds the account menu on right-click", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        fireEvent.contextMenu(screen.getByRole("button", { name: /^me@example\.com/ }));
        await waitFor(() => expect(menu.show).toHaveBeenCalledTimes(1));
        expect(menu.build.mock.calls[0][0].id).toBe("a2");
    });

    it("lists the agents using an account in its details, with no Unbind (an agent switches account from its composer)", async () => {
        const link = { agent_id: "ag1", account_id: "a1", provider: "github" };
        rpc.ListAgentDefinitionsCommand.mockResolvedValue([{ id: "ag1", name: "Korp", accounts: "{}" }]);
        rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([link]);
        render(() => <AccountsManager />);
        await waitFor(() => expect(screen.getByText("Used by 1 agent")).toBeTruthy());
        fireEvent.click(screen.getByRole("button", { name: /^octo@example\.com/ }));
        const list = await screen.findByRole("list", { name: "Agents using this account" });
        expect(list.textContent).toContain("Korp");
        expect(screen.queryByRole("button", { name: "Unbind Korp" })).toBeNull();
        expect(rpc.UnlinkAgentIdentityCommand).not.toHaveBeenCalled();
    });

    it("lists an agent that is only in the old account index", async () => {
        rpc.ListAgentDefinitionsCommand.mockResolvedValue([{ id: "ag9", name: "Old", accounts: '{"aws":"a3"}' }]);
        rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([]);
        render(() => <AccountsManager />);
        await waitFor(() => expect(cards()).toHaveLength(4));
        fireEvent.click(screen.getByRole("button", { name: /^ops@example\.com/ }));
        const list = await screen.findByRole("list", { name: "Agents using this account" });
        expect(list.textContent).toContain("Old");
    });
});
