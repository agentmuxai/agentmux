// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The guide's data: every shown fact has a source and a day, no copy tells history, and what it offers is something
// AgentMux can do. Then the explanation draws only facts that have a source.

import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { ACCOUNT_BRAND_ORDER } from "@/app/view/identity/identity-model";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openLink = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/global", () => ({ openLink }));

import { SERVICE_CATALOG } from "./accounts-catalog";
import { CLI_SIGN_INS } from "./cli-sign-ins";
import { ProviderGuideDetails, ProviderGuideInside } from "./ProviderGuideDetails";
import { OtherAgentsSection, otherAgentsShown } from "./OtherAgentsSection";
import { guideFor, optionForMode, OTHER_AGENTS, PROVIDER_GUIDES, shown, type Fact, type ProviderGuide } from "./provider-guide";

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
        out.push(o.title, o.pickWhen ?? "", o.what.value, o.plansNote?.value ?? "", o.billedAgainst?.value ?? "", ...(o.plans?.value ?? []));
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

describe("the tiles that have no sign-in to run", () => {
    it("gives xAI text only: a tile with no way to connect, and a guide with nothing to pick", () => {
        const xai = SERVICE_CATALOG.find((t) => t.id === "xai");
        expect(xai?.authModes).toEqual([]);
        expect(xai?.signIn).toBeUndefined();
        expect(guideFor("xai")?.options.map((o) => o.kind)).toEqual(["inside"]);
    });

    it("gives Moonshot text only: Kimi Code's sign-in, and no key form until a key is seen reaching the CLI", () => {
        const kimi = SERVICE_CATALOG.find((t) => t.id === "kimi");
        expect(kimi?.displayName).toBe("Moonshot");
        expect(kimi?.authModes).toEqual([]);
        expect(kimi?.signIn).toBeUndefined();
        expect(guideFor("kimi")?.options.map((o) => o.kind)).toEqual(["inside"]);
        expect(optionForMode(guideFor("kimi"), "key")).toBeUndefined();
    });

    it("puts Copilot under GitHub as text, with no sign-in of its own", () => {
        const github = guideFor("github");
        expect(github?.options.map((o) => o.kind)).toEqual(["inside"]);
        expect(github?.options[0].plans?.value).toContain("Copilot Pro+");
        expect(SERVICE_CATALOG.find((t) => t.id === "github")?.signIn).toBeUndefined();
        const signIns = PROVIDER_GUIDES.flatMap((g) => g.options).filter((o) => o.kind === "signin");
        expect(signIns.filter((o) => o.cli === "copilot" || o.cli === "grok")).toEqual([]);
    });
});

describe("the other agents' data", () => {
    it("gives each line an https source and the day it was read, and tells no history", () => {
        for (const a of OTHER_AGENTS) {
            const facts = [a.line, a.key?.createAt, ...(a.key?.notes ?? [])].filter((f): f is Fact<string> => !!f);
            for (const f of facts) {
                expect(shown(f), `${a.id}: ${f.value}`).toBeTruthy();
                expect(f.source.startsWith("https://"), `${a.id}: ${f.value}`).toBe(true);
                expect(f.value, `${a.id}: ${f.value}`).not.toMatch(/was renamed|moved to|formerly|previously|since \d|used to|no longer/i);
            }
        }
    });

    it("names only agents the catalog has, once each, with an https docs link, and none that has a tile", () => {
        const ids = OTHER_AGENTS.map((a) => a.id);
        expect(new Set(ids).size).toBe(ids.length);
        const tileClis = new Set(SERVICE_CATALOG.flatMap((t) => (t.signIn ? [t.signIn.provider] : [])));
        for (const id of ids) {
            expect(PROVIDERS[id], id).toBeTruthy();
            expect(PROVIDERS[id].docsUrl.startsWith("https://"), id).toBe(true);
            expect(tileClis.has(id), id).toBe(false);
            expect(
                SERVICE_CATALOG.some((t) => t.id === id),
                id
            ).toBe(false);
        }
        expect(otherAgentsShown().map((a) => a.id)).toEqual(ids);
    });

    it("sends a key to the vendor the catalog says the agent runs on", () => {
        for (const a of OTHER_AGENTS.filter((x) => x.key)) {
            const vendor = PROVIDERS[a.id].supportedVendors?.[0];
            expect(["openrouter", "deepseek"], a.id).toContain(vendor);
            expect(a.key?.createAt.value, a.id).toBe(
                vendor === "openrouter" ? "https://openrouter.ai/keys" : "https://platform.deepseek.com/api_keys"
            );
        }
    });
});

describe("OtherAgentsSection", () => {
    it("lists each agent with its line and a docs link from the catalog, collapsed until opened", () => {
        const { container } = render(() => <OtherAgentsSection />);
        expect(container.querySelector("details.accounts-other")?.hasAttribute("open")).toBe(false);
        expect(screen.getByText(`Other agents (${OTHER_AGENTS.length})`)).toBeTruthy();
        expect(screen.getByText("Muse Code")).toBeTruthy();
        expect(screen.getByText("Asks you to sign in with your browser or an API key the first time it runs.")).toBeTruthy();
        const row = container.querySelector('[data-agent="goose"]') as HTMLElement;
        (row.querySelector("button") as HTMLButtonElement).click();
        expect(openLink).toHaveBeenCalledWith(PROVIDERS["goose"].docsUrl);
    });

    it("gives OpenRouter's key to Qwen Code and DeepSeek's to its harness", () => {
        const { container } = render(() => <OtherAgentsSection />);
        const keyButton = (row: HTMLElement, label: string) =>
            Array.from(row.querySelectorAll("button")).find((b) => b.textContent === label) as HTMLButtonElement;
        const qwen = container.querySelector('[data-agent="qwen"]') as HTMLElement;
        keyButton(qwen, "OpenRouter key").click();
        expect(openLink).toHaveBeenCalledWith("https://openrouter.ai/keys");
        expect(qwen.textContent).toContain("sk-or-v1-");
        const dsh = container.querySelector('[data-agent="dsh"]') as HTMLElement;
        keyButton(dsh, "DeepSeek key").click();
        expect(openLink).toHaveBeenCalledWith("https://platform.deepseek.com/api_keys");
    });

    it("draws no key for an agent that has none", () => {
        const { container } = render(() => <OtherAgentsSection />);
        expect(container.querySelector('[data-agent="aider"] .accounts-other-key')).toBeNull();
    });
});

describe("the tile guides draw", () => {
    it("Grok Build's sign-in in plain view, with its docs page", () => {
        const { container } = render(() => <ProviderGuideInside guide={guideFor("xai")!} />);
        expect(container.querySelector("details")).toBeNull();
        expect(screen.getByText("Grok Build signs in inside the agent")).toBeTruthy();
        screen.getByRole("button", { name: "Grok Build docs" }).click();
        expect(openLink).toHaveBeenCalledWith("https://docs.x.ai/build/overview");
    });

    it("Kimi Code's own sign-in in plain view, with the platform docs", () => {
        const { container } = render(() => <ProviderGuideInside guide={guideFor("kimi")!} />);
        expect(container.querySelector("details")).toBeNull();
        expect(screen.getByText("Kimi Code signs in inside the agent")).toBeTruthy();
        expect(screen.getByText(/open a Kimi agent and run \/login/)).toBeTruthy();
        screen.getByRole("button", { name: "Kimi API Platform docs" }).click();
        expect(openLink).toHaveBeenCalledWith("https://platform.kimi.ai/docs/overview");
    });

    it("Copilot's plans under GitHub, with nothing to pick between", () => {
        const g = guideFor("github")!;
        const { container } = render(() => (
            <>
                <ProviderGuideInside guide={g} />
                <ProviderGuideDetails guide={g} />
            </>
        ));
        expect(screen.getByText("GitHub Copilot signs in inside the agent")).toBeTruthy();
        expect(screen.getByText(/Copilot Student/)).toBeTruthy();
        expect(screen.getByText("Usage is counted in GitHub AI Credits.")).toBeTruthy();
        expect(container.querySelector("details")).toBeNull();
    });
});

describe("the CLI sign-ins", () => {
    it("are exactly the tiles that run a sign-in, with the brand each names", () => {
        const fromTiles = Object.fromEntries(SERVICE_CATALOG.flatMap((t) => (t.signIn ? [[t.signIn.provider, t.signIn.brand]] : [])));
        expect(fromTiles).toEqual(CLI_SIGN_INS);
    });
});

describe("the accounts' brands", () => {
    it("group under every tile, so a connected account counts on its tile", () => {
        expect([...ACCOUNT_BRAND_ORDER].sort()).toEqual(SERVICE_CATALOG.map((t) => t.id).sort());
    });
});
