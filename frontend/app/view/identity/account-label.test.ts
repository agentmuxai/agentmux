// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md §3: an Armory account is called
// by its login email when the provider recorded one, else by its name.

import { describe, expect, it } from "vitest";
import { accountLabel, boundAccountEmail, shortenEmail } from "./identity-model";

describe("accountLabel", () => {
    it("is the login email when one is recorded", () => {
        expect(accountLabel({ name: "claude-oauth", context: { email: "me@example.com" } })).toBe(
            "me@example.com"
        );
    });

    it("falls back to the account name", () => {
        expect(accountLabel({ name: "claude-oauth", context: {} })).toBe("claude-oauth");
        expect(accountLabel({ name: "claude-oauth", context: { email: "" } })).toBe("claude-oauth");
        expect(accountLabel({ name: "claude-oauth" } as never)).toBe("claude-oauth");
    });
});

// The agent composer's "Logged in" chip shows the account email instead, in
// at most 22 characters by default (full address in the tooltip). The part
// before the @ shortens first, in the middle, down to its first character +
// its last two; only then does the domain shorten the same way, keeping its TLD.
describe("shortenEmail", () => {
    describe("default budget (22)", () => {
        it("returns an email that fits unchanged", () => {
            expect(shortenEmail("asafebgi@gmail.com")).toBe("asafebgi@gmail.com");
            expect(shortenEmail("someone@anthropic.com")).toBe("someone@anthropic.com");
        });

        it("shortens only the part before @ while the whole domain still fits", () => {
            expect(shortenEmail("jonathan.ross@anthropic.com")).toBe("jonat…ss@anthropic.com");
            expect(shortenEmail("jonathan.ross@anthropic.com").length).toBeLessThanOrEqual(22);
        });

        it("keeps the last two characters before @ so similar names stay distinguishable", () => {
            expect(shortenEmail("asafe.bgi.dev.one@gmail.com")).toBe("asafe.bgi…ne@gmail.com");
            expect(shortenEmail("asafe.bgi.dev.two@gmail.com")).toBe("asafe.bgi…wo@gmail.com");
        });

        it("then shortens a long domain the same way, keeping its TLD", () => {
            const out = shortenEmail("someone@engineering.example-corporation.com");
            expect(out.startsWith("s…ne@")).toBe(true);
            expect(out.endsWith("n.com")).toBe(true);
            expect(out.length).toBeLessThanOrEqual(22);
        });

        it("middle-shortens a value with no @ rather than breaking", () => {
            const out = shortenEmail("claude-oauth-account-with-a-long-name");
            expect(out).toContain("…");
            expect(out.length).toBeLessThanOrEqual(22);
        });
    });

    describe("a tighter budget (12)", () => {
        const fits = (s: string) => expect(s.length).toBeLessThanOrEqual(12);

        it("returns an email that fits unchanged", () => {
            expect(shortenEmail("me@gmail.com", 12)).toBe("me@gmail.com");
        });

        it("shortens only the part before @ while the whole domain still fits", () => {
            expect(shortenEmail("jonathan.ross@ex.io", 12)).toBe("jon…ss@ex.io");
        });

        it("then shortens the domain the same way, keeping its TLD", () => {
            expect(shortenEmail("asafebgi@gmail.com", 12)).toBe("a…gi@g…l.com");
            expect(shortenEmail("someone@anthropic.com", 12)).toBe("s…ne@a…c.com");
            fits(shortenEmail("asafebgi@gmail.com", 12));
        });

        it("never shortens a part that is already at or below its minimum", () => {
            expect(shortenEmail("ab@engineering.example.com", 12)).toBe("ab@eng…e.com");
        });

        it("handles a domain without a dot", () => {
            const out = shortenEmail("someone@localhost-machine", 12);
            expect(out.startsWith("s…ne@")).toBe(true);
            fits(out);
        });
    });
});

describe("boundAccountEmail", () => {
    const accounts = [
        { id: "a1", name: "claude-oauth", context: { email: "asafebgi@gmail.com" } },
        { id: "a2", name: "claude-oauth", context: {} },
    ];

    it("is the email of the account the agent is bound to", () => {
        expect(boundAccountEmail(accounts, "a1")).toBe("asafebgi@gmail.com");
    });

    it("is undefined when unbound, unknown, or the account recorded no email", () => {
        expect(boundAccountEmail(accounts, undefined)).toBeUndefined();
        expect(boundAccountEmail(accounts, "missing")).toBeUndefined();
        expect(boundAccountEmail(accounts, "a2")).toBeUndefined();
    });
});
