// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { getCliCatalogEntry } from "./cli-catalog";
import { PROVIDER_ACCESS, getProviderAccess, type AccessClaim, type ProviderAccess } from "./provider-access";
import {
    AGING_DAYS,
    CARD_FILTERS,
    STALE_DAYS,
    accessTagsFor,
    checkedNote,
    daysSince,
    filterCounts,
    freeGuideFor,
    freeTagFor,
    localTagFor,
    matchesFilters,
    plainNoteFor,
    type CardFilter,
} from "./provider-access-model";

const CHECKED = new Date("2026-10-29T12:00:00Z");

function daysAfter(iso: string, days: number): Date {
    return new Date(new Date(`${iso}T12:00:00Z`).getTime() + days * 24 * 60 * 60 * 1000);
}

function claimsOf(a: ProviderAccess): AccessClaim[] {
    return [
        ...(a.free ? [a.free.limits, ...a.free.catches] : []),
        ...(a.local ? [a.local.how, ...(a.local.note ? [a.local.note] : [])] : []),
        ...(a.successor ? [a.successor.text] : []),
        ...(a.plain ? [a.plain] : []),
    ];
}

describe("the access data", () => {
    it("names only providers the picker has a card for, once each", () => {
        const seen = new Set<string>();
        for (const a of PROVIDER_ACCESS) {
            expect(getCliCatalogEntry(a.provider), a.provider).not.toBeNull();
            expect(seen.has(a.provider), `${a.provider} twice`).toBe(false);
            seen.add(a.provider);
        }
    });

    it("gives every claim a source on https and a day that has already passed", () => {
        for (const a of PROVIDER_ACCESS) {
            for (const c of claimsOf(a)) {
                expect(c.text.trim().length, a.provider).toBeGreaterThan(0);
                expect(c.source.startsWith("https://"), `${a.provider}: ${c.source}`).toBe(true);
                expect(c.verifiedOn, a.provider).toMatch(/^\d{4}-\d{2}-\d{2}$/);
                expect(daysSince(c.verifiedOn, CHECKED), `${a.provider}: ${c.verifiedOn}`).toBeGreaterThanOrEqual(0);
            }
        }
    });

    it("keeps the sign-up link and the steps of every free route short and on https", () => {
        for (const a of PROVIDER_ACCESS) {
            if (!a.free) continue;
            expect(a.free.signupUrl.startsWith("https://"), a.provider).toBe(true);
            expect(a.free.steps.length, a.provider).toBeGreaterThanOrEqual(2);
            expect(a.free.steps.length, a.provider).toBeLessThanOrEqual(3);
            expect(a.free.catches.length, a.provider).toBeLessThanOrEqual(2);
        }
    });

    it("points a successor at a provider that has a card", () => {
        for (const a of PROVIDER_ACCESS) {
            if (a.successor) expect(getCliCatalogEntry(a.successor.provider)).not.toBeNull();
        }
    });
});

// The operator approved these sets on 2026-10-29 (SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL §4 and §11). A
// change to one is a decision, so it changes this test on purpose.
describe("who gets which tag", () => {
    const tagged = (label: "Free" | "Local") =>
        PROVIDER_ACCESS.map((a) => a.provider).filter((p) => accessTagsFor(p, CHECKED).some((t) => t.label === label));

    it("gives Free to seven providers", () => {
        expect(tagged("Free").sort()).toEqual(["antigravity", "cline", "copilot", "gemini", "grok", "kilo", "opencode"]);
    });

    it("gives Codex no Free tag until it is tested on a real Free account, and Claude Code none either", () => {
        expect(freeTagFor(getProviderAccess("codex"), CHECKED)).toBeNull();
        expect(freeTagFor(getProviderAccess("claude"), CHECKED)).toBeNull();
    });

    it("gives Local to fourteen providers, Codex and Claude Code among them, and not to Kimi", () => {
        const local = tagged("Local");
        expect(local).toHaveLength(14);
        expect(local).toContain("codex");
        expect(local).toContain("claude");
        for (const none of ["gemini", "antigravity", "muse", "kimi"]) expect(local).not.toContain(none);
    });

    it("lists Free before Local on a card that has both", () => {
        expect(accessTagsFor("opencode", CHECKED).map((t) => t.label)).toEqual(["Free", "Local"]);
    });

    it("points Gemini CLI's card at Antigravity", () => {
        expect(getProviderAccess("gemini")?.successor?.provider).toBe("antigravity");
    });
});

describe("how fresh a claim is", () => {
    const claim = (verifiedOn: string): AccessClaim => ({ text: "x", source: "https://example.com", verifiedOn });

    it("is fresh for a month, and says the day it was checked", () => {
        const note = checkedNote([claim("2026-10-29")], daysAfter("2026-10-29", AGING_DAYS));
        expect(note).toEqual({ date: "2026-10-29", state: "fresh", text: "checked 2026-10-29" });
    });

    it("ages after 30 days and says how long ago", () => {
        const note = checkedNote([claim("2026-10-29")], daysAfter("2026-10-29", AGING_DAYS + 11));
        expect(note.state).toBe("aging");
        expect(note.text).toBe("checked 41 days ago, may have changed");
    });

    it("goes stale after 90 days and says since when", () => {
        const fine = checkedNote([claim("2026-10-29")], daysAfter("2026-10-29", STALE_DAYS));
        expect(fine.state).toBe("aging");
        const note = checkedNote([claim("2026-10-29")], daysAfter("2026-10-29", STALE_DAYS + 1));
        expect(note).toEqual({ date: "2026-10-29", state: "stale", text: "Not re-checked since 2026-10-29" });
    });

    it("rests on the oldest claim", () => {
        const note = checkedNote([claim("2026-10-29"), claim("2026-06-01")], CHECKED);
        expect(note.date).toBe("2026-06-01");
        expect(note.state).toBe("stale");
    });
});

describe("the Free and Local tags", () => {
    it("shows Free in the accent tone with the lead, the limit and the catches, at most four lines", () => {
        const tag = freeTagFor(getProviderAccess("gemini"), CHECKED)!;
        expect(tag.label).toBe("Free");
        expect(tag.tone).toBe("accent");
        expect(tag.lines[0]).toBe("Free with an AI Studio key");
        expect(tag.lines.length).toBeLessThanOrEqual(4);
        expect(tag.lines.some((l) => l.includes("Antigravity"))).toBe(true);
        expect(tag.checked.text).toBe("checked 2026-10-29");
    });

    it("turns Free neutral, not hidden, once its claims are stale, and says so", () => {
        const stale = daysAfter("2026-10-29", STALE_DAYS + 1);
        const tag = freeTagFor(getProviderAccess("antigravity"), stale)!;
        expect(tag.tone).toBe("neutral");
        expect(tag.checked.text).toBe("Not re-checked since 2026-10-29");
    });

    it("drops Free after a promotion's end date", () => {
        const base = getProviderAccess("grok")!;
        const withEnd: ProviderAccess = { ...base, free: { ...base.free!, expiresOn: "2026-11-15" } };
        expect(freeTagFor(withEnd, new Date("2026-11-15T12:00:00Z"))).not.toBeNull();
        expect(freeTagFor(withEnd, new Date("2026-11-16T12:00:00Z"))).toBeNull();
    });

    it("says Local is documented, not tested, until AgentMux has run it", () => {
        const tag = localTagFor(getProviderAccess("opencode"), CHECKED)!;
        expect(tag.tone).toBe("neutral");
        expect(tag.lines.at(-1)).toBe("Documented by the CLI, not tested by AgentMux.");
        const base = getProviderAccess("opencode")!;
        const tested: ProviderAccess = { ...base, local: { ...base.local!, testedOn: { model: "qwen3-coder:30b", date: "2026-11-02", ok: true } } };
        expect(localTagFor(tested, CHECKED)!.lines.at(-1)).toBe("Works in AgentMux's own test with qwen3-coder:30b (2026-11-02).");
    });

    it("keeps Codex's plain answer and its Responses API line for the card's tooltip and panel", () => {
        const tag = localTagFor(getProviderAccess("codex"), CHECKED)!;
        expect(tag.lines.join(" ")).toContain("Responses API");
        expect(tag.lines.join(" ")).toContain("No verified free cloud route");
        expect(plainNoteFor("codex", CHECKED)?.text).toContain("isn't confirmed");
    });

    it("gives nothing for a provider the file doesn't know", () => {
        expect(accessTagsFor("not-a-provider", CHECKED)).toEqual([]);
        expect(plainNoteFor("not-a-provider", CHECKED)).toBeNull();
    });
});

describe("the card filters", () => {
    const cards = [
        { provider: "opencode", installed: true },
        { provider: "kilo", installed: false },
        { provider: "gemini", installed: true },
        { provider: "codex", installed: undefined },
        { provider: "claude", installed: true },
        { provider: "muse", installed: false },
    ];
    const none = new Set<CardFilter>();
    const names = (sel: CardFilter[]) =>
        cards.filter((c) => matchesFilters(c, new Set(sel), CHECKED)).map((c) => c.provider);

    it("shows every card with nothing selected", () => {
        expect(names([])).toHaveLength(cards.length);
    });

    it("narrows: more than one selected keeps only a card that passes them all", () => {
        expect(names(["free"])).toEqual(["opencode", "kilo", "gemini"]);
        expect(names(["free", "local"])).toEqual(["opencode", "kilo"]);
        expect(names(["free", "local", "installed"])).toEqual(["opencode"]);
    });

    it("counts Installed only for a card the probe found, never one it hasn't answered for", () => {
        expect(names(["installed"])).toEqual(["opencode", "gemini", "claude"]);
    });

    it("gives each chip the count it would leave on top of what is selected", () => {
        expect(filterCounts(cards, none, CHECKED)).toEqual({ free: 3, local: 4, installed: 3 });
        expect(filterCounts(cards, new Set<CardFilter>(["free"]), CHECKED)).toEqual({ free: 3, local: 2, installed: 2 });
    });

    it("has the three chips the spec names", () => {
        expect([...CARD_FILTERS]).toEqual(["free", "local", "installed"]);
    });
});

describe("the free sign-up guide", () => {
    it("is the link, the steps, the limit and the catches for a sign-in provider", () => {
        const g = freeGuideFor("antigravity", CHECKED)!;
        expect(g.signupUrl).toBe("https://antigravity.google/docs/getting-started?tab=cli");
        expect(g.steps.length).toBeGreaterThanOrEqual(2);
        expect(g.lead).toBe("Free with a Google sign-in");
        expect(g.checked.text).toBe("checked 2026-10-29");
        expect(g.stale).toBe(false);
    });

    it("says when its claims are stale, and still shows", () => {
        const g = freeGuideFor("kilo", daysAfter("2026-10-29", STALE_DAYS + 1))!;
        expect(g.stale).toBe(true);
        expect(g.checked.text).toBe("Not re-checked since 2026-10-29");
    });

    it("is not offered where the route is a pasted key, for no free route, or for a provider the file doesn't know", () => {
        expect(freeGuideFor("gemini", CHECKED)).toBeNull();
        expect(freeGuideFor("codex", CHECKED)).toBeNull();
        expect(freeGuideFor("claude", CHECKED)).toBeNull();
        expect(freeGuideFor("not-a-provider", CHECKED)).toBeNull();
    });

    it("is offered for exactly the six sign-in providers", () => {
        const offered = PROVIDER_ACCESS.map((a) => a.provider).filter((p) => freeGuideFor(p, CHECKED) !== null);
        expect(offered.sort()).toEqual(["antigravity", "cline", "copilot", "grok", "kilo", "opencode"]);
    });
});
