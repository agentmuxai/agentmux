// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What Connectors tells the person about each way to connect a provider: the plans a sign-in works with, what
 * it is billed against, the key's name and where to get one, and the pages to look at (SPEC_CONNECTORS_EVERY_WAY_TO_CONNECT_2026_11_14.md).
 *
 * This file is the one place a plan name, a key prefix, a price page or a link lives; components read it and write
 * none of it. Every fact carries the page it was read on and the day, and `shown()` drops one that has no source,
 * so a plan name nobody checked can't reach the screen. To change one, re-read its page, then change `value`,
 * `source` and `checked` together.
 */

import type { AccountProvider } from "@/app/view/identity/identity-model";

/** One checked fact: where it was read and the day it was read there (YYYY-MM-DD). */
export interface Fact<T> {
    value: T;
    source: string;
    checked: string;
}

export interface ConnectOption {
    /** A sign-in AgentMux runs, an account key it stores, or a sign-in the agent's own CLI asks for in its pane. */
    kind: "signin" | "key" | "inside";
    title: string;
    /** One plain sentence: what it is. */
    what: Fact<string>;
    /** The plan names it works with, as the vendor writes them. */
    plans?: Fact<string[]>;
    /** A line about plans the list above leaves out. */
    plansNote?: Fact<string>;
    /** What using it is billed against. */
    billedAgainst?: Fact<string>;
    /** The agent CLIs it signs in. */
    drives?: string[];
    /** For a key: what the vendor calls it, its usual start, the page that makes one. */
    key?: { calledOnVendorSite: string; prefix?: Fact<string>; createAt: Fact<string> };
    /** When to pick this one, in a sentence. */
    pickWhen: string;
    /** For a sign-in: the provider id of the CLI AgentMux runs (a key of the provider catalog). */
    cli?: string;
}

export interface Pointer {
    label: string;
    url: Fact<string>;
}

export interface ProviderGuide {
    brand: AccountProvider;
    options: ConnectOption[];
    pointers: Pointer[];
}

const CHECKED = "2026-11-14";
const fact = <T>(value: T, source: string): Fact<T> => ({ value, source, checked: CHECKED });

const CLAUDE_PLAN_HELP = "https://support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan";
const CLAUDE_PRICING = "https://claude.com/pricing";
const CLAUDE_KEYS_DOC = "https://platform.claude.com/docs/en/manage-claude/authentication";
const CLAUDE_API_DOC = "https://platform.claude.com/docs/en/api/overview";
const CODEX_AUTH = "https://learn.chatgpt.com/docs/auth";
const CODEX_PRICING = "https://learn.chatgpt.com/codex/pricing";
const CHATGPT_PRICING = "https://chatgpt.com/pricing";
const OPENAI_QUICKSTART = "https://developers.openai.com/api/docs/quickstart";
const GEMINI_README = "https://github.com/google-gemini/gemini-cli";
const GEMINI_AUTH = "https://geminicli.com/docs/get-started/authentication/";
const ANTIGRAVITY_INSTALL = "https://antigravity.google/docs/install/";

export const PROVIDER_GUIDES: ProviderGuide[] = [
    {
        brand: "anthropic",
        options: [
            {
                kind: "signin",
                cli: "claude",
                title: "Sign in with your Claude plan",
                what: fact("Use your Claude account. Claude Code runs on your plan's usage limits, with no per-token bill.", CLAUDE_PLAN_HELP),
                plans: fact(["Claude Pro", "Claude Max", "Claude Ultra"], CLAUDE_PLAN_HELP),
                plansNote: fact("Team and Enterprise seats include Claude Code too.", CLAUDE_PRICING),
                billedAgainst: fact(
                    "Your plan's usage limits, which Claude and Claude Code share. If you run out, you can add usage credits.",
                    CLAUDE_PLAN_HELP
                ),
                drives: ["Claude Code"],
                pickWhen: "Have a Claude plan? Sign in.",
            },
            {
                kind: "key",
                title: "API key",
                what: fact("A key from the Claude Console, billed per token on your Console account.", CLAUDE_KEYS_DOC),
                billedAgainst: fact(
                    "Per token, on your Claude Console account. This is separate from any Claude plan, and while a key is set Claude Code uses it instead of your plan.",
                    CLAUDE_PLAN_HELP
                ),
                drives: ["Claude Code"],
                key: {
                    calledOnVendorSite: "API key",
                    prefix: fact("sk-ant-api", CLAUDE_KEYS_DOC),
                    createAt: fact("https://platform.claude.com/settings/keys", CLAUDE_KEYS_DOC),
                },
                pickWhen: "Building an app, running automation, or want per-token control? Use a key.",
            },
        ],
        pointers: [
            { label: "Pricing", url: fact(CLAUDE_PRICING, CLAUDE_PRICING) },
            { label: "API rate limits", url: fact("https://platform.claude.com/usage/limits", CLAUDE_API_DOC) },
            { label: "API spend limits", url: fact("https://platform.claude.com/settings/spend-limits", CLAUDE_API_DOC) },
        ],
    },
    {
        brand: "openai",
        options: [
            {
                kind: "signin",
                cli: "codex",
                title: "Sign in with ChatGPT",
                what: fact("Use your ChatGPT account. Codex runs on your plan's included usage.", CODEX_AUTH),
                plans: fact(
                    ["ChatGPT Free", "ChatGPT Go", "ChatGPT Plus", "ChatGPT Pro", "ChatGPT Business", "ChatGPT Edu", "ChatGPT Enterprise"],
                    CODEX_PRICING
                ),
                plansNote: fact("Free and Go include a limited amount of Codex.", CHATGPT_PRICING),
                billedAgainst: fact("Your ChatGPT plan's included Codex usage. Codex cloud needs this sign-in.", CODEX_AUTH),
                drives: ["Codex CLI"],
                pickWhen: "Have ChatGPT? Sign in.",
            },
            {
                kind: "key",
                title: "API key",
                what: fact("A key from the OpenAI Platform. Codex then bills per token at API prices.", CODEX_AUTH),
                billedAgainst: fact(
                    "Per token, at standard API pricing, on your OpenAI API account, not your ChatGPT plan. It has no Codex cloud features: no cloud chats, PR reviews, or Slack and Linear.",
                    CODEX_PRICING
                ),
                drives: ["Codex CLI"],
                key: {
                    calledOnVendorSite: "API key",
                    createAt: fact("https://platform.openai.com/api-keys", OPENAI_QUICKSTART),
                },
                pickWhen: "Building an app, or have no ChatGPT plan? Use a key.",
            },
        ],
        pointers: [
            { label: "ChatGPT plans", url: fact(CHATGPT_PRICING, CHATGPT_PRICING) },
            { label: "Codex plans", url: fact(CODEX_PRICING, CODEX_PRICING) },
            { label: "API billing", url: fact("https://platform.openai.com/account/billing/overview", OPENAI_QUICKSTART) },
        ],
    },
    {
        // The Google tile connects Gmail, Calendar and Drive; Google's coding agents sign in inside the agent.
        brand: "google",
        options: [
            {
                kind: "inside",
                title: "Google's coding agents sign in inside the agent",
                what: fact(
                    "Antigravity signs in with the Google account that holds your Google AI plan. Gemini CLI is for organisations with Gemini Code Assist licences. Open an agent of that kind and it asks you.",
                    GEMINI_README
                ),
                plans: fact(["Free", "Pro", "Ultra"], ANTIGRAVITY_INSTALL),
                key: {
                    calledOnVendorSite: "Gemini API key",
                    createAt: fact("https://aistudio.google.com/apikey", GEMINI_AUTH),
                },
                pickWhen: "A Gemini API key from Google AI Studio also works there.",
            },
        ],
        pointers: [],
    },
];

export function guideFor(brand: AccountProvider): ProviderGuide | undefined {
    return PROVIDER_GUIDES.find((g) => g.brand === brand);
}

/** The option a chooser mode stands for: "oauth" is the guide's sign-in, "key" its key. */
export function optionForMode(guide: ProviderGuide | undefined, mode: "oauth" | "key"): ConnectOption | undefined {
    return guide?.options.find((o) => o.kind === (mode === "oauth" ? "signin" : "key"));
}

/** A fact that may be shown: it names an https page it was read on and the day. Anything else is left out. */
export function shown<T>(f: Fact<T> | undefined): Fact<T> | undefined {
    if (!f || !f.source.startsWith("https://") || !/^\d{4}-\d{2}-\d{2}$/.test(f.checked)) return undefined;
    return f;
}

/** The sentence a source tip says. */
export const sourceTip = (f: Fact<unknown>): string => `Source: ${f.source}, read ${f.checked}`;
