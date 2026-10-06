// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The My Agents tile and its panels. The tile is the grid item and owns the
 * box; the actions menu and the Rename / Duplicate panels float in a portal
 * instead of being absolutely positioned inside it (the cause of the misplaced
 * chevron and panel: docs/reports/REPORT_MY_AGENTS_TILE_ACTIONS_ALIGNMENT_2026_10_05.md).
 * Where things actually land on screen is measured in
 * `MyAgentsList.layout.test.tsx`; this file pins the DOM contract and the
 * open / close behaviour.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MyAgentsList } from "./MyAgentsList";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListRecentSessionsCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/element/ProviderLogo", () => ({ ProviderLogo: () => null }));

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
});

afterEach(() => {
    cleanup();
});

const makeRow = (overrides: Partial<RecentSessionRow> = {}): RecentSessionRow => ({
    instance_id: "inst-1",
    instance_name: "Maks",
    definition_id: "def-claude",
    definition_name: "Claude Code",
    provider: "claude",
    working_directory: "/tmp/maks",
    identity_id: "id-work",
    identity_name: "Work",
    memory_id: "mem-notes",
    memory_name: "Notes",
    block_id_hint: "blk-1",
    preview: "fix the live-feed hover delay",
    node_count: 12,
    last_active_at: Date.now() - 60_000,
    has_snapshot: true,
    agent_created_at: Date.now() - 7_776_000_000,
    started_at: Date.now() - 3_600_000,
    ...overrides,
});

const mount = async (rows: RecentSessionRow[] = [makeRow()], extra: Record<string, unknown> = {}) => {
    vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({ rows, degraded: [] } as never);
    render(() => <MyAgentsList onReattach={() => {}} {...extra} />);
    await screen.findAllByTestId("agent-my-agents-entry");
};

const tile = () => document.querySelector<HTMLElement>(".agent-recent-sessions-row")!;

describe("MyAgentsList — tile and panels", () => {
    it("keeps the chevron and the open button in the tile, as separate controls", async () => {
        await mount();
        const entry = screen.getByTestId("agent-my-agents-entry");
        const toggle = screen.getByTestId("agent-my-agents-menu-toggle");
        expect(tile().contains(entry)).toBe(true);
        expect(tile().contains(toggle)).toBe(true);
        expect(entry.contains(toggle)).toBe(false);
        expect(toggle.contains(entry)).toBe(false);
    });

    it("opens the actions menu in a portal, outside the tile, and marks the tile expanded", async () => {
        await mount();
        expect(screen.queryByTestId("agent-row-menu")).toBeNull();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        const menu = await screen.findByTestId("agent-row-menu");
        expect(tile().contains(menu)).toBe(false);
        expect(document.body.contains(menu)).toBe(true);
        expect(menu.closest(".agent-row-menu")).not.toBeNull();
        expect(tile()).toHaveClass("is-expanded");
        expect(screen.getByTestId("agent-my-agents-menu-toggle")).toHaveAttribute("aria-expanded", "true");
    });

    it("closes the menu on a second chevron click, on an outside press and on Escape", async () => {
        await mount();
        const toggle = screen.getByTestId("agent-my-agents-menu-toggle");
        await userEvent.click(toggle);
        await screen.findByTestId("agent-row-menu");
        await userEvent.click(toggle);
        expect(screen.queryByTestId("agent-row-menu")).toBeNull();

        await userEvent.click(toggle);
        await screen.findByTestId("agent-row-menu");
        await userEvent.click(document.body);
        expect(screen.queryByTestId("agent-row-menu")).toBeNull();

        await userEvent.click(toggle);
        await screen.findByTestId("agent-row-menu");
        await userEvent.keyboard("{Escape}");
        expect(screen.queryByTestId("agent-row-menu")).toBeNull();
    });

    it("a press inside the menu does not dismiss it", async () => {
        await mount();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        const menu = await screen.findByTestId("agent-row-menu");
        await userEvent.pointer({ target: menu, keys: "[MouseLeft]" });
        expect(screen.getByTestId("agent-row-menu")).toBeInTheDocument();
    });

    it("Rename swaps the menu for a focused name input in a portal, and Escape in it cancels", async () => {
        await mount([makeRow({ instance_name: "Maks" })]);
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        await userEvent.click(await screen.findByText("Rename"));
        expect(screen.queryByTestId("agent-row-menu")).toBeNull();
        const input = (await screen.findByTestId("agent-rename-input")) as HTMLInputElement;
        expect(tile().contains(input)).toBe(false);
        expect(input.value).toBe("Maks");
        expect(input).toHaveFocus();
        expect(tile()).toHaveClass("is-expanded");
        await userEvent.keyboard("{Escape}");
        expect(screen.queryByTestId("agent-rename-prompt")).toBeNull();
        expect(tile()).not.toHaveClass("is-expanded");
    });

    it("Duplicate opens the naming panel in a portal", async () => {
        await mount();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        await userEvent.click(await screen.findByText("Duplicate"));
        const input = await screen.findByTestId("agent-fork-name-input");
        expect(tile().contains(input)).toBe(false);
        expect(screen.getByTestId("agent-fork-prompt")).toBeInTheDocument();
    });

    it("expands one tile only, and blurs the others", async () => {
        await mount([
            makeRow({ instance_id: "a", definition_id: "def-a", instance_name: "Alpha" }),
            makeRow({ instance_id: "b", definition_id: "def-b", instance_name: "Beta" }),
        ]);
        const toggles = screen.getAllByTestId("agent-my-agents-menu-toggle");
        await userEvent.click(toggles[1]);
        await screen.findByTestId("agent-row-menu");
        expect(screen.getAllByTestId("agent-row-menu")).toHaveLength(1);
        const rows = [...document.querySelectorAll(".agent-recent-sessions-row")];
        expect(rows.map((r) => r.classList.contains("is-expanded"))).toEqual([false, true]);
        expect(document.querySelector(".agent-recent-sessions-list")).toHaveClass("has-expanded-row");
    });

    it("marks an open agent's tile, not its button, as active", async () => {
        await mount([makeRow({ definition_id: "def-claude" })], {
            openDefinitions: () => new Map([["def-claude", "blk-1"]]),
        });
        expect(tile()).toHaveClass("agent-recent-sessions-row--active");
        expect(screen.getByTestId("agent-my-agents-entry")).not.toHaveClass("agent-recent-sessions-entry--active");
    });
});
