// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    ACCOUNT_BUDGETS,
    COMPACT_ORDER,
    accountInitial,
    chooseCompactKeys,
    chooseMiddleBudget,
    middleTruncate,
    shortContextText,
} from "./composer-forms";

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

describe("chooseCompactKeys with a row that overflows", () => {
    // The readout beside the context reading is one unit that is never split, so the strip stays one row at every
    // choice; what changes is how far that row overruns the strip (the cost the strip passes as `rowCount`).
    const full = { ctx: 110, stats: 64 };
    const compact = { ctx: 76, stats: 56 };
    const costAt = (width: number) => (widths: Record<string, number>) => {
        const total = (widths.ctx ?? 0) + (widths.stats ?? 0) + 8;
        return 1 + Math.min(1, Math.max(0, total - width) / width) / 2;
    };

    it("collapses what brings the row nearer the strip, though the row count never changes", () => {
        expect([...chooseCompactKeys(["ctx", "stats"], full, compact, costAt(117))].sort()).toEqual(["ctx", "stats"]);
    });

    it("collapses only the readout when that alone brings the row inside", () => {
        expect([...chooseCompactKeys(["ctx", "stats"], full, { ctx: 110, stats: 56 }, costAt(175))]).toEqual(["stats"]);
    });

    it("keeps both full when the row fits", () => {
        expect(chooseCompactKeys(["ctx", "stats"], full, compact, costAt(400)).size).toBe(0);
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


describe("middleTruncate", () => {
    it("leaves a label that fits as it is", () => {
        expect(middleTruncate("ada@mail.dev", 15)).toBe("ada@mail.dev");
    });

    it("keeps the start of the local part and the whole domain when the domain fits", () => {
        expect(middleTruncate("taylor.morgan@mail.dev", 15)).toBe("taylo…@mail.dev");
        expect(middleTruncate("taylor.morgan@mail.dev", 12)).toBe("ta…@mail.dev");
    });

    it("keeps the start of the address when the domain doesn't fit", () => {
        expect(middleTruncate("taylor.morgan@mail.dev", 9)).toBe("taylor.m…");
        expect(middleTruncate("taylor.morgan@mail.dev", 7)).toBe("taylor…");
    });

    it("cuts a long address once, from the whole address: one ellipsis, as many characters as the budget allows", () => {
        const cut = middleTruncate("jonathan.ross@anthropic.com", 9);
        expect(cut).toBe("jonathan…");
        expect(cut.match(/…/g)).toHaveLength(1);
    });

    it("cuts a label with no address to its start", () => {
        expect(middleTruncate("Not logged in", 9)).toBe("Not logg…");
    });

    it("never returns more than the budget", () => {
        for (const budget of ACCOUNT_BUDGETS) {
            expect(middleTruncate("someone.long.name@example.org", budget).length).toBeLessThanOrEqual(budget);
        }
    });
});

describe("chooseMiddleBudget", () => {
    // The account's width at a budget: 8 px a character and a 30 px dot.
    const widthAt = (budget: number) => 30 + budget * 8;

    it("takes the widest budget that keeps the rows", () => {
        expect(chooseMiddleBudget(ACCOUNT_BUDGETS, widthAt, (w) => w <= 140)).toBe(12);
        expect(chooseMiddleBudget(ACCOUNT_BUDGETS, widthAt, (w) => w <= 200)).toBe(15);
    });

    it("takes none when even the narrowest doesn't fit, so the account shows its letter", () => {
        expect(chooseMiddleBudget(ACCOUNT_BUDGETS, widthAt, (w) => w <= 70)).toBeUndefined();
    });

    it("skips a budget that was not measured", () => {
        expect(chooseMiddleBudget(ACCOUNT_BUDGETS, (b) => (b === 9 ? 100 : undefined), () => true)).toBe(9);
    });
});
