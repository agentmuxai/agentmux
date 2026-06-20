// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for openOAuthBrowserPane (P2.1 of SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20).
 *
 * Pane-first, system-browser fallback, never-throws contract:
 *   - success → createBlock with a browser BlockDef, returns "pane"
 *   - createBlock throws → openExternal fires, returns "external"
 *   - both fail → returns "failed" (no throw)
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    createBlock: vi.fn(),
    openExternal: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    createBlock: hub.createBlock,
    getApi: () => ({ openExternal: hub.openExternal }),
}));

import { openOAuthBrowserPane } from "./open-oauth-pane";

const URL = "https://claude.ai/oauth/authorize?client_id=abc";

beforeEach(() => {
    hub.createBlock.mockReset();
    hub.openExternal.mockReset();
});
afterEach(() => vi.clearAllMocks());

describe("openOAuthBrowserPane", () => {
    it("opens an in-app browser pane and returns 'pane' on success", async () => {
        hub.createBlock.mockResolvedValue("block-1");
        const result = await openOAuthBrowserPane(URL);

        expect(result).toBe("pane");
        expect(hub.createBlock).toHaveBeenCalledTimes(1);
        expect(hub.createBlock).toHaveBeenCalledWith({ meta: { view: "browser", url: URL } });
        expect(hub.openExternal).not.toHaveBeenCalled();
    });

    it("falls back to the system browser and returns 'external' when the pane fails", async () => {
        hub.createBlock.mockRejectedValue(new Error("no layout model"));
        const result = await openOAuthBrowserPane(URL);

        expect(result).toBe("external");
        expect(hub.openExternal).toHaveBeenCalledExactlyOnceWith(URL);
    });

    it("returns 'failed' (no throw) when both pane and system browser fail", async () => {
        hub.createBlock.mockRejectedValue(new Error("no layout model"));
        hub.openExternal.mockImplementation(() => { throw new Error("no shell"); });

        await expect(openOAuthBrowserPane(URL)).resolves.toBe("failed");
    });
});
