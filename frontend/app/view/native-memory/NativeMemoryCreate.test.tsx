// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * New file and Create MEMORY.md: a write reports the agent it was started for,
 * so a caller that has moved on to another agent by the time it finishes can
 * ignore it.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const writeMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { NativeMemoryWriteFileCommand: (...args: unknown[]) => writeMock(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { NativeMemoryCreate } from "./NativeMemoryCreate";

describe("NativeMemoryCreate", () => {
    afterEach(() => {
        cleanup();
        writeMock.mockReset();
    });

    it("reports the agent a write was started for, even if the agent shown has changed since", async () => {
        let finish: (v: unknown) => void = () => {};
        writeMock.mockImplementation(() => new Promise((resolve) => (finish = resolve)));
        const [agentId, setAgentId] = createSignal("agent-a");
        const onCreated = vi.fn();
        render(() => (
            <NativeMemoryCreate
                agentId={agentId()}
                filenames={[]}
                hasIndex={true}
                readOnly={false}
                listUnknown={false}
                onCreated={onCreated}
            />
        ));
        fireEvent.click(screen.getByRole("button", { name: "New file" }));
        const box = screen.getByRole("textbox");
        fireEvent.input(box, { target: { value: "late.md" } });
        fireEvent.keyDown(box, { key: "Enter" });
        await waitFor(() => expect(writeMock).toHaveBeenCalledTimes(1));

        setAgentId("agent-b");
        finish({});
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated).toHaveBeenCalledWith("late.md", "agent-a");
    });

    it("refuses to write while the folder's file list is unknown", async () => {
        render(() => (
            <NativeMemoryCreate
                agentId="agent-a"
                filenames={[]}
                hasIndex={true}
                readOnly={false}
                listUnknown={true}
                onCreated={vi.fn()}
            />
        ));
        expect(screen.getByRole("button", { name: "New file" })).toBeDisabled();
        expect(writeMock).not.toHaveBeenCalled();
    });
});
