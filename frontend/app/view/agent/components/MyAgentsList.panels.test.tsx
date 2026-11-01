// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The My Agents tile and its panel. The tile is the grid item and owns the box; its panel is a
 * full-width row after the last tile of the tile's row, the New Agent panel's own look (AgentRowPanel),
 * holding the actions and every view they lead to. Where things land on screen is measured in
 * `MyAgentsList.layout.test.tsx`; this file pins the DOM contract and the open / close behaviour.
 */

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { cleanup, render, screen, within } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MyAgentsList } from "./MyAgentsList";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListRecentSessionsCommand: vi.fn(), DeleteAgentDefinitionCommand: vi.fn() } }));
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
    model_vendor_base_url: "",
    session_id: "",
    snapshot_check_failed: false,
    agent_type: "",
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

    const panel = () => screen.queryByTestId("agent-row-panel");

    it("opens the actions panel in the list, right under the tile, and marks the tile expanded", async () => {
        await mount();
        expect(panel()).toBeNull();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        const open = await screen.findByTestId("agent-row-panel");
        expect(open.dataset.view).toBe("actions");
        expect(tile().contains(open)).toBe(false);
        expect(open.parentElement).toBe(tile().parentElement);
        expect(tile().nextElementSibling).toBe(open);
        expect(open).toHaveClass("ui-card-detail");
        expect(tile()).toHaveClass("is-expanded");
        expect(screen.getByTestId("agent-my-agents-menu-toggle")).toHaveAttribute("aria-expanded", "true");
        expect(within(open).getAllByRole("button").map((b) => b.textContent?.trim())).toEqual(
            expect.arrayContaining(["Rename", "Duplicate", "View History", "Seat…", "Bundles…", "Environment…", "Delete"]),
        );
    });

    it("closes on a second chevron click, on its close button and on Escape, not on an outside press", async () => {
        await mount();
        const toggle = screen.getByTestId("agent-my-agents-menu-toggle");
        await userEvent.click(toggle);
        await screen.findByTestId("agent-row-panel");
        await userEvent.click(toggle);
        expect(panel()).toBeNull();

        await userEvent.click(toggle);
        await screen.findByTestId("agent-row-panel");
        await userEvent.click(document.body);
        expect(panel()).not.toBeNull();

        await userEvent.click(screen.getByTestId("agent-row-panel-close"));
        expect(panel()).toBeNull();

        await userEvent.click(toggle);
        await screen.findByTestId("agent-row-panel");
        await userEvent.keyboard("{Escape}");
        expect(panel()).toBeNull();
    });

    it("Rename swaps the actions for a focused name input in the panel; Escape in it goes back to the actions, and Escape again closes", async () => {
        await mount([makeRow({ instance_name: "Maks" })]);
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        await userEvent.click(await screen.findByRole("button", { name: "Rename" }));
        expect(screen.queryByTestId("agent-row-actions")).toBeNull();
        const input = (await screen.findByTestId("agent-rename-input")) as HTMLInputElement;
        expect(screen.getByTestId("agent-row-panel").contains(input)).toBe(true);
        expect(input.value).toBe("Maks");
        expect(input).toHaveFocus();
        expect(tile()).toHaveClass("is-expanded");
        await userEvent.keyboard("{Escape}");
        expect(screen.queryByTestId("agent-rename-prompt")).toBeNull();
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("actions");
        expect(tile()).toHaveClass("is-expanded");
        await userEvent.keyboard("{Escape}");
        expect(panel()).toBeNull();
        expect(tile()).not.toHaveClass("is-expanded");
    });

    it("Back returns a sub-view to the actions", async () => {
        await mount();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        await userEvent.click(await screen.findByRole("button", { name: "Rename" }));
        await screen.findByTestId("agent-rename-input");
        await userEvent.click(screen.getByTestId("agent-row-panel-back"));
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("actions");
        expect(screen.queryByTestId("agent-rename-input")).toBeNull();
    });

    it("Duplicate opens the naming step in the panel", async () => {
        await mount();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        await userEvent.click(await screen.findByRole("button", { name: "Duplicate" }));
        const input = await screen.findByTestId("agent-fork-name-input");
        expect(screen.getByTestId("agent-row-panel").contains(input)).toBe(true);
        expect(screen.getByTestId("agent-fork-prompt")).toBeInTheDocument();
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("fork-name");
    });

    it("Delete asks in the panel, not in a modal, and Cancel goes back to the actions", async () => {
        await mount();
        await userEvent.click(screen.getByTestId("agent-my-agents-menu-toggle"));
        await userEvent.click(await screen.findByTestId("agent-my-agents-delete"));
        const confirm = await screen.findByTestId("agent-delete-confirm");
        expect(screen.getByTestId("agent-row-panel").contains(confirm)).toBe(true);
        expect(screen.queryByRole("dialog")).toBeNull();
        await userEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("actions");
        expect(RpcApi.DeleteAgentDefinitionCommand).not.toHaveBeenCalled();
    });

    it("expands one tile only, and puts the panel after it", async () => {
        await mount([
            makeRow({ instance_id: "a", definition_id: "def-a", instance_name: "Alpha" }),
            makeRow({ instance_id: "b", definition_id: "def-b", instance_name: "Beta" }),
        ]);
        const toggles = screen.getAllByTestId("agent-my-agents-menu-toggle");
        await userEvent.click(toggles[1]);
        await screen.findByTestId("agent-row-panel");
        expect(screen.getAllByTestId("agent-row-panel")).toHaveLength(1);
        const rows = [...document.querySelectorAll(".agent-recent-sessions-row")];
        expect(rows.map((r) => r.classList.contains("is-expanded"))).toEqual([false, true]);
        expect(rows[1].nextElementSibling).toBe(screen.getByTestId("agent-row-panel"));
        // Another tile's chevron moves the panel to it: the panels are alternatives.
        await userEvent.click(toggles[0]);
        expect(screen.getAllByTestId("agent-row-panel")).toHaveLength(1);
        expect(rows.map((r) => r.classList.contains("is-expanded"))).toEqual([true, false]);
    });

    it("marks an open agent's tile, not its button, as active", async () => {
        await mount([makeRow({ definition_id: "def-claude" })], {
            openDefinitions: () => new Map([["def-claude", "blk-1"]]),
        });
        expect(tile()).toHaveClass("agent-recent-sessions-row--active");
        expect(screen.getByTestId("agent-my-agents-entry")).not.toHaveClass("agent-recent-sessions-entry--active");
    });
});
