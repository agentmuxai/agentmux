// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpManager, pinned as a whole before it was split into a state object, hook groups and view parts: each
 * scenario drives the Connectors → MCP servers page (the filter, the cards opening and closing in either
 * section, the Add gallery and its forms, the right-click menu, the delete confirmation, the error notice) and
 * logs, in order, the whole rendered page as a tree and every RPC the page made. The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
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
    McpCatalogUnbindCommand: vi.fn(),
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
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({
    ...(await original<typeof import("@/app/element/ui/open-menu-at")>()),
    openMenuAt: menu.show,
}));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: vi.fn() }));

import { McpManager } from "./mcp-manager";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/McpManager.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

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
            ["slack", "claudeai", "connected"],
        ],
        ["fetch"],
    ),
];

const AGENTS = [
    { id: "a1", name: "Korp", updated_at: 200 },
    { id: "a2", name: "AgentY", updated_at: 100 },
];

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|name|placeholder|for|style|href)$/;

/** The page as an indented tree: tags, classes, the attributes that carry state, form values and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    // Generated ids (fields, textareas, modal titles) count up across the whole file, and a closed modal leaves
    // `style=""` on the body: neither is the page's, and either would tie a snapshot to the order scenarios run in.
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name) && !(el === document.body && a.name === "style"))
        .map((a) => `${a.name}=${JSON.stringify(a.value.replace(/(-cl)-\d+/g, "$1-N"))}`);
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
        attrs.push(`value=${JSON.stringify(el.value)}`);
        if (el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio")) {
            attrs.push(`checked=${el.checked}`);
        }
    }
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const seen = new Map<string, number>();
/** The RPCs made since the last look, in call order per command. */
function calls() {
    for (const [name, fn] of Object.entries(rpc)) {
        const made = fn.mock.calls;
        for (let i = seen.get(name) ?? 0; i < made.length; i++) {
            log(`  rpc ${name} ${JSON.stringify(made[i].slice(1))}`);
        }
        seen.set(name, made.length);
    }
    for (const [name, fn] of Object.entries({ watch: probe.watch, stop: probe.stop })) {
        const made = fn.mock.calls;
        for (let i = seen.get("probe." + name) ?? 0; i < made.length; i++) {
            log(`  probe ${name} ${JSON.stringify(made[i])}`);
        }
        seen.set("probe." + name, made.length);
    }
}

function page(label: string) {
    log(`== ${label}`);
    dump(document.body).forEach((l) => log(l));
    calls();
}

const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    for (let i = 0; i < 8; i++) await Promise.resolve();
};

async function renderManager() {
    const result = render(() => <McpManager />);
    await flush();
    return result;
}

const card = (name: string) => screen.getByRole("button", { name });
const click = (el: Element) => fireEvent.click(el);

/** The menu the last right-click asked for. */
function menuItems(label: string, items: ContextMenuItem[], depth = 0) {
    const pad = "  ".repeat(depth + 1);
    for (const i of items) {
        log(
            `${pad}${label}: ${i.type ?? "item"} ${JSON.stringify(i.label ?? "")} icon=${i.icon ?? "-"} sub=${i.sublabel ?? "-"} enabled=${i.enabled ?? "-"}`,
        );
        if (i.submenu) menuItems(label, i.submenu, depth + 1);
    }
}

beforeEach(() => {
    T.length = 0;
    seen.clear();
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
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
        return this.classList.contains("ui-card-grid") && !this.classList.contains("ui-card-grid--chip") ? 406 : 0;
    });
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("McpManager behavior", () => {
    it("lists the catalog and what agents run, and filters both", async () => {
        await renderManager();
        page("loaded");
        fireEvent.input(screen.getByLabelText("Filter servers"), { target: { value: "http linear" } });
        page("filter: http linear");
        fireEvent.input(screen.getByLabelText("Filter servers"), { target: { value: "claude" } });
        page("filter: claude");
        fireEvent.input(screen.getByLabelText("Filter servers"), { target: { value: "zzz" } });
        page("filter: zzz");
        fireEvent.input(screen.getByLabelText("Filter servers"), { target: { value: "" } });
        page("filter cleared");
        await snapshot("list");
    });

    it("opens one card at a time across both sections", async () => {
        await renderManager();
        click(card("Ableton Live"));
        await flush();
        page("catalog card open");
        click(card("git"));
        await flush();
        page("another catalog card");
        click(card("github"));
        await flush();
        page("observed card replaces the catalog one");
        click(card("slack"));
        await flush();
        page("another observed card");
        click(card("linear"));
        await flush();
        page("catalog card replaces the observed one");
        click(card("linear"));
        await flush();
        page("catalog card toggled shut");
        click(card("agentmux"));
        click(card("agentmux"));
        await flush();
        page("observed card toggled open and shut");
        await snapshot("open");
    });

    it("closes an open card with Escape in its details, in either section", async () => {
        await renderManager();
        click(card("Ableton Live"));
        await flush();
        fireEvent.keyDown(document.querySelector(".ui-card-detail")!, { key: "Escape" });
        await flush();
        page("Escape on a catalog card's details");
        click(card("github"));
        await flush();
        fireEvent.keyDown(document.querySelector(".ui-card-detail")!, { key: "Escape" });
        await flush();
        page("Escape on an observed card's details");
        await snapshot("escape");
    });

    it("offers Edit, Bind to agent and Delete on right-click, and acts on each", async () => {
        await renderManager();
        click(card("github"));
        fireEvent.contextMenu(card("git"));
        const items = menu.show.mock.calls[0][0] as ContextMenuItem[];
        menuItems("menu", items);
        page("right-click git, observed card still open");
        items[1].submenu![1].click?.();
        await flush();
        page("bound to AgentY");
        click(card("github"));
        items[0].click?.();
        await flush();
        page("Edit: observed card closed, form open");
        fireEvent.contextMenu(card("fetch"));
        const second = menu.show.mock.calls[1][0] as ContextMenuItem[];
        second[3].click?.();
        await flush();
        page("Delete from the menu");
        await snapshot("menu");
    });

    it("has nothing to bind to when there are no agents", async () => {
        rpc.ListAgentDefinitionsCommand.mockResolvedValue([]);
        await renderManager();
        fireEvent.contextMenu(card("notes"));
        menuItems("menu", menu.show.mock.calls[0][0] as ContextMenuItem[]);
        calls();
        await snapshot("menu-no-agents");
    });

    it("adds a server from the gallery, a preset, a copy of one already added, or the button", async () => {
        await renderManager();
        click(screen.getByRole("button", { name: "Add a custom server" }));
        await flush();
        page("custom chip: blank form");
        click(screen.getByRole("button", { name: "New MCP server" }));
        await flush();
        page("the new card cancels it");
        click(screen.getByRole("button", { name: "Add TouchDesigner" }));
        await flush();
        page("preset: filled form");
        click(card("github"));
        await flush();
        page("an observed card closes the form");
        click(card("Ableton Live"));
        fireEvent.input(screen.getByLabelText("Filter servers"), { target: { value: "notes" } });
        click(screen.getByRole("button", { name: "Ableton Live, added: open it" }));
        await flush();
        page("an added preset clears the filter and opens its card");
        click(card("slack"));
        click(screen.getAllByRole("button", { name: "Add server" })[0]);
        await flush();
        page("Add server button closes the observed card and opens the form");
        await snapshot("add");
    });

    it("saves a new server, and says what is wrong first", async () => {
        await renderManager();
        click(screen.getByRole("button", { name: "Add a custom server" }));
        await flush();
        click(screen.getByRole("button", { name: "Save" }));
        await flush();
        page("save with no name");
        fireEvent.input(screen.getByLabelText("Name"), { target: { value: "  scratch  " } });
        await flush();
        page("name typed: the card takes it");
        rpc.McpCatalogUpsertCommand.mockResolvedValue({ id: "s9" });
        rpc.McpCatalogListCommand.mockResolvedValue([...SERVERS, server("s9", "scratch")]);
        click(screen.getByRole("button", { name: "Save" }));
        await flush();
        page("saved");
        await snapshot("save");
    });

    it("asks before deleting, and says who loses the server", async () => {
        await renderManager();
        click(card("Ableton Live"));
        click(screen.getByRole("button", { name: "Delete…" }));
        page("dialog just opened: bound names still loading");
        await flush();
        page("dialog: bound names and running agents");
        click(screen.getByRole("button", { name: "Cancel" }));
        await flush();
        page("cancelled");
        click(card("git"));
        click(screen.getByRole("button", { name: "Delete…" }));
        await flush();
        page("dialog for a server every agent gets");
        rpc.McpCatalogListCommand.mockResolvedValue(SERVERS.filter((s) => s.id !== "s2"));
        click(screen.getByRole("button", { name: "Delete server" }));
        await flush();
        page("deleted");
        await snapshot("delete");
    });

    it("keeps the dialog's answer for the server it was opened for, and shows a failed delete", async () => {
        await renderManager();
        click(card("Ableton Live"));
        click(screen.getByRole("button", { name: "Delete…" }));
        click(card("fetch"));
        click(screen.getAllByRole("button", { name: "Delete…" })[0]);
        await flush();
        page("second delete asked before the first answered");
        rpc.McpCatalogDeleteCommand.mockRejectedValue(new Error("nope"));
        click(screen.getByRole("button", { name: "Delete server" }));
        await flush();
        page("delete failed");
        await snapshot("delete-race");
    });

    it("lists the agents a server is bound to, and Unbind removes one", async () => {
        await renderManager();
        click(card("Ableton Live"));
        await flush();
        page("details");
        rpc.McpCatalogUnbindCommand.mockResolvedValue({ unbound: true });
        rpc.McpCatalogListCommand.mockResolvedValue([{ ...SERVERS[0], bound_count: 0 }, ...SERVERS.slice(1)]);
        rpc.McpCatalogListForAgentCommand.mockResolvedValue([]);
        click(screen.getByRole("button", { name: "Unbind Korp" }));
        await flush();
        page("unbound");
        await snapshot("unbind");
    });

    it("says what goes here when there is nothing, and shows a failed load", async () => {
        rpc.McpCatalogListCommand.mockResolvedValue([]);
        rpc.McpObservedListCommand.mockResolvedValue([]);
        await renderManager();
        page("empty");
        click(screen.getAllByRole("button", { name: "Add server" })[1]);
        await flush();
        page("Add server from the empty state");
        cleanup();
        rpc.McpCatalogListCommand.mockRejectedValue(new Error("srv down"));
        await renderManager();
        page("failed load");
        await snapshot("empty");
    });
});
