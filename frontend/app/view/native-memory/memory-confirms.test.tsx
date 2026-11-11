// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Adopting an agent's earlier memory, and releasing its claim on a folder, is confirmed in a modal
 * over the Memory pane whose button was pressed, and only a yes there reaches srv
 * (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §6 row 8).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = {
    list: vi.fn(),
    adopt: vi.fn(),
    claims: vi.fn(),
    release: vi.fn(),
};
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NativeMemoryAdoptionListCommand: (...a: unknown[]) => rpc.list(...a),
        NativeMemoryAdoptCommand: (...a: unknown[]) => rpc.adopt(...a),
        NativeMemoryClaimsCommand: (...a: unknown[]) => rpc.claims(...a),
        NativeMemoryReleaseCommand: (...a: unknown[]) => rpc.release(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: () => {}, endWaitingForYou: () => {} }));
// The panes that exist, by id: a test deletes one by setting it to undefined.
const panes = new Map<string, ReturnType<typeof createSignal<object | undefined>>>();
const pane = (id: string) => {
    if (!panes.has(id)) panes.set(id, createSignal<object | undefined>({ oid: id }));
    return panes.get(id)!;
};
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: (key: string) => pane(key.replace("block:", ""))[0] },
}));

import { ModalLayer } from "@/app/element/ModalLayer";
import { __resetPaneRequests, paneRequests } from "@/app/store/pane-requests";
import { PaneRequestModals } from "@/app/view/agent/pane-requests/PaneRequestDock";
import { MemoryAdoptionPanel } from "./MemoryAdoptionPanel";
import { MemoryClaimsPanel } from "./MemoryClaimsPanel";

const file = (name: string) => ({ name, size: 10, modified_ms: 1, sha256: "s" });
const LIST = {
    list_id: "L1",
    agent_uid: "a1",
    candidates: [
        { index: 0, account: "e562b87a-0000", dir_hash: "h0", files: [file("project_notes.md"), file("MEMORY.md")] },
        { index: 1, account: "held", dir_hash: "h1", files: [file("held.md")] },
    ],
};
const CLAIMS = { list_id: "C1", agent_uid: "a1", folders: [{ index: 0, dir: "/home/u/.claude/projects/x/memory", claimed_at_ms: 0, others: 0 }] };

const onAdopted = vi.fn();

/** The Memory pane and another pane, each with the block frame's modal host. */
function Window() {
    return (
        <div>
            <div data-blockid="memory" data-testid="pane-memory">
                <ModalLayer scope="pane" fill>
                    <PaneRequestModals blockId="memory" />
                    <MemoryAdoptionPanel agentId="a1" agentName="Opaz" onAdopted={onAdopted} />
                    <MemoryClaimsPanel agentId="a1" agentName="Opaz" />
                </ModalLayer>
            </div>
            <div data-blockid="other" data-testid="pane-other">
                <ModalLayer scope="pane" fill>
                    <PaneRequestModals blockId="other" />
                </ModalLayer>
            </div>
        </div>
    );
}

const requests = () => paneRequests.of("memory").filter((r) => r.source === "memory-confirm");

/** Open the earlier-memory list, tick its first folder and press Adopt selected. */
async function askToAdopt() {
    fireEvent.click(await screen.findByRole("button", { name: /Earlier memory found under 1 other account/ }));
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    fireEvent.click(screen.getByRole("button", { name: "Adopt selected…" }));
    return screen.findByText("Adopt earlier memory into Opaz?");
}

async function askToRelease() {
    fireEvent.click(await screen.findByRole("button", { name: /Memory folders this agent claims \(1\)/ }));
    fireEvent.click(screen.getByRole("button", { name: "Release…" }));
    return screen.findByText("Release Opaz's claim on this folder?");
}

beforeEach(() => {
    __resetPaneRequests();
    panes.clear();
    for (const f of Object.values(rpc)) f.mockReset();
    onAdopted.mockReset();
    rpc.list.mockResolvedValue({ list: LIST });
    rpc.claims.mockResolvedValue(CLAIMS);
    rpc.adopt.mockResolvedValue({ files_added: 2, versions_kept: 0, index_lines_added: 1 });
    rpc.release.mockResolvedValue({ released: true });
});

afterEach(() => cleanup());

describe("adopting earlier memory", () => {
    it("asks in the Memory pane, naming the account and its files, and nothing is adopted yet", async () => {
        const { getByTestId } = render(() => <Window />);
        const title = await askToAdopt();
        expect(requests()).toHaveLength(1);
        expect(requests()[0].presentation).toBe("modal");
        expect(getByTestId("pane-memory").contains(title)).toBe(true);
        expect(getByTestId("pane-other").contains(title)).toBe(false);
        const modal = title.closest("[role=dialog],[role=alertdialog]")!;
        expect(modal.textContent).toContain("Account e562b87a");
        expect(modal.textContent).toContain("project_notes.md");
        expect(modal.textContent).not.toContain("held.md");
        expect(rpc.adopt).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: "Waiting for confirmation…" })).toBeTruthy();
        // The person's own click, like adding a host: the accent button, not the tone for an agent that waits.
        expect(screen.getByRole("button", { name: "Adopt" }).className).toContain("ui-tone-accent");
        expect(screen.getByRole("button", { name: "Adopt" }).className).not.toContain("ui-tone-attention");
    });

    it("adopts on Adopt, with the list's own ids, then says what it did", async () => {
        render(() => <Window />);
        await askToAdopt();
        fireEvent.click(screen.getByRole("button", { name: "Adopt" }));
        await vi.waitFor(() =>
            expect(rpc.adopt).toHaveBeenCalledWith(
                {},
                { agent_id: "a1", list_id: "L1", choices: [{ index: 0, dir_hash: "h0" }] },
                { timeout: 30_000 }
            )
        );
        expect(await screen.findByText(/2 file\(s\) added, 1 index line\(s\) added/)).toBeTruthy();
        await vi.waitFor(() => expect(requests()).toHaveLength(0));
        expect(onAdopted).toHaveBeenCalledTimes(1);
        // The list is read again, so an adopted folder is no longer offered.
        await vi.waitFor(() => expect(rpc.list).toHaveBeenCalledTimes(2));
    });

    it("adopts nothing on Cancel", async () => {
        render(() => <Window />);
        await askToAdopt();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(await screen.findByText("Adoption cancelled.")).toBeTruthy();
        expect(requests()).toHaveLength(0);
        expect(rpc.adopt).not.toHaveBeenCalled();
        expect(onAdopted).not.toHaveBeenCalled();
    });

    it("adopts nothing when the pane is closed with the question open", async () => {
        render(() => <Window />);
        pane("memory");
        await askToAdopt();
        pane("memory")[1](undefined);
        await vi.waitFor(() => expect(requests()).toHaveLength(0));
        expect(rpc.adopt).not.toHaveBeenCalled();
    });

    it("says to choose again when srv finds a folder changed since the list", async () => {
        rpc.adopt.mockRejectedValueOnce(new Error("changed: folder 0 changed since the list was issued; list again"));
        render(() => <Window />);
        await askToAdopt();
        fireEvent.click(screen.getByRole("button", { name: "Adopt" }));
        expect(await screen.findByText(/A folder changed since it was listed/)).toBeTruthy();
        expect(onAdopted).not.toHaveBeenCalled();
    });

    it("says so when it is in no pane, and asks nothing", async () => {
        render(() => <MemoryAdoptionPanel agentId="a1" agentName="Opaz" />);
        fireEvent.click(await screen.findByRole("button", { name: /Earlier memory found/ }));
        fireEvent.click(screen.getAllByRole("checkbox")[0]);
        fireEvent.click(screen.getByRole("button", { name: "Adopt selected…" }));
        expect(await screen.findByText("Couldn't ask: Memory isn't open in a pane.")).toBeTruthy();
        expect(rpc.adopt).not.toHaveBeenCalled();
    });
});

describe("releasing a claimed memory folder", () => {
    it("asks in the Memory pane, naming the folder, and releases on Release", async () => {
        const { getByTestId } = render(() => <Window />);
        const title = await askToRelease();
        expect(getByTestId("pane-memory").contains(title)).toBe(true);
        const modal = title.closest("[role=dialog],[role=alertdialog]")!;
        expect(modal.textContent).toContain("/home/u/.claude/projects/x/memory");
        expect(rpc.release).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Release" }));
        await vi.waitFor(() =>
            expect(rpc.release).toHaveBeenCalledWith({}, { agent_id: "a1", list_id: "C1", index: 0 })
        );
        expect(await screen.findByText(/^Released\./)).toBeTruthy();
        await vi.waitFor(() => expect(requests()).toHaveLength(0));
    });

    it("releases nothing on Cancel", async () => {
        render(() => <Window />);
        await askToRelease();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(await screen.findByText("Release cancelled.")).toBeTruthy();
        expect(rpc.release).not.toHaveBeenCalled();
    });
});
