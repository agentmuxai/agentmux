// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A notification click reveals its pane through the one shared reveal path
 * (SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md Phase 2), so it now also switches a
 * multi-tab pane to the block instead of only focusing the pane.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const reveal = vi.fn();
const focusWindow = vi.fn();

vi.mock("@/app/util/reveal-block", () => ({ revealBlockLocally: (...a: unknown[]) => reveal(...a) }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ addEventListener: () => () => {} }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { blockFocusAtom: () => null } }));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "w1" }));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({ getWindowLabel: async () => "main", focusWindow: (l: string) => focusWindow(l) }),
    getSettingsKeyAtom: () => () => undefined,
    workspace: () => null,
}));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));

import { activateBlockLocally } from "./os-notify-bridge";

beforeEach(() => {
    reveal.mockReset();
    focusWindow.mockReset();
});

describe("activateBlockLocally", () => {
    it("reveals the block through revealBlockLocally, passing srv's tab, then raises this window", async () => {
        reveal.mockResolvedValue(true);
        expect(await activateBlockLocally("b1", "tab-7")).toBe(true);
        expect(reveal).toHaveBeenCalledWith("b1", { tabId: "tab-7" });
        expect(focusWindow).toHaveBeenCalledWith("main");
    });

    it("a block that isn't in this window: false, and the window isn't raised", async () => {
        reveal.mockResolvedValue(false);
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        expect(await activateBlockLocally("elsewhere")).toBe(false);
        expect(focusWindow).not.toHaveBeenCalled();
        warn.mockRestore();
    });
});
