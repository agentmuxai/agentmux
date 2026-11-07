// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DEFAULT_KEYBINDINGS, rowKeys } from "./defaults";
import { reservedFor } from "./reserved";

describe("reservedFor", () => {
    it("blocks the keys that cannot arrive", () => {
        expect(reservedFor("ctrl+alt+Delete", "other")?.tier).toBe("blocked");
        expect(reservedFor("alt+F4", "other")?.tier).toBe("blocked");
        expect(reservedFor("meta+e", "other")?.tier).toBe("blocked");
        expect(reservedFor("meta+q", "mac")?.tier).toBe("blocked");
        expect(reservedFor("meta+Tab", "mac")?.tier).toBe("blocked");
        expect(reservedFor("meta+code:Space", "mac")?.tier).toBe("blocked");
    });

    it("blocks a bare Escape everywhere, and leaves it free with a modifier", () => {
        for (const p of ["mac", "linux", "other"] as const) expect(reservedFor("Escape", p)?.tier).toBe("blocked");
        expect(reservedFor("ctrl+Escape", "mac")).toBeNull();
    });

    it("warns on the keys measured on each desktop", () => {
        expect(reservedFor("ctrl+ArrowUp", "mac")?.tier).toBe("warned");
        expect(reservedFor("F5", "mac")?.tier).toBe("warned");
        expect(reservedFor("ctrl+alt+ArrowLeft", "linux")?.tier).toBe("warned");
        expect(reservedFor("ctrl+shift+alt+ArrowUp", "linux")?.tier).toBe("warned");
        expect(reservedFor("meta+3", "linux")?.tier).toBe("warned");
        expect(reservedFor("alt+Tab", "linux")?.tier).toBe("warned");
        expect(reservedFor("ctrl+shift+0", "other")?.tier).toBe("warned");
        expect(reservedFor("ctrl+alt+q", "other")?.tier).toBe("warned");
    });

    it("only reserves a key on the platforms it was measured on", () => {
        expect(reservedFor("ctrl+ArrowUp", "other")).toBeNull();
        expect(reservedFor("meta+e", "mac")).toBeNull();
        expect(reservedFor("ctrl+alt+ArrowLeft", "other")).toBeNull();
        expect(reservedFor("ctrl+shift+0", "linux")).toBeNull();
    });

    it("leaves an ordinary key free", () => {
        expect(reservedFor("ctrl+shift+t", "other")).toBeNull();
        expect(reservedFor("meta+t", "mac")).toBeNull();
        expect(reservedFor("ctrl+alt+shift+PageUp", "other")).toBeNull();
    });

    it("takes the worst step of a chord, and ignores a malformed key", () => {
        expect(reservedFor("ctrl+shift+s Escape", "other")?.tier).toBe("blocked");
        expect(reservedFor("ctrl+shift+s ArrowDown", "other")).toBeNull();
        expect(reservedFor("ctrl+nonsense+x", "other")).toBeNull();
    });

    it("gives every hit a reason to show", () => {
        expect(reservedFor("meta+e", "other")?.reason).toMatch(/Windows/);
    });

    it("blocks no key of the default table except the fixed Escape", () => {
        const blocked: string[] = [];
        for (const platform of ["mac", "linux", "other"] as const) {
            for (const row of DEFAULT_KEYBINDINGS) {
                for (const key of rowKeys(row, platform)) if (reservedFor(key, platform)?.tier === "blocked") blocked.push(`${platform} ${row.command} ${key}`);
            }
        }
        expect(blocked).toEqual(["mac app:escape Escape", "linux app:escape Escape", "other app:escape Escape"]);
    });
});
