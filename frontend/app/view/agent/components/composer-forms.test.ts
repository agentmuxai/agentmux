// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { COMPACT_ORDER, accountInitial, chooseCompactKeys, shortContextText } from "./composer-forms";

// The strip at about 420 px: the runtime trigger, the account, the context reading with Compact, and Shell.
const FULL = { runtime: 168, auth: 152, ctx: 256, hostShell: 72 };
const COMPACT = { auth: 32, ctx: 112, hostShell: 32 };

describe("chooseCompactKeys", () => {
    it("keeps every slot full when they fit one row", () => {
        expect(chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, 900, 8).size).toBe(0);
    });

    it("collapses in order, only as far as it takes to fit", () => {
        // 648 + 3 gaps = 672. Auth alone saves 120.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, 560, 8)]).toEqual(["auth"]);
        // Auth and ctx: 672 - 120 - 144 = 408.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, 420, 8)]).toEqual(["auth", "ctx"]);
        // And Shell: 408 - 40 = 368.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, 370, 8)]).toEqual(["auth", "ctx", "hostShell"]);
    });

    it("keeps every compact form when even those don't fit, and leaves the rest to the row layout", () => {
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, 200, 8)]).toEqual(["auth", "ctx", "hostShell"]);
    });

    it("never touches a slot with no compact form", () => {
        const chosen = chooseCompactKeys(["auth", "ctx", "hostShell", "runtime"], FULL, COMPACT, 100, 8);
        expect(chosen.has("runtime")).toBe(false);
    });

    it("leaves out a slot the strip already hides (0 wide), and a form that is no smaller", () => {
        const hidden = chooseCompactKeys(COMPACT_ORDER, { ...FULL, auth: 0 }, COMPACT, 300, 8);
        expect(hidden.has("auth")).toBe(false);
        const same = chooseCompactKeys(COMPACT_ORDER, FULL, { ...COMPACT, ctx: 256 }, 300, 8);
        expect(same.has("ctx")).toBe(false);
    });

    it("puts the account first and Shell after the context reading", () => {
        expect(COMPACT_ORDER.indexOf("auth")).toBeLessThan(COMPACT_ORDER.indexOf("ctx"));
        expect(COMPACT_ORDER.indexOf("ctx")).toBeLessThan(COMPACT_ORDER.indexOf("hostShell"));
    });
});

describe("shortContextText", () => {
    it("is the reading without its spaces and its .0", () => {
        expect(shortContextText("392k / 1.0m")).toBe("392k/1m");
        expect(shortContextText("104k / 200k")).toBe("104k/200k");
        expect(shortContextText("93k / 1.5m")).toBe("93k/1.5m");
    });

    it("leaves a reading with no window alone", () => {
        expect(shortContextText("104k ctx")).toBe("104k ctx");
        expect(shortContextText("session")).toBe("session");
    });
});

describe("accountInitial", () => {
    it("is the first letter or digit, upper case", () => {
        expect(accountInitial("poppercor…@gmail.com")).toBe("P");
        expect(accountInitial("…dev")).toBe("D");
        expect(accountInitial("42")).toBe("4");
    });

    it("has a mark when there is no letter", () => {
        expect(accountInitial("…")).toBe("?");
        expect(accountInitial("")).toBe("?");
    });
});
