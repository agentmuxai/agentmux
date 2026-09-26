// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";

const sourcesMock = vi.fn();
const importMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        GlobalMemoryImportSourcesCommand: (...a: unknown[]) => sourcesMock(...a),
        GlobalMemoryImportCommand: (...a: unknown[]) => importMock(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { GlobalMemoryImportBanner, scopeLabel } from "./GlobalMemoryImportBanner";

describe("GlobalMemoryImportBanner", () => {
    beforeEach(() => {
        sourcesMock.mockReset();
        importMock.mockReset();
    });

    it("renders nothing when there is nothing to bring", async () => {
        sourcesMock.mockResolvedValue({ list_id: "l", scope: "channel:dev", sources: [] });
        const { container } = render(() => <GlobalMemoryImportBanner />);
        await waitFor(() => expect(sourcesMock).toHaveBeenCalled());
        expect(container.querySelector(".global-memory-import")).toBeNull();
    });

    it("offers what the main Global Memory has, and brings it here", async () => {
        sourcesMock.mockResolvedValueOnce({
            list_id: "l1",
            scope: "channel:dev",
            sources: [{ index: 0, scope: "shared", missing: [{ entry_id: "g1", name: "Rules" }] }],
        });
        sourcesMock.mockResolvedValue({ list_id: "l2", scope: "channel:dev", sources: [] });
        importMock.mockResolvedValue({ added: 1, renamed: 0 });
        const onImported = vi.fn();
        render(() => <GlobalMemoryImportBanner onImported={onImported} />);
        expect(await screen.findByText("Rules")).toBeTruthy();
        fireEvent.click(screen.getByText("Bring them here"));
        await waitFor(() => expect(importMock).toHaveBeenCalledWith({}, { list_id: "l1", index: 0 }));
        expect(await screen.findByText(/Added 1 entry/)).toBeTruthy();
        expect(onImported).toHaveBeenCalled();
    });

    it("names scopes for the human", () => {
        expect(scopeLabel("shared")).toBe("your main Global Memory");
        expect(scopeLabel("channel:dev-x")).toBe('the "dev-x" channel');
    });
});
