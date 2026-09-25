// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Armory → Personal Memory full view: editing through agent:memory:write_file,
// the editor pinned to the bottom, and unsaved drafts that survive live
// changes. docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4.

import { createHash } from "node:crypto";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const readMock = vi.fn();
const writeMock = vi.fn();
const historyMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NativeMemoryReadFileCommand: (...a: unknown[]) => readMock(...a),
        NativeMemoryWriteFileCommand: (...a: unknown[]) => writeMock(...a),
        NativeMemoryHistoryCommand: (...a: unknown[]) => historyMock(...a),
        NativeMemoryDiffCommand: vi.fn(),
        NativeMemoryRevertCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { NativeMemoryFileView } from "./NativeMemoryFileView";

const sha = (s: string) => createHash("sha256").update(s).digest("hex");
const textarea = () => document.querySelector<HTMLTextAreaElement>(".memory-content-textarea");

let saved = "v1";
const [nonce, setNonce] = createSignal(0);
const bump = () => setNonce((n) => n + 1);

function mount(onDirtyChange?: (d: boolean) => void) {
    return render(() => (
        <NativeMemoryFileView agentId="a1" filename="MEMORY.md" refreshNonce={nonce} onDirtyChange={onDirtyChange} />
    ));
}

async function startEditing(): Promise<void> {
    // Edit is disabled until the content has loaded.
    const edit = await screen.findByText("Edit");
    await waitFor(() => expect(edit).not.toBeDisabled());
    fireEvent.click(edit);
    await waitFor(() => expect(textarea()).not.toBeNull());
}

beforeEach(() => {
    saved = "v1";
    readMock.mockReset();
    writeMock.mockReset();
    historyMock.mockReset();
    readMock.mockImplementation(() => Promise.resolve({ content: saved }));
    writeMock.mockImplementation((_c: unknown, req: { content: string }) => {
        saved = req.content;
        return Promise.resolve(null);
    });
    historyMock.mockResolvedValue({ versions: [] });
    localStorage.clear();
});

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("NativeMemoryFileView", () => {
    test("pins the editor to the bottom: actions + history on top, content below", async () => {
        mount();
        const top = await screen.findByTestId("memory-pinned-top");
        await waitFor(() => expect(screen.getByTestId("memory-pinned-bottom").textContent).toContain("v1"));
        expect(top.textContent).toContain("Edit");
        expect(top.querySelector('[data-testid="memory-history"]')).not.toBeNull();
        expect(screen.getByTestId("memory-pinned-bottom").querySelector("button")).toBeNull();
    });

    test("Edit opens an editor on the current content; Save writes it with base_sha256 and a human provenance", async () => {
        mount();
        await startEditing();
        expect(textarea()?.value).toBe("v1");
        fireEvent.input(textarea()!, { target: { value: "v2" } });
        fireEvent.click(screen.getByText("Save"));

        await waitFor(() => expect(writeMock).toHaveBeenCalledTimes(1));
        expect(writeMock.mock.calls[0][1]).toEqual({
            agent_id: "a1",
            filename: "MEMORY.md",
            content: "v2",
            provenance: { source: "human" },
            base_sha256: sha("v1"),
        });
        await waitFor(() => expect(textarea()).toBeNull());
    });

    test("Ctrl+S and Cmd+S save from inside the editor", async () => {
        mount();
        await startEditing();
        fireEvent.input(textarea()!, { target: { value: "v2" } });
        fireEvent.keyDown(textarea()!, { key: "s", ctrlKey: true });
        await waitFor(() => expect(writeMock).toHaveBeenCalledTimes(1));

        await startEditing();
        fireEvent.input(textarea()!, { target: { value: "v3" } });
        fireEvent.keyDown(textarea()!, { key: "S", metaKey: true });
        await waitFor(() => expect(writeMock).toHaveBeenCalledTimes(2));
        expect(writeMock.mock.calls[1][1]).toMatchObject({ content: "v3", base_sha256: sha("v2") });
    });

    test("Esc on a dirty draft asks before discarding", async () => {
        mount();
        await startEditing();
        fireEvent.input(textarea()!, { target: { value: "dirty" } });
        const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
        fireEvent.keyDown(textarea()!, { key: "Escape" });
        expect(confirmSpy).toHaveBeenCalledTimes(1);
        expect(textarea()?.value).toBe("dirty");
    });

    test("reports dirtiness to the manager (which guards Back)", async () => {
        const reports: boolean[] = [];
        mount((d) => reports.push(d));
        await startEditing();
        fireEvent.input(textarea()!, { target: { value: "dirty" } });
        await waitFor(() => expect(reports.at(-1)).toBe(true));
    });

    describe("a live change (refreshNonce bump) while editing", () => {
        test("keeps a dirty draft and shows the banner when the file's hash moved", async () => {
            mount();
            await startEditing();
            fireEvent.input(textarea()!, { target: { value: "my draft" } });

            saved = "the agent wrote this";
            bump();

            expect(await screen.findByTestId("memory-conflict-banner")).toHaveTextContent(
                "This file changed since you started editing."
            );
            expect(textarea()?.value).toBe("my draft");
        });

        test("stays silent when the file's hash still equals the draft's base", async () => {
            mount();
            await startEditing();
            fireEvent.input(textarea()!, { target: { value: "my draft" } });

            // Another file of the same agent changed: this one re-reads identical.
            const readsBefore = readMock.mock.calls.length;
            bump();
            await waitFor(() => expect(readMock.mock.calls.length).toBeGreaterThan(readsBefore));
            await new Promise((r) => setTimeout(r, 20));

            expect(screen.queryByTestId("memory-conflict-banner")).toBeNull();
            expect(textarea()?.value).toBe("my draft");
        });

        test("follows along silently when the draft has no changes yet", async () => {
            mount();
            await startEditing();
            saved = "updated underneath";
            bump();
            await waitFor(() => expect(textarea()?.value).toBe("updated underneath"));
            expect(screen.queryByTestId("memory-conflict-banner")).toBeNull();
        });

        test("refreshes the read-only content in place when not editing", async () => {
            mount();
            await waitFor(() => expect(screen.getByTestId("memory-pinned-bottom").textContent).toContain("v1"));
            saved = "fresh";
            bump();
            await waitFor(() => expect(screen.getByTestId("memory-pinned-bottom").textContent).toContain("fresh"));
        });
    });

    test("a save refused as a conflict keeps the draft; Save anyway rebases onto what's saved now", async () => {
        mount();
        await startEditing();
        fireEvent.input(textarea()!, { target: { value: "my draft" } });
        saved = "someone else";
        writeMock.mockRejectedValueOnce(new Error("agent:memory:write_file: conflict: MEMORY.md changed since your edit began"));
        fireEvent.click(screen.getByText("Save"));

        expect(await screen.findByTestId("memory-conflict-banner")).toHaveTextContent("Not saved");
        expect(textarea()?.value).toBe("my draft");

        await waitFor(() => expect(screen.getByText("Save anyway")).not.toBeDisabled());
        fireEvent.click(screen.getByText("Save anyway"));
        await waitFor(() => expect(writeMock).toHaveBeenCalledTimes(2));
        expect(writeMock.mock.calls[1][1]).toMatchObject({ content: "my draft", base_sha256: sha("someone else") });
    });
});
