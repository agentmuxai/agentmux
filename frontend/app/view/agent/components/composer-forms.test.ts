// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { COMPACT_ORDER, accountInitial, chooseCompactKeys, shortContextText } from "./composer-forms";

// The strip at about 420 px: the runtime trigger, the account, the context reading with Compact, and Shell.
const FULL = { runtime: 168, auth: 152, ctx: 256, hostShell: 72 };
const COMPACT = { auth: 32, ctx: 112, hostShell: 32 };

/** A stand-in for the strip's row layout: lines of `width` filled in order, `gap` between slots. */
const rowsAt =
    (width: number, gap = 8) =>
    (widths: Record<string, number>): number => {
        const shown = Object.values(widths).filter((w) => w > 0);
        const total = shown.reduce((sum, w) => sum + w, 0) + Math.max(0, shown.length - 1) * gap;
        return Math.max(1, Math.ceil(total / width));
    };

describe("chooseCompactKeys", () => {
    it("keeps every slot full when they fit one row", () => {
        expect(chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, rowsAt(900)).size).toBe(0);
    });

    it("collapses in order, only as far as it takes to reach the fewest rows", () => {
        // 648 + 3 gaps = 672. Auth alone saves 120.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, rowsAt(560))]).toEqual(["auth"]);
        // Auth and ctx: 672 - 120 - 144 = 408.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, rowsAt(420))]).toEqual(["auth", "ctx"]);
        // And Shell: 408 - 40 = 368.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, rowsAt(370))]).toEqual(["auth", "ctx", "hostShell"]);
    });

    it("when one row is out of reach, stops at the fewest rows the compact forms give and keeps the other full forms", () => {
        // All compact: 368, two rows of 300. Full: three. The account alone gets it to two.
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, rowsAt(300))]).toEqual(["auth"]);
    });

    it("keeps every compact form when the rows only come down with all of them", () => {
        expect([...chooseCompactKeys(COMPACT_ORDER, FULL, COMPACT, rowsAt(200))]).toEqual(["auth", "ctx", "hostShell"]);
    });

    it("collapses as few slots as it takes, not the first ones in the order", () => {
        // 654 + 4 gaps = 686 in 600 px: the view chip alone (126) is enough; the account (72) and context (48) are not.
        const full = { runtime: 168, auth: 104, ctx: 160, hostShell: 72, view: 150 };
        const compact = { auth: 32, ctx: 112, hostShell: 32, view: 24 };
        expect([...chooseCompactKeys(COMPACT_ORDER, full, compact, rowsAt(600))]).toEqual(["view"]);
    });

    it("of equal numbers of slots takes the earlier in the order", () => {
        const full = { runtime: 168, auth: 104, ctx: 160, hostShell: 72, view: 150 };
        const compact = { auth: 32, ctx: 112, hostShell: 32, view: 24 };
        // 686 in 640: the account (72) or the context reading (48) or the chip (126) alone each do it.
        expect([...chooseCompactKeys(COMPACT_ORDER, full, compact, rowsAt(640))]).toEqual(["auth"]);
    });

    it("never touches a slot with no compact form", () => {
        const chosen = chooseCompactKeys(["auth", "ctx", "hostShell", "runtime"], FULL, COMPACT, rowsAt(100));
        expect(chosen.has("runtime")).toBe(false);
    });

    it("leaves out a slot the strip already hides (0 wide), and a form that is no smaller", () => {
        const hidden = chooseCompactKeys(COMPACT_ORDER, { ...FULL, auth: 0 }, COMPACT, rowsAt(300));
        expect(hidden.has("auth")).toBe(false);
        const same = chooseCompactKeys(COMPACT_ORDER, FULL, { ...COMPACT, ctx: 256 }, rowsAt(300));
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
