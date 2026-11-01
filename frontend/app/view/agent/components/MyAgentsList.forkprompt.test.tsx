// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The fork prompts of a My Agents row ("already open in another pane" and "running with no pane"):
// both are views of the row's panel (the New Agent panel's look, under the tile), both offer "Fork…",
// and, like the New Agent panel, they stay until closed: the panel's X, Escape, or the tile clicked again.

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
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
afterEach(cleanup);

const makeRow = (overrides: Partial<RecentSessionRow> = {}): RecentSessionRow => ({
    instance_id: "inst-1",
    instance_name: "Maks",
    definition_id: "def-1",
    definition_name: "Claude Code",
    provider: "claude",
    working_directory: "/tmp/maks",
    identity_id: "id-work",
    identity_name: "Work",
    memory_id: "mem-notes",
    memory_name: "Notes",
    block_id_hint: "blk-1",
    preview: "fix the hover delay",
    node_count: 1,
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

const load = (rows: RecentSessionRow[]) =>
    vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({ rows, degraded: [] });

/** The agent is open in another pane: clicking it shows the "already open" prompt. */
const mountAlreadyOpen = async () => {
    load([makeRow()]);
    const onSwitchToExisting = vi.fn();
    render(() => (
        <MyAgentsList
            onReattach={() => {}}
            onSwitchToExisting={onSwitchToExisting}
            openDefinitions={() => new Map([["def-1", "blk-other"]])}
            openLocations={() => new Map([["def-1", { blockId: "blk-other", here: false, label: "open in another pane" }]])}
        />
    ));
    await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
    await screen.findByTestId("agent-row-actions");
    return { onSwitchToExisting };
};

/** The agent runs with no pane: clicking it shows the no-pane prompt. */
const mountNoPane = async (extra: { onFork?: () => Promise<void> } = {}) => {
    load([makeRow()]);
    render(() => (
        <MyAgentsList
            onReattach={() => {}}
            onAttach={() => {}}
            onFork={extra.onFork}
            runtimeOf={() => ({
                agent_id: "def-1",
                state: "working",
                desired: "running",
                name: "Maks",
                provider: "claude",
                icon: "",
            })}
        />
    ));
    await userEvent.click(await screen.findByTestId("agent-my-agents-entry"));
    await screen.findByTestId("agent-row-actions");
};

describe("the 'already open' prompt", () => {
    it("offers Fork… and Switch to existing, and starts the fork flow from Fork…", async () => {
        await mountAlreadyOpen();
        const fork = screen.getByTestId("agent-fork-open-new");
        expect(fork.textContent).toBe("Fork…");
        expect(screen.getByTestId("agent-fork-switch").textContent).toBe("Switch to existing");
        await userEvent.click(fork);
        expect(await screen.findByTestId("agent-fork-name-input")).toBeTruthy();
    });

    it("is the row's panel: a line saying why, then the two buttons, under the tile", async () => {
        await mountAlreadyOpen();
        const panel = screen.getByTestId("agent-row-panel");
        expect(panel.dataset.view).toBe("actions");
        expect(panel.querySelector(".agent-row-panel-note")?.textContent).toContain("is already");
        expect(screen.getByTestId("agent-row-open").querySelectorAll("button")).toHaveLength(2);
        // The other actions are in the same panel, under the choice; Fork… stands for Duplicate.
        expect(screen.getByTestId("agent-row-more").textContent).not.toContain("Duplicate");
        expect(document.querySelector(".agent-recent-sessions-row")?.nextElementSibling).toBe(panel);
        expect(screen.getByTestId("agent-row-panel-close")).toBeInTheDocument();
    });

    it("picking Switch to existing hands over the open pane and closes the panel", async () => {
        const { onSwitchToExisting } = await mountAlreadyOpen();
        await userEvent.click(screen.getByTestId("agent-fork-switch"));
        expect(onSwitchToExisting).toHaveBeenCalledWith("blk-other");
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });

    it("stays on a click outside, as the New Agent panel does, and closes with its X", async () => {
        await mountAlreadyOpen();
        await userEvent.click(document.body);
        expect(screen.queryByTestId("agent-row-panel")).not.toBeNull();
        await userEvent.click(screen.getByTestId("agent-row-panel-close"));
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });

    it("closes on Escape", async () => {
        await mountAlreadyOpen();
        await userEvent.keyboard("{Escape}");
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });

    it("clicking the agent's own tile again closes the panel", async () => {
        await mountAlreadyOpen();
        await userEvent.click(screen.getByTestId("agent-my-agents-entry"));
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
        await userEvent.click(screen.getByTestId("agent-my-agents-entry"));
        expect(screen.queryByTestId("agent-row-panel")).not.toBeNull();
    });
});

describe("the 'running with no pane' prompt", () => {
    it("says Fork…, the same word as the other prompt, in the same panel", async () => {
        await mountNoPane();
        expect(screen.getByTestId("agent-fork-fork").textContent).toBe("Fork…");
        expect(screen.getByTestId("agent-fork-attach").textContent).toBe("Open it");
        const panel = screen.getByTestId("agent-row-panel");
        expect(panel.dataset.view).toBe("actions");
        expect(panel.querySelector(".agent-row-panel-note")?.textContent).toContain("no pane");
    });

    it("stays on a click outside, and closes on Escape", async () => {
        await mountNoPane();
        await userEvent.click(document.body);
        expect(screen.queryByTestId("agent-row-panel")).not.toBeNull();
        await userEvent.keyboard("{Escape}");
        expect(screen.queryByTestId("agent-row-panel")).toBeNull();
    });
});

describe("the naming step", () => {
    it("stays on a click outside while the suggested name is untouched, and Back returns to the actions", async () => {
        await mountNoPane();
        await userEvent.click(screen.getByTestId("agent-fork-fork"));
        await screen.findByTestId("agent-fork-name-input");
        // The suggestion request has failed in this mock and fallen back to a default name.
        await waitFor(() => expect((screen.getByTestId("agent-fork-name-input") as HTMLInputElement).value).not.toBe(""));
        await userEvent.click(document.body);
        expect(screen.queryByTestId("agent-fork-name-input")).not.toBeNull();
        await userEvent.click(screen.getByTestId("agent-row-panel-back"));
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("actions");
    });

    it("stays when the user has typed, however many clicks land outside", async () => {
        await mountNoPane();
        await userEvent.click(screen.getByTestId("agent-fork-fork"));
        const input = (await screen.findByTestId("agent-fork-name-input")) as HTMLInputElement;
        await waitFor(() => expect(input.value).not.toBe(""));
        await userEvent.type(input, " v2");
        await userEvent.click(document.body);
        expect(screen.queryByTestId("agent-fork-prompt")).not.toBeNull();
        expect((screen.getByTestId("agent-fork-name-input") as HTMLInputElement).value).toContain("v2");
    });

    it("stays after a failed start, with its error, when a click lands outside", async () => {
        await mountNoPane({ onFork: async () => Promise.reject(new Error("boom")) });
        await userEvent.click(screen.getByTestId("agent-fork-fork"));
        const input = (await screen.findByTestId("agent-fork-name-input")) as HTMLInputElement;
        await waitFor(() => expect(input.value).not.toBe(""));
        await userEvent.click(screen.getByTestId("agent-fork-start"));
        await screen.findByText("boom");
        await userEvent.click(document.body);
        expect(screen.queryByTestId("agent-fork-prompt")).not.toBeNull();
        expect(screen.queryByText("boom")).not.toBeNull();
    });
});
