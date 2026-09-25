// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent pane → Stash → Memory: every bar that used to sit below the content
// (Edit/History, Cancel/Save, Close) is at the top now, the content or
// editor fills the bottom, saves carry their base, and a live change never
// replaces an unsaved draft. SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.

import { createHash } from "node:crypto";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const listMock = vi.fn();
const readMock = vi.fn();
const writeMock = vi.fn();
const historyMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        NativeMemoryListCommand: (...a: unknown[]) => listMock(...a),
        NativeMemoryReadFileCommand: (...a: unknown[]) => readMock(...a),
        NativeMemoryWriteFileCommand: (...a: unknown[]) => writeMock(...a),
        NativeMemoryHistoryCommand: (...a: unknown[]) => historyMock(...a),
        NativeMemoryDiffCommand: vi.fn(),
        NativeMemoryRevertCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const mpsHub = vi.hoisted(() => ({ handlers: new Map<string, () => void>() }));
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: vi.fn((sub: { eventType: string; handler: () => void }) => {
        mpsHub.handlers.set(sub.eventType, sub.handler);
        return () => mpsHub.handlers.delete(sub.eventType);
    }),
}));

import { AgentNativeMemoryModal } from "./AgentNativeMemoryModal";

const sha = (s: string) => createHash("sha256").update(s).digest("hex");
const textarea = () => document.querySelector<HTMLTextAreaElement>(".memory-content-textarea");
let saved = "v1";

beforeEach(() => {
    saved = "v1";
    for (const m of [listMock, readMock, writeMock, historyMock]) m.mockReset();
    mpsHub.handlers.clear();
    localStorage.clear();
    listMock.mockResolvedValue({
        files: [{ filename: "MEMORY.md", is_index: true, metadata_type: null, size_bytes: 2, modified_at: 0 }],
    });
    readMock.mockImplementation(() => Promise.resolve({ content: saved }));
    writeMock.mockImplementation((_c: unknown, req: { content: string }) => {
        saved = req.content;
        return Promise.resolve(null);
    });
    historyMock.mockResolvedValue({ versions: [] });
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

async function openFile(onClose?: () => void): Promise<void> {
    render(() => <AgentNativeMemoryModal agentId="a1" agentName="Manoz" workingDirectory="/w" onClose={onClose} />);
    fireEvent.click(await screen.findByText("MEMORY.md"));
    await waitFor(() => expect(screen.getByTestId("memory-pinned-bottom").textContent).toContain("v1"));
}

describe("AgentNativeMemoryModal", () => {
    test("bars on top, content at the bottom; Close sits in the header, not a footer", async () => {
        await openFile(() => undefined);
        const top = screen.getByTestId("memory-pinned-top");
        expect(top.textContent).toContain("Edit");
        expect(top.textContent).toContain("History");
        expect(screen.getByTestId("memory-pinned-bottom").querySelector("button")).toBeNull();
        expect(document.querySelector(".agent-memory-modal-footer")).toBeNull();
        expect(document.querySelector(".agent-memory-modal-header")?.textContent).toContain("Close");
    });

    test("History opens in the top region, above the content", async () => {
        await openFile();
        fireEvent.click(screen.getByText("History"));
        await waitFor(() =>
            expect(screen.getByTestId("memory-pinned-top").querySelector('[data-testid="memory-history"]')).not.toBeNull()
        );
        expect(historyMock).toHaveBeenCalledWith({}, { agent_id: "a1", filename: "MEMORY.md" });
        expect(screen.getByTestId("memory-pinned-bottom").textContent).toContain("v1");
    });

    test("Save (and Ctrl+S) writes with base_sha256", async () => {
        await openFile();
        fireEvent.click(screen.getByText("Edit"));
        fireEvent.input(textarea()!, { target: { value: "v2" } });
        fireEvent.keyDown(textarea()!, { key: "s", ctrlKey: true });

        await waitFor(() => expect(writeMock).toHaveBeenCalledTimes(1));
        expect(writeMock.mock.calls[0][1]).toEqual({
            agent_id: "a1",
            filename: "MEMORY.md",
            content: "v2",
            provenance: { source: "human" },
            base_sha256: sha("v1"),
        });
    });

    test("a live agent:memory:changed keeps a dirty draft and raises the banner when the hash moved", async () => {
        await openFile();
        fireEvent.click(screen.getByText("Edit"));
        fireEvent.input(textarea()!, { target: { value: "my draft" } });

        saved = "the agent wrote this";
        mpsHub.handlers.get("agent:memory:changed:a1")?.();

        expect(await screen.findByTestId("memory-conflict-banner", {}, { timeout: 2000 })).toHaveTextContent(
            "This file changed since you started editing."
        );
        expect(textarea()?.value).toBe("my draft");
    });

    test("a live change with no open draft updates the content in place", async () => {
        await openFile();
        saved = "fresh";
        mpsHub.handlers.get("agent:memory:changed:a1")?.();
        await waitFor(() => expect(screen.getByTestId("memory-pinned-bottom").textContent).toContain("fresh"), {
            timeout: 2000,
        });
    });
});
