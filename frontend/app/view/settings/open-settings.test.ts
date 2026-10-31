// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Opening Settings at a section is the window's own pane open, never the server's pick of a tab.

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ open: vi.fn(() => Promise.resolve()), rpc: vi.fn() }));

vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: h.open }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { PaneOpenCommand: h.rpc } }));

import { openSettingsAt } from "./open-settings";

beforeEach(() => {
    h.open.mockClear();
    h.rpc.mockClear();
});

describe("openSettingsAt", () => {
    it("opens or focuses this window's Settings pane at the section", async () => {
        await openSettingsAt("browser");
        expect(h.open).toHaveBeenCalledWith(
            "settings",
            { meta: { view: "settings", "settings:section": "browser" } },
            { "settings:section": "browser" },
        );
    });

    it("never asks the server to pick the tab", async () => {
        await openSettingsAt("agents");
        expect(h.rpc).not.toHaveBeenCalled();
    });

    it("is the same call for every click: the open-or-focus is what keeps it to one pane", async () => {
        await openSettingsAt("browser");
        await openSettingsAt("browser");
        await openSettingsAt("browser");
        expect(h.open).toHaveBeenCalledTimes(3);
        expect(new Set(h.open.mock.calls.map((c) => JSON.stringify(c))).size).toBe(1);
    });
});
