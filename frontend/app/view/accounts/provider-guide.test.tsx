// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The guide's data: every shown fact has a source and a day, no copy tells history, and what it offers is something
// AgentMux can do. Then the explanation draws only facts that have a source.

import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openLink = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/global", () => ({ openLink }));

import { SERVICE_CATALOG } from "./accounts-catalog";
import { ProviderGuideDetails, ProviderGuideInside } from "./ProviderGuideDetails";
import { guideFor, optionForMode, PROVIDER_GUIDES, shown, type Fact, type ProviderGuide } from "./provider-guide";

afterEach(() => {
    cleanup();
    openLink.mockClear();
});

/** Every fact in a guide, with where it sits. */
function factsOf(g: ProviderGuide): Array<[string, Fact<unknown>]> {
    const out: Array<[string, Fact<unknown>]> = [];
    const add = (where: string, f: Fact<unknown> | undefined) => f && out.push([where, f]);
    for (const o of g.options) {
        const at = `${g.brand}/${o.kind}`;
        add(`${at}/what`, o.what);
        add(`${at}/plans`, o.plans);
        add(`${at}/plansNote`, o.plansNote);
        add(`${at}/billedAgainst`, o.billedAgainst);
        add(`${at}/key.prefix`, o.key?.prefix);
        add(`${at}/key.createAt`, o.key?.createAt);
    }
    for (const p of g.pointers) add(`${g.brand}/pointer/${p.label}`, p.url);
    return out;
}

/** Every string a person reads in a guide. */
function wordsOf(g: ProviderGuide): string[] {
    const out: string[] = [];
    for (const o of g.options) {
        out.push(o.title, o.pickWhen, o.what.value, o.plansNote?.value ?? "", o.billedAgainst?.value ?? "", ...(o.plans?.value ?? []));
    }
    for (const p of g.pointers) out.push(p.label);
    return out;
}

describe("the provider guide's data", () => {
    it("gives every fact an https source and the day it was read", () => {
        for (const g of PROVIDER_GUIDES) {
            for (const [where, f] of factsOf(g)) {
                expect(shown(f), where).toBeTruthy();
                expect(f.source.startsWith("https://"), where).toBe(true);
                expect(f.checked, where).toMatch(/^\d{4}-\d{2}-\d{2}$/);
            }
        }
    });

    it("lists plan names, never an empty list", () => {
        for (const g of PROVIDER_GUIDES) {
            for (const o of g.options) if (o.plans) expect(o.plans.value.length, `${g.brand}/${o.kind}`).toBeGreaterThan(0);
        }
    });

    it("tells no history: current offerings only", () => {
        for (const g of PROVIDER_GUIDES) {
            for (const w of wordsOf(g)) expect(w, `${g.brand}: ${w}`).not.toMatch(/was renamed|moved to|formerly|previously|since \d|used to|no longer/i);
        }
    });

    it("offers a sign-in only for a provider AgentMux runs the login of, and the tile says which", () => {
        for (const g of PROVIDER_GUIDES) {
            const tile = SERVICE_CATALOG.find((t) => t.id === g.brand);
            expect(tile, g.brand).toBeTruthy();
            for (const o of g.options.filter((x) => x.kind === "signin")) {
                expect(o.cli, `${g.brand} sign-in names its CLI`).toBeTruthy();
                expect(PROVIDERS[o.cli!]?.authType, `${g.brand}: ${o.cli}`).toBe("oauth");
                expect(tile?.signIn?.provider).toBe(o.cli);
                expect(tile?.authModes).toContain("oauth");
            }
            for (const o of g.options.filter((x) => x.kind === "key")) {
                expect(tile?.authModes, `${g.brand} key`).toContain("key");
            }
        }
    });

    it("gives OpenAI both its ChatGPT sign-in and its API key", () => {
        const openai = SERVICE_CATALOG.find((t) => t.id === "openai");
        expect(openai?.authModes).toEqual(["oauth", "key"]);
        expect(optionForMode(guideFor("openai"), "oauth")?.title).toBe("Sign in with ChatGPT");
        expect(optionForMode(guideFor("openai"), "key")?.title).toBe("API key");
    });

    it("leaves out a key prefix no vendor page states", () => {
        expect(optionForMode(guideFor("openai"), "key")?.key?.prefix).toBeUndefined();
        expect(optionForMode(guideFor("anthropic"), "key")?.key?.prefix?.value).toBe("sk-ant-api");
    });
});

describe("shown", () => {
    it("drops a fact with no source or no day", () => {
        expect(shown({ value: "x", source: "", checked: "2026-11-14" })).toBeUndefined();
        expect(shown({ value: "x", source: "http://plain.example", checked: "2026-11-14" })).toBeUndefined();
        expect(shown({ value: "x", source: "https://a.example", checked: "yesterday" })).toBeUndefined();
        expect(shown(undefined)).toBeUndefined();
        expect(shown({ value: "x", source: "https://a.example", checked: "2026-11-14" })?.value).toBe("x");
    });
});

describe("ProviderGuideDetails", () => {
    it("draws each option's plans, cost, key and when to pick it, and the vendor's pages", () => {
        render(() => <ProviderGuideDetails guide={guideFor("openai")!} />);
        expect(screen.getByText("Which one should I pick?")).toBeTruthy();
        expect(screen.getByText("Sign in with ChatGPT")).toBeTruthy();
        expect(screen.getByText(/ChatGPT Plus/)).toBeTruthy();
        expect(screen.getByText("Have ChatGPT? Sign in.")).toBeTruthy();
        expect(screen.getByText(/Called "API key" on the vendor's site/)).toBeTruthy();
        screen.getByRole("button", { name: "Make one" }).click();
        expect(openLink).toHaveBeenCalledWith("https://platform.openai.com/api-keys");
        screen.getByRole("button", { name: "ChatGPT plans" }).click();
        expect(openLink).toHaveBeenCalledWith("https://chatgpt.com/pricing");
    });

    it("says the key's start only where a vendor page states it", () => {
        render(() => <ProviderGuideDetails guide={guideFor("anthropic")!} />);
        expect(screen.getByText("sk-ant-api")).toBeTruthy();
    });

    it("draws nothing to pick between for a brand whose agents sign in inside the agent", () => {
        const { container } = render(() => <ProviderGuideDetails guide={guideFor("google")!} />);
        expect(container.querySelector(".accounts-guide")).toBeNull();
        expect(screen.queryByText("Which one should I pick?")).toBeNull();
    });

    it("says what Google's agents ask for inside the agent in plain view, with no disclosure", () => {
        const { container } = render(() => <ProviderGuideInside guide={guideFor("google")!} />);
        expect(screen.getByText("Google's coding agents sign in inside the agent")).toBeTruthy();
        expect(container.querySelector("details")).toBeNull();
        expect(screen.getByText(/Antigravity signs in with the Google account/)).toBeTruthy();
    });

    it("shows nothing inside for a brand with a sign-in and a key to pick between", () => {
        const { container } = render(() => <ProviderGuideInside guide={guideFor("openai")!} />);
        expect(container.querySelector(".accounts-guide")).toBeNull();
    });

    it("leaves out a fact that names no source", () => {
        const g: ProviderGuide = {
            brand: "openai",
            pointers: [{ label: "Unsourced page", url: { value: "https://x.example", source: "", checked: "2026-11-14" } }],
            options: [
                {
                    kind: "key",
                    title: "API key",
                    what: { value: "Sourced sentence.", source: "https://a.example", checked: "2026-11-14" },
                    billedAgainst: { value: "Unsourced cost.", source: "", checked: "2026-11-14" },
                    pickWhen: "Pick me.",
                },
            ],
        };
        render(() => <ProviderGuideDetails guide={g} />);
        expect(screen.getByText("Sourced sentence.")).toBeTruthy();
        expect(screen.queryByText("Unsourced cost.")).toBeNull();
        expect(screen.queryByRole("button", { name: "Unsourced page" })).toBeNull();
    });
});
