// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentBundlesTab — the Stash's Bundles tab: the agent's own bundle
 * first and fixed, then its picks, saved on each change
 * (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6).
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listBundles = vi.fn();
const getAgentBundles = vi.fn();
const setAgentBundles = vi.fn();
const openMemory = vi.fn();

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListBundlesCommand: (...args: unknown[]) => listBundles(...args),
        GetAgentBundlesCommand: (...args: unknown[]) => getAgentBundles(...args),
        SetAgentBundlesCommand: (...args: unknown[]) => setAgentBundles(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/section-pane/panes", () => ({
    openMemory: (...args: unknown[]) => openMemory(...args),
}));

import { AgentBundlesTab } from "./AgentBundlesTab";
import type { Bundle } from "@/app/store/rpc-api";

function mkBundle(overrides: Partial<Bundle>): Bundle {
    return {
        id: "bundle-1",
        name: "Code Reviewer",
        is_blank: false,
        is_global: false,
        is_system: false,
        instructions: "Review the diff for bugs.",
        ...overrides,
    } as Bundle;
}

const BUNDLES = [
    mkBundle({ id: "blank", name: "Vanilla CLI", is_blank: true }),
    mkBundle({ id: "sys", name: "Operator Config", is_system: true }),
    mkBundle({ id: "own", name: "Mary — ABF" }),
    mkBundle({ id: "b1", name: "Code Reviewer" }),
    mkBundle({ id: "b2", name: "Release Notes Writer" }),
];

const names = () => screen.queryAllByTestId("bundle-list-editor-item").map((row) => row.textContent);

/** The list has loaded and its controls are enabled. */
async function ready(expected: string[]) {
    await waitFor(() => expect(names()).toEqual(expected));
    await waitFor(() => expect((screen.getByTestId("bundle-list-editor-add") as HTMLSelectElement).disabled).toBe(false));
}

describe("AgentBundlesTab", () => {
    beforeEach(() => {
        listBundles.mockReset().mockResolvedValue(BUNDLES);
        getAgentBundles.mockReset().mockResolvedValue({ bundle_ids: ["b2"], own_bundle_id: "own" });
        setAgentBundles
            .mockReset()
            .mockImplementation(async (_c: unknown, d: { bundle_ids: string[] }) => ({
                bundle_ids: d.bundle_ids,
                own_bundle_id: "own",
            }));
        openMemory.mockReset();
    });

    afterEach(() => cleanup());

    it("shows the own bundle first and fixed, then the picks", async () => {
        render(() => <AgentBundlesTab agentId="agent-1" />);
        await waitFor(() => expect(screen.getByText("Mary — ABF")).toBeInTheDocument());
        expect(screen.getByText("this agent's own")).toBeInTheDocument();
        await waitFor(() => expect(names()).toEqual(["Release Notes Writer"]));

        // Only what can still be added: not blank, system, own or picked.
        const add = screen.getByTestId("bundle-list-editor-add") as HTMLSelectElement;
        const options = Array.from(add.options).filter((o) => o.value).map((o) => o.textContent);
        expect(options).toEqual(["Code Reviewer"]);
    });

    it("saves the list on each change", async () => {
        render(() => <AgentBundlesTab agentId="agent-1" />);
        await ready(["Release Notes Writer"]);

        fireEvent.change(screen.getByTestId("bundle-list-editor-add"), { target: { value: "b1" } });
        await waitFor(() =>
            expect(setAgentBundles).toHaveBeenCalledWith({}, { agent_id: "agent-1", bundle_ids: ["b2", "b1"] }),
        );
        await waitFor(() => expect(names()).toEqual(["Release Notes Writer", "Code Reviewer"]));

        fireEvent.click(screen.getAllByRole("button", { name: "Move up" })[1]);
        await waitFor(() =>
            expect(setAgentBundles).toHaveBeenLastCalledWith({}, { agent_id: "agent-1", bundle_ids: ["b1", "b2"] }),
        );

        fireEvent.click(screen.getAllByRole("button", { name: "Remove" })[0]);
        await waitFor(() =>
            expect(setAgentBundles).toHaveBeenLastCalledWith({}, { agent_id: "agent-1", bundle_ids: ["b2"] }),
        );
    });

    it("keeps the list it had when a save fails, and says why", async () => {
        setAgentBundles.mockRejectedValue(new Error("disk full"));
        render(() => <AgentBundlesTab agentId="agent-1" />);
        await ready(["Release Notes Writer"]);

        fireEvent.click(screen.getByRole("button", { name: "Remove" }));
        await waitFor(() => expect(screen.getByText(/disk full/)).toBeInTheDocument());
        expect(names()).toEqual(["Release Notes Writer"]);
    });

    it("names a listed bundle that was deleted, so it can be removed", async () => {
        getAgentBundles.mockResolvedValue({ bundle_ids: ["gone"], own_bundle_id: "own" });
        render(() => <AgentBundlesTab agentId="agent-1" />);
        await waitFor(() => expect(names()).toEqual(["Deleted bundle"]));
    });

    it("links to Memory → Bundles for editing", async () => {
        render(() => <AgentBundlesTab agentId="agent-1" />);
        fireEvent.click(await screen.findByText("Memory → Bundles"));
        expect(openMemory).toHaveBeenCalledWith("bundles");
    });
});
