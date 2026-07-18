// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for openOAuthBrowserPane (P2.1 of SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20,
 * flipped to system-browser-first — retro-agent-login-browser-2026-07-18).
 *
 * System-browser-first, in-app-pane fallback, never-throws contract:
 *   - success → open_external fires, returns "external"
 *   - open_external throws → createBlock with a browser BlockDef, returns "pane"
 *   - both fail → returns "failed" (no throw)
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    createBlock: vi.fn(),
    invokeCommand: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    createBlock: hub.createBlock,
}));

vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: hub.invokeCommand,
}));

import { openOAuthBrowserPane } from "./open-oauth-pane";

const URL = "https://claude.ai/oauth/authorize?client_id=abc";

beforeEach(() => {
    hub.createBlock.mockReset();
    hub.invokeCommand.mockReset();
});
afterEach(() => vi.clearAllMocks());

describe("openOAuthBrowserPane", () => {
    it("opens the system browser and returns 'external' on success", async () => {
        hub.invokeCommand.mockResolvedValue(undefined);
        const result = await openOAuthBrowserPane(URL);

        expect(result).toBe("external");
        expect(hub.invokeCommand).toHaveBeenCalledExactlyOnceWith("open_external", { url: URL });
        expect(hub.createBlock).not.toHaveBeenCalled();
    });

    it("falls back to an in-app browser pane and returns 'pane' when the system browser fails", async () => {
        hub.invokeCommand.mockRejectedValue(new Error("no shell"));
        hub.createBlock.mockResolvedValue("block-1");
        const result = await openOAuthBrowserPane(URL);

        expect(result).toBe("pane");
        expect(hub.createBlock).toHaveBeenCalledExactlyOnceWith({ meta: { view: "browser", url: URL } });
    });

    it("returns 'failed' (no throw) when both the system browser and pane fail", async () => {
        hub.invokeCommand.mockRejectedValue(new Error("no shell"));
        hub.createBlock.mockRejectedValue(new Error("no layout model"));

        await expect(openOAuthBrowserPane(URL)).resolves.toBe("failed");
    });
});
