// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the host is told to forward out of a focused browser pane when the person's keybindings change
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §7).

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setHostKeys: vi.fn() }));

vi.mock("@/app/keybindings", () => ({ keyPlatform: () => "other" }));
vi.mock("@/app/store/global", () => ({ getApi: () => ({ setHostKeys: h.setHostKeys }) }));

import { setUserKeybindings } from "@/app/keybindings/registry";
import { pushHostKeys, resetHostKeysPush } from "./host-keys-push";

const sentKeys = (call: number): string[] => (h.setHostKeys.mock.calls[call][0] as Array<{ source: string }>).map((k) => k.source);

beforeEach(() => {
    h.setHostKeys.mockClear();
    resetHostKeysPush();
    setUserKeybindings([], () => false);
});

describe("pushHostKeys", () => {
    it("sends nothing while the table is the default one: the host starts with that list", () => {
        pushHostKeys();
        expect(h.setHostKeys).not.toHaveBeenCalled();
    });

    it("sends the list the person's table gives: a key they bind is forwarded, one they unbind is not", () => {
        setUserKeybindings([{ command: "-tab:new", key: "ctrl+shift+t", platform: "other" }, { command: "tab:new", key: "ctrl+shift+y", platform: "other" }], () => false);
        pushHostKeys();
        expect(h.setHostKeys).toHaveBeenCalledTimes(1);
        const keys = sentKeys(0);
        expect(keys).toContain("ctrl+shift+y");
        expect(keys).not.toContain("ctrl+shift+t");
    });

    it("sends the same list once", () => {
        setUserKeybindings([{ command: "tab:new", key: "ctrl+shift+y", platform: "other" }], () => false);
        pushHostKeys();
        pushHostKeys();
        expect(h.setHostKeys).toHaveBeenCalledTimes(1);
    });

    it("sends the default list again when the person goes back to the defaults", () => {
        setUserKeybindings([{ command: "tab:new", key: "ctrl+shift+y", platform: "other" }], () => false);
        pushHostKeys();
        setUserKeybindings([], () => false);
        pushHostKeys();
        expect(h.setHostKeys).toHaveBeenCalledTimes(2);
        expect(sentKeys(1)).toContain("ctrl+shift+t");
        expect(sentKeys(1)).not.toContain("ctrl+shift+y");
    });
});
