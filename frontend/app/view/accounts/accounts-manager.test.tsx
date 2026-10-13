// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Accounts section of Connectors as cards
// (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5, §5).

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    ListIdentityAccountsCommand: vi.fn(),
    ListAllAgentIdentitiesCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    MuxBusStatusCommand: vi.fn(),
    MuxBusCloudConfigCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
const menu = vi.hoisted(() => ({ build: vi.fn(), show: vi.fn() }));
vi.mock("@/app/view/identity/bind-to-agent-menu", () => ({ buildAccountRowMenu: menu.build }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: menu.show } }));

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
    menu.build.mockResolvedValue([]);
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
        // One fetch of the agent links for every card.
        expect(rpc.ListAllAgentIdentitiesCommand).toHaveBeenCalledTimes(1);
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
});
