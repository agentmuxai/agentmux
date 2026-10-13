// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connectors → MCP servers as cards (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md
// §3.5, §5): the Add gallery, the two groups, a card's status from what agents
// report, details below the card with the probe running only while it's open,
// and a delete that asks first.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { McpObservedAgent } from "@/types/rpc/McpObservedAgent";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";

const rpc = vi.hoisted(() => ({
    McpCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    McpObservedListCommand: vi.fn(),
    McpCatalogDeleteCommand: vi.fn(),
    McpCatalogListForAgentCommand: vi.fn(),
    McpCatalogSetReachCommand: vi.fn(),
    McpCatalogBindCommand: vi.fn(),
    McpCatalogUpsertCommand: vi.fn(),
}));
const probe = vi.hoisted(() => ({ watch: vi.fn(), stop: vi.fn() }));
const menu = vi.hoisted(() => ({ show: vi.fn() }));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}), muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/ws", () => ({ addWSReconnectHandler: vi.fn() }));
vi.mock("@/app/store/mcp-capabilities", () => ({
    watchMcpCapability: (id: string) => {
        probe.watch(id);
        return () => probe.stop(id);
    },
    getMcpCapability: () => ({ status: "connected", toolCount: 3 }),
}));
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: { showContextMenu: menu.show },
    showTextInputContextMenu: vi.fn(),
}));

import { McpManager } from "./mcp-manager";

function server(id: string, name: string, over: Partial<McpServerCatalogItem> = {}): McpServerCatalogItem {
    return {
        id,
        name,
        transport: "stdio",
        config: "{}",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 0,
        all_agents: false,
        in_containers: false,
        ...over,
    };
}

const SERVERS = [
    server("s1", "Ableton Live", { config: '{"command":"uvx","args":["ableton-mcp"]}', bound_count: 2 }),
    server("s2", "git", {
        config: '{"command":"npx","args":["-y","git-mcp"],"env":{"GITHUB_TOKEN":"ghp_secret123"}}',
        all_agents: true,
        in_containers: true,
    }),
    server("s3", "linear", { transport: "http", config: '{"url":"https://mcp.linear.app/sse?token=abc123"}' }),
    server("s4", "notes"),
    server("s5", "fetch"),
];

function agent(label: string, servers: [string, string, string][], missing: string[] = []): McpObservedAgent {
    return {
        block_id: `blk-${label}`,
        label,
        at_ms: 1,
        servers: servers.map(([name, source, status]) => ({ name, source, status, tools: 2 })),
        missing,
    };
}

const OBSERVED = [
    agent("Korp", [
        ["agentmux", "builtin", "connected"],
        ["Ableton Live", "catalog", "connected"],
        ["git", "catalog", "connected"],
        ["linear", "catalog", "failed"],
    ]),
    agent(
        "AgentY",
        [
            ["git", "catalog", "needs-auth"],
            ["github", "claudeai", "pending"],
        ],
        ["fetch"],
    ),
];

const AGENTS = [
    { id: "a1", name: "Korp" },
    { id: "a2", name: "AgentY" },
];

async function renderManager() {
    const result = render(() => <McpManager />);
    await screen.findByRole("button", { name: "git" });
    return result;
}

const card = (name: string) => screen.getByRole("button", { name });
const dotOf = (name: string) => card(name).querySelector(".ui-status-dot")!;

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    rpc.McpCatalogListCommand.mockResolvedValue(SERVERS);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue(AGENTS);
    rpc.McpObservedListCommand.mockResolvedValue(OBSERVED);
    rpc.McpCatalogListForAgentCommand.mockImplementation((_c: unknown, { agent_id }: { agent_id: string }) =>
        Promise.resolve(agent_id === "a1" ? [{ ...SERVERS[0], bound_to_agent: true }] : []),
    );
    probe.watch.mockReset();
    probe.stop.mockReset();
    menu.show.mockReset();
});
afterEach(() => cleanup());

describe("McpManager", () => {
    it("offers each preset as a chip, saying Added for one already in the catalog", async () => {
        await renderManager();
        const gallery = screen.getByRole("group", { name: "Add an MCP server" });
        const ableton = within(gallery).getByRole("button", { name: "Ableton Live, added: open it" });
        expect(ableton.textContent).toContain("Added");
        const td = within(gallery).getByRole("button", { name: "Add TouchDesigner" });
        expect(td.textContent).not.toContain("Added");
        expect(within(gallery).getByRole("button", { name: "Add a custom server" })).toBeTruthy();

        // An added preset opens its card rather than adding it twice.
        fireEvent.click(ableton);
        expect(card("Ableton Live").getAttribute("aria-expanded")).toBe("true");

        // A new one opens the form, filled in, as the first card of the group.
        fireEvent.click(td);
        const form = await screen.findByRole("form", { name: "New MCP server" });
        expect((within(form).getByLabelText("Name") as HTMLInputElement).value).toBe("TouchDesigner");
        const grid = screen.getByRole("group", { name: "Your servers" });
        expect(grid.querySelector(".ui-card-cell")?.textContent).toContain("TouchDesigner");
    });

    it("shows the catalog under Your servers and the rest under Also in use", async () => {
        await renderManager();
        const headings = [...document.querySelectorAll(".ui-card-group-heading")].map((h) => h.textContent);
        expect(headings).toEqual(["Your servers", "Also in use"]);
        const names = (group: string) =>
            [...screen.getByRole("group", { name: group }).querySelectorAll(".ui-card-title")].map((t) => t.textContent);
        expect(names("Your servers")).toEqual(["Ableton Live", "git", "linear", "notes", "fetch"]);
        expect(names("Also in use")).toEqual(["agentmux", "github"]);
        expect(card("github").textContent).toContain("claude.ai connector");
        expect(card("github").textContent).toContain("1 agent, 1 not connected");
        expect(card("git").textContent).toContain("All agents");
        expect(card("git").textContent).toContain("Containers too");
    });

    it("shows the command or URL, never args, env or a query", async () => {
        await renderManager();
        expect(card("git").querySelector(".ui-card-subtitle")?.textContent).toBe("stdio · npx");
        expect(card("linear").querySelector(".ui-card-subtitle")?.textContent).toBe("http · mcp.linear.app/sse");
        const cards = screen.getByRole("group", { name: "Your servers" }).textContent ?? "";
        expect(cards).not.toContain("ghp_secret123");
        expect(cards).not.toContain("abc123");
    });

    it("takes a card's status from what agents report", async () => {
        await renderManager();
        expect(dotOf("Ableton Live").classList.contains("ui-status-dot--ok")).toBe(true);
        expect(dotOf("git").classList.contains("ui-status-dot--warn")).toBe(true);
        expect(dotOf("git").getAttribute("title")).toContain("Needs sign-in for AgentY");
        expect(dotOf("linear").classList.contains("ui-status-dot--error")).toBe(true);
        expect(card("linear").classList.contains("is-attention")).toBe(true);
        expect(dotOf("notes").classList.contains("ui-status-dot--idle")).toBe(true);
        // Not loaded by an agent that should have it: a warning, saying who.
        expect(dotOf("fetch").classList.contains("ui-status-dot--warn")).toBe(true);
        expect(dotOf("fetch").getAttribute("title")).toContain("Not loaded by AgentY. Restart it to apply.");
    });

    it("opens the details below the card's row, probing only while open", async () => {
        const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
            return this.classList.contains("ui-card-grid") && !this.classList.contains("ui-card-grid--chip") ? 406 : 0;
        });
        try {
            await renderManager();
            expect(probe.watch).not.toHaveBeenCalled();

            fireEvent.click(card("Ableton Live"));
            expect(probe.watch).toHaveBeenCalledWith("s1");
            const grid = screen.getByRole("group", { name: "Your servers" });
            const order = [...grid.children].map((el) =>
                el.classList.contains("ui-card-detail") ? "D" : el.getAttribute("data-card-key"),
            );
            // Two columns: the details follow the row, after git.
            expect(order).toEqual(["s1", "s2", "D", "s3", "s4", "s5"]);
            const detail = grid.querySelector(".ui-card-detail")!;
            expect(within(detail as HTMLElement).getByRole("img", { name: "Connected" })).toBeTruthy();
            expect(detail.textContent).toContain("3 tools");
            expect(within(detail as HTMLElement).getByRole("switch", { name: "All agents" })).toBeTruthy();

            fireEvent.click(card("Ableton Live"));
            expect(probe.stop).toHaveBeenCalledWith("s1");
            expect(grid.querySelector(".ui-card-detail")).toBeNull();
        } finally {
            width.mockRestore();
        }
    });

    it("shows who reports a server it doesn't manage, read-only", async () => {
        await renderManager();
        fireEvent.click(card("github"));
        const detail = screen.getByRole("group", { name: "Also in use" }).querySelector(".ui-card-detail") as HTMLElement;
        expect(within(detail).getByText("AgentY")).toBeTruthy();
        expect(within(detail).getByRole("img", { name: "starting" })).toBeTruthy();
        expect(within(detail).queryByRole("button", { name: /Delete|Edit/ })).toBeNull();
        expect(probe.watch).not.toHaveBeenCalled();
    });

    it("asks before deleting, naming the agents it's bound to", async () => {
        await renderManager();
        fireEvent.click(card("Ableton Live"));
        fireEvent.click(screen.getByRole("button", { name: "Delete…" }));

        const dialog = await screen.findByRole("dialog");
        expect(rpc.McpCatalogDeleteCommand).not.toHaveBeenCalled();
        await waitFor(() => expect(dialog.textContent).toContain("Bound to Korp."));
        expect(dialog.textContent).toContain("Running now in Korp");

        fireEvent.click(within(dialog).getByRole("button", { name: "Delete server" }));
        await waitFor(() => expect(rpc.McpCatalogDeleteCommand).toHaveBeenCalledWith({}, { id: "s1" }));
    });

    it("offers Edit, Bind to agent and Delete on right-click", async () => {
        await renderManager();
        fireEvent.contextMenu(card("git"));
        const items = menu.show.mock.calls[0][0] as ContextMenuItem[];
        expect(items.map((i) => i.label ?? i.type)).toEqual(["Edit", "Bind to agent", "separator", "Delete…"]);
        expect(items[1].submenu?.map((i) => i.label)).toEqual(["Korp", "AgentY"]);

        items[0].click?.();
        expect(await screen.findByRole("form", { name: "Edit git" })).toBeTruthy();
    });

    it("filters cards by every word", async () => {
        await renderManager();
        fireEvent.input(screen.getByLabelText("Filter servers"), { target: { value: "http linear" } });
        const titles = [...document.querySelectorAll(".ui-card-grid:not(.ui-card-grid--chip) .ui-card-title")].map(
            (t) => t.textContent,
        );
        expect(titles).toEqual(["linear"]);
    });

    it("says what goes here when there is nothing", async () => {
        rpc.McpCatalogListCommand.mockResolvedValue([]);
        rpc.McpObservedListCommand.mockResolvedValue([]);
        render(() => <McpManager />);
        expect(await screen.findByText(/No MCP servers yet/)).toBeTruthy();
        expect(document.querySelector(".ui-card-group-heading")).toBeNull();
    });
});
