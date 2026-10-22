// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    calls: 0,
    reply: { models: [] as { id: string; display_name: string }[] } as unknown,
    fail: false,
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ProvidersModelsCommand: async () => {
            h.calls++;
            if (h.fail) throw new Error("rpc down");
            return h.reply;
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { _resetCodexModelsRefreshForTests, MIN_INTERVAL_MS, refreshCodexModels } from "./codex-models-refresh";
import { getProvider, isAccountListed } from "./model-overlay";

const codexIds = () => getProvider("codex")!.models!.map((m) => m.value);

describe("refreshCodexModels", () => {
    beforeEach(() => {
        _resetCodexModelsRefreshForTests();
        h.calls = 0;
        h.fail = false;
    });

    it("keeps the curated list until an account has a list, then takes the account's", async () => {
        const curated = codexIds();
        h.reply = { models: [] };
        refreshCodexModels(0);
        await vi.waitFor(() => expect(h.calls).toBe(1));
        expect(codexIds()).toEqual(curated);
        expect(isAccountListed("codex")).toBe(false);

        // A fresh sign-in's first turn wrote the cache; the next trigger picks it up.
        h.reply = { models: [{ id: "gpt-7-nova", display_name: "GPT-7-Nova" }] };
        refreshCodexModels(MIN_INTERVAL_MS);
        await vi.waitFor(() => expect(codexIds()).toEqual(["gpt-7-nova"]));
        expect(isAccountListed("codex")).toBe(true);
    });

    it("asks at most once per interval", async () => {
        refreshCodexModels(1_000);
        refreshCodexModels(1_000 + MIN_INTERVAL_MS - 1);
        await vi.waitFor(() => expect(h.calls).toBe(1));
        refreshCodexModels(1_000 + MIN_INTERVAL_MS);
        await vi.waitFor(() => expect(h.calls).toBe(2));
    });

    it("retries at the next trigger after a failure", async () => {
        h.fail = true;
        refreshCodexModels(5_000);
        await vi.waitFor(() => expect(h.calls).toBe(1));
        await new Promise((r) => setTimeout(r, 0));
        h.fail = false;
        refreshCodexModels(5_001);
        await vi.waitFor(() => expect(h.calls).toBe(2));
    });
});
