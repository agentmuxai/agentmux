// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the macOS menu bar is told to show next to its items, and when.

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setMenuKeys: vi.fn(), platform: "mac" }));

vi.mock("@/app/keybindings", () => ({ keyPlatform: () => h.platform }));
vi.mock("@/app/store/global", () => ({ getApi: () => ({ setMenuKeys: h.setMenuKeys }) }));

import { setUserKeybindings } from "@/app/keybindings/registry";
import { pushMenuKeys, resetMenuKeysPush } from "./menu-keys-push";

const sent = (call: number): Record<string, string> =>
    Object.fromEntries((h.setMenuKeys.mock.calls[call][0] as Array<{ command: string; key: string }>).map((k) => [k.command, k.key]));

beforeEach(() => {
    h.setMenuKeys.mockClear();
    h.platform = "mac";
    resetMenuKeysPush();
    setUserKeybindings([], () => false);
});

describe("pushMenuKeys", () => {
    it("sends the keys of the table on macOS", () => {
        pushMenuKeys();
        expect(h.setMenuKeys).toHaveBeenCalledTimes(1);
        expect(sent(0)["tab:new"]).toBe("t");
    });

    it("sends nothing off macOS, which has no native menu bar", () => {
        h.platform = "other";
        pushMenuKeys();
        expect(h.setMenuKeys).not.toHaveBeenCalled();
    });

    it("sends the same keys once, and again when the person's keybindings change them", () => {
        pushMenuKeys();
        pushMenuKeys();
        expect(h.setMenuKeys).toHaveBeenCalledTimes(1);
        setUserKeybindings([{ command: "tab:new", key: "meta+y", platform: "mac" }], () => false);
        pushMenuKeys();
        expect(h.setMenuKeys).toHaveBeenCalledTimes(2);
        expect(sent(1)["tab:new"]).toBe("y");
    });
});
