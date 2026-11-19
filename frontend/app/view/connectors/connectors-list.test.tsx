// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connectors' list as cards (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4, §8).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    ListIdentityAccountsCommand: vi.fn(),
    ListAllAgentIdentitiesCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    UnlinkAgentIdentityCommand: vi.fn(),
    MuxBusStatusCommand: vi.fn(),
    MuxBusCloudConfigCommand: vi.fn(),
    GithubConnectionCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
const menu = vi.hoisted(() => ({ build: vi.fn(), show: vi.fn() }));
vi.mock("@/app/view/identity/bind-to-agent-menu", () => ({ buildAccountRowMenu: menu.build }));
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({
    ...(await original<typeof import("@/app/element/ui/open-menu-at")>()),
    openMenuAt: menu.show,
}));

import { ConnectorsList } from "./connectors-list";

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
    rpc.GithubConnectionCommand.mockResolvedValue({});
    menu.build.mockReturnValue([]);
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const names = (group: string) =>
    [...document.querySelectorAll(`[aria-label="${group}"] .connector-card-name`)].map((n) => n.textContent);
const connectedNames = () => names("Connected");
const availableNames = () => names("Available");
// A connected card's disclosure button (the "Other agents" lines under the list have buttons named for providers too).
const disclosure = (name: RegExp) =>
    screen.getAllByRole("button", { name }).find((b) => b.classList.contains("connector-card-disclosure"))!;
const ready = () => waitFor(() => expect(connectedNames()).toHaveLength(4));

describe("Connectors as cards", () => {
    it("shows what is connected first (AgentMux Cloud, then the catalog's order), then what is available", async () => {
        render(() => <ConnectorsList />);
        await ready();
        expect(connectedNames()).toEqual(["AgentMux Cloud", "GitHub", "AWS", "Anthropic"]);
        expect(availableNames()).toEqual(["Google", "OpenAI", "Slack", "Custom", "xAI", "Moonshot"]);
    });

    it("says each state in words, and what each other connector is for", async () => {
        render(() => <ConnectorsList />);
        await ready();
        const line = (key: string) => document.querySelector(`[data-connector="${key}"] .connector-card-line`)?.textContent;
        expect(line("agentmux")).toContain("Connected · cloud@example.com");
        expect(line("github")).toContain("Needs sign-in again · octo@example.com");
        expect(line("aws")).toContain("Couldn't connect · ops@example.com");
        expect(line("anthropic")).toContain("Connected · me@example.com");
        expect(line("google")).toBe("Gmail, Calendar and Drive for your agents");
        // No Connect button for an agent that signs in inside its own pane.
        expect(screen.queryByRole("button", { name: "Connect xAI" })).toBeNull();
        expect(screen.getByRole("button", { name: "How to connect xAI" })).toBeTruthy();
    });

    it("opens a connected card's details under its row, labelled by the card, and closes them again", async () => {
        render(() => <ConnectorsList />);
        await ready();
        const github = disclosure(/^GitHub/);
        expect(github.getAttribute("aria-expanded")).toBe("false");
        fireEvent.click(github);
        const region = await screen.findByRole("region", { name: "GitHub details" });
        expect(region.closest(".ui-card-detail")).toBe(region);
        expect(github.getAttribute("aria-expanded")).toBe("true");
        expect(github.getAttribute("aria-controls")).toBe(region.id);
        // One account: its details show at once.
        await waitFor(() => expect(region.querySelector(".identity-account-panel")).toBeTruthy());
        fireEvent.click(github);
        await waitFor(() => expect(screen.queryByRole("region", { name: "GitHub details" })).toBeNull());
    });

    it("opens by clicking the card itself, not only its button", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(document.querySelector('[data-connector="github"]')!);
        expect(await screen.findByRole("region", { name: "GitHub details" })).toBeTruthy();
    });

    it("shows the agents using an account in its details, with no Unbind", async () => {
        const link = { agent_id: "ag1", account_id: "a1", provider: "github" };
        rpc.ListAgentDefinitionsCommand.mockResolvedValue([{ id: "ag1", name: "Korp", accounts: "{}" }]);
        rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([link]);
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(disclosure(/^GitHub/));
        const list = await screen.findByRole("list", { name: "Agents using this account" });
        expect(list.textContent).toContain("Korp");
        expect(screen.queryByRole("button", { name: "Unbind Korp" })).toBeNull();
        expect(rpc.UnlinkAgentIdentityCommand).not.toHaveBeenCalled();
    });

    it("lists an agent that is only in the old account index", async () => {
        rpc.ListAgentDefinitionsCommand.mockResolvedValue([{ id: "ag9", name: "Old", accounts: '{"aws":"a3"}' }]);
        rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([]);
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(disclosure(/^AWS/));
        const list = await screen.findByRole("list", { name: "Agents using this account" });
        expect(list.textContent).toContain("Old");
    });

    it("shows each account of a connector with several, and how many agents use it", async () => {
        rpc.ListIdentityAccountsCommand.mockResolvedValue([
            account("g1", "github", "one@example.com", { kind: "pat" }),
            account("g2", "github", "two@example.com", { kind: "pat", status: "expired" }),
        ]);
        rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([{ agent_id: "ag1", account_id: "g2" }]);
        render(() => <ConnectorsList />);
        await waitFor(() => expect(document.querySelector('[data-connector="github"]')?.classList.contains("is-connected")).toBe(true));
        expect(document.querySelector('[data-connector="github"] .connector-card-line')?.textContent).toContain(
            "2 accounts · 1 needs sign-in again",
        );
        fireEvent.click(disclosure(/^GitHub/));
        const accounts = await screen.findByRole("group", { name: "GitHub accounts" });
        expect([...accounts.querySelectorAll(".ui-card-title")].map((n) => n.textContent)).toEqual(["one@example.com", "two@example.com"]);
        expect(within(accounts).getByText("Used by 1 agent")).toBeTruthy();
        // Several accounts: none is open until one is picked.
        expect(document.querySelector(".identity-account-panel")).toBeNull();
        fireEvent.click(within(accounts).getByRole("button", { name: /^two@example\.com/ }));
        await waitFor(() => expect(document.querySelector(".identity-account-panel")).toBeTruthy());
    });

    it("builds the account menu on right-click of an account in the details", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(disclosure(/^Anthropic/));
        const card = await screen.findByRole("button", { name: /^me@example\.com/ });
        fireEvent.contextMenu(card);
        await waitFor(() => expect(menu.show).toHaveBeenCalledTimes(1));
        expect(menu.build.mock.calls[0][0].id).toBe("a2");
    });

    it("moves between cards: opening another closes the first", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(disclosure(/^GitHub/));
        await screen.findByRole("region", { name: "GitHub details" });
        fireEvent.click(disclosure(/^AWS/));
        await screen.findByRole("region", { name: "AWS details" });
        expect(screen.queryByRole("region", { name: "GitHub details" })).toBeNull();
        expect(disclosure(/^GitHub/).getAttribute("aria-expanded")).toBe("false");
    });

    it("signs in to AgentMux Cloud in the details under its card, not in a dialog", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(disclosure(/^AgentMux Cloud/));
        expect(await screen.findByText(/Connected as/)).toBeTruthy();
        expect(screen.queryByRole("dialog")).toBeNull();
    });
});

describe("Connect", () => {
    it("opens the add section under the card, in the caret-topped details, with no dialog over the pane", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(screen.getByRole("button", { name: "Connect Custom" }));
        const section = await screen.findByRole("group", { name: "Connect Custom" });
        expect(section.closest(".ui-card-detail")).toBeTruthy();
        // Custom has one way to connect (a key), so the account form is already in the section.
        expect(section.querySelector(".identity-form--inline")).toBeTruthy();
        expect(document.querySelector(".identity-form-overlay, .accounts-chooser-overlay")).toBeNull();
        expect(screen.queryByRole("dialog")).toBeNull();
        // Close collapses it.
        fireEvent.click(within(section).getAllByRole("button", { name: "Close" })[0]);
        await waitFor(() => expect(screen.queryByRole("group", { name: "Connect Custom" })).toBeNull());
    });

    it("moves the add section to another card when the person picks one, and keeps its form open", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(screen.getByRole("button", { name: "Connect Custom" }));
        await screen.findByRole("group", { name: "Connect Custom" });
        fireEvent.click(screen.getByRole("button", { name: "Connect Slack" }));
        const section = await screen.findByRole("group", { name: "Connect Slack" });
        expect(screen.queryByRole("group", { name: "Connect Custom" })).toBeNull();
        expect(section.querySelector(".identity-form--inline, .accounts-add-status, .oauth-connect")).toBeTruthy();
    });

    it("offers OpenAI's ChatGPT sign-in as well as its API key, with the explanation", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(screen.getByRole("button", { name: "Connect OpenAI" }));
        const section = await screen.findByRole("group", { name: "Connect OpenAI" });
        expect(within(section).getByRole("button", { name: /Sign in with ChatGPT/ })).toBeTruthy();
        expect(within(section).getByRole("button", { name: /API key/ })).toBeTruthy();
        expect(within(section).getByText("Which one should I pick?")).toBeTruthy();
        // Nothing opens until one is picked.
        expect(section.querySelector(".identity-form--inline")).toBeNull();
    });

    it("opens how to connect for an agent that signs in inside its own pane", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(screen.getByRole("button", { name: "How to connect xAI" }));
        expect(await screen.findByRole("group", { name: "Connect xAI" })).toBeTruthy();
    });

    it("opens Add another account in a connected card's details", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(disclosure(/^GitHub/));
        await screen.findByRole("region", { name: "GitHub details" });
        fireEvent.click(screen.getByRole("button", { name: "Add another account" }));
        expect(await screen.findByRole("group", { name: "Connect GitHub" })).toBeTruthy();
    });

    it("opens Add a custom connector from the foot", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.click(screen.getByRole("button", { name: "Add a custom connector" }));
        expect(await screen.findByRole("group", { name: "Connect Custom" })).toBeTruthy();
    });
});

describe("the filter", () => {
    it("matches every word over a card's name, line, state and accounts", async () => {
        render(() => <ConnectorsList />);
        await ready();
        fireEvent.input(screen.getByPlaceholderText("Filter connectors"), { target: { value: "octo" } });
        await waitFor(() => expect(connectedNames()).toEqual(["GitHub"]));
        expect(availableNames()).toEqual([]);
        fireEvent.input(screen.getByPlaceholderText("Filter connectors"), { target: { value: "gmail" } });
        await waitFor(() => expect(availableNames()).toEqual(["Google"]));
        fireEvent.input(screen.getByPlaceholderText("Filter connectors"), { target: { value: "nobody" } });
        await waitFor(() => expect(screen.getByText(/No connectors match/)).toBeTruthy());
    });
});

describe("with nothing connected", () => {
    it("lists every connector as available, in the catalog's order", async () => {
        rpc.ListIdentityAccountsCommand.mockResolvedValue([]);
        rpc.MuxBusStatusCommand.mockResolvedValue({ connected: false, valid: false });
        render(() => <ConnectorsList />);
        await waitFor(() => expect(availableNames()).toHaveLength(10));
        expect(availableNames()).toEqual([
            "AgentMux Cloud", "GitHub", "Google", "AWS", "OpenAI", "Anthropic", "Slack", "Custom", "xAI", "Moonshot",
        ]);
        expect(document.querySelector('[aria-label="Connected"]')).toBeNull();
    });
});
