// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const sectionsCommand = vi.fn();

vi.mock("@/app/store/rpc-api/bundle", () => ({
    BundleApi: { GlobalMemorySectionsCommand: (...args: unknown[]) => sectionsCommand(...args) },
}));
vi.mock("@/app/store/rpc-api/native-memory", () => ({
    NativeMemoryApi: {
        NativeMemoryListCommand: async () => ({ files: [] }),
        NativeMemoryReadFileCommand: async () => ({ content: "" }),
    },
}));

import { fetchGlobalMemoryEntries, fetchMemoryReinjectionEntries } from "./memory-reinjection-fetch";

const client = {} as never;

describe("Global Memory for a reinjection", () => {
    beforeEach(() => {
        sectionsCommand.mockReset();
        sectionsCommand.mockResolvedValue([
            { id: "operator-config-workspace", name: "Your workspace", is_system: true, text: "# body", size_bytes: 6 },
        ]);
    });

    it("asks srv for the sections of this pane, so it gets what the pane's startup file got", async () => {
        await fetchMemoryReinjectionEntries(client, "clamk", "block-1");
        expect(sectionsCommand).toHaveBeenCalledWith(client, { block_id: "block-1" });
    });

    it("labels each section by tier", async () => {
        const entries = await fetchGlobalMemoryEntries(client, "block-1");
        expect(entries).toEqual([
            { label: "[AgentMux System] Your workspace", source: "global", body: "# body", sizeBytes: 6 },
        ]);
    });

    it("without a block, asks for every section", async () => {
        await fetchGlobalMemoryEntries(client);
        expect(sectionsCommand).toHaveBeenCalledWith(client, { block_id: undefined });
    });
});
