// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const setBlockMeta = vi.fn<(blockId: string, meta: Record<string, unknown>) => Promise<void>>();
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: (...a: [string, Record<string, unknown>]) => setBlockMeta(...a) }));

import {
    CATCH_UP_META_KEY,
    CATCH_UP_REQUEST_MAX_AGE_MS,
    clearCatchUpRequest,
    launchResumesConversation,
    parseCatchUpRequest,
    requestCatchUp,
} from "./catch-up-request";

const NOW = 1_790_000_000_000;

describe("catch-up requests", () => {
    beforeEach(() => {
        setBlockMeta.mockReset();
        setBlockMeta.mockResolvedValue(undefined);
    });

    it("stores the reason and the time under the block's meta", async () => {
        await requestCatchUp("b1", "resumed", NOW);
        expect(setBlockMeta).toHaveBeenCalledWith("b1", { [CATCH_UP_META_KEY]: `resumed:${NOW}` });
    });

    it("clears the key with null", async () => {
        await clearCatchUpRequest("b1");
        expect(setBlockMeta).toHaveBeenCalledWith("b1", { [CATCH_UP_META_KEY]: null });
    });

    it("reads back a request made just now", () => {
        expect(parseCatchUpRequest(`resumed:${NOW}`, NOW + 1_000)).toBe("resumed");
    });

    it("asks for a catch-up on a launch that resumes a session, not on a fork or a fresh one", () => {
        expect(launchResumesConversation("sid-1", undefined)).toBe(true);
        expect(launchResumesConversation("sid-1", false)).toBe(true);
        expect(launchResumesConversation("sid-1", true)).toBe(false);
        expect(launchResumesConversation("", undefined)).toBe(false);
        expect(launchResumesConversation("  ", undefined)).toBe(false);
    });

    it("ignores a request that is too old, or one this build does not know, or nothing", () => {
        expect(parseCatchUpRequest(`resumed:${NOW}`, NOW + CATCH_UP_REQUEST_MAX_AGE_MS + 1)).toBeNull();
        expect(parseCatchUpRequest(`reborn:${NOW}`, NOW)).toBeNull();
        expect(parseCatchUpRequest("resumed", NOW)).toBeNull();
        expect(parseCatchUpRequest(undefined, NOW)).toBeNull();
        expect(parseCatchUpRequest(42, NOW)).toBeNull();
    });
});
