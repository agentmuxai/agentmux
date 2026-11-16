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
    /** When to pick this one, in a sentence; left out where there is nothing to pick between. */
    pickWhen?: string;
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

/** The day the PR 1 pages were read, and the day the pages added after them were. */
const CHECKED = "2026-11-14";
const CHECKED_2 = "2026-11-16";
const fact = <T>(value: T, source: string, checked = CHECKED): Fact<T> => ({ value, source, checked });
const read2 = <T>(value: T, source: string): Fact<T> => fact(value, source, CHECKED_2);

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
const COPILOT_PLANS = "https://docs.github.com/en/copilot/get-started/plans";
const COPILOT_SIGN_IN = "https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli";
const GROK_AUTH =
    "https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/02-authentication.md";
const GROK_DOCS = "https://docs.x.ai/build/overview";
const KIMI_PLATFORM = "https://platform.kimi.ai/docs/overview";
const KIMI_PRODUCTS = "https://platform.kimi.ai/docs/guide/product-plans";
const KIMI_CODE_PROVIDERS = "https://moonshotai.github.io/kimi-code/en/configuration/providers";
const OPENROUTER_AUTH = "https://openrouter.ai/docs/api-reference/authentication";
const OPENROUTER_FAQ = "https://openrouter.ai/docs/faq";
const DEEPSEEK_DOCS = "https://api-docs.deepseek.com";
const DEEPSEEK_PRICING = "https://api-docs.deepseek.com/quick_start/pricing";

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
    {
        // GitHub's tile connects the GitHub App and a token (github-account-copy.ts); Copilot CLI signs in inside the agent.
        brand: "github",
        options: [
            {
                kind: "inside",
                title: "GitHub Copilot signs in inside the agent",
                what: read2(
                    "Uses your GitHub account. Copilot CLI signs in through your browser, or with a code on a remote machine; /login inside it lets you pick.",
                    COPILOT_SIGN_IN
                ),
                plans: read2(
                    ["Copilot Free", "Copilot Student", "Copilot Pro", "Copilot Pro+", "Copilot Max", "Copilot Business", "Copilot Enterprise"],
                    COPILOT_PLANS
                ),
                plansNote: read2("Every plan includes Copilot CLI.", COPILOT_PLANS),
                billedAgainst: read2("Usage is counted in GitHub AI Credits.", COPILOT_PLANS),
                drives: ["GitHub Copilot CLI"],
            },
        ],
        pointers: [{ label: "Copilot plans", url: read2(COPILOT_PLANS, COPILOT_PLANS) }],
    },
    {
        // xAI has no account to connect here: Grok Build asks for its own sign-in.
        brand: "xai",
        options: [
            {
                kind: "inside",
                title: "Grok Build signs in inside the agent",
                what: read2(
                    "Grok Build opens your browser to sign in the first time it runs, or shows a code to enter on a remote machine. Open a Grok agent and it asks you.",
                    GROK_AUTH
                ),
                drives: ["Grok Build"],
            },
        ],
        pointers: [{ label: "Grok Build docs", url: read2(GROK_DOCS, GROK_DOCS) }],
    },
    {
        brand: "kimi",
        options: [
            {
                kind: "key",
                title: "API key",
                what: read2("A key from the Kimi API Platform.", KIMI_PLATFORM),
                billedAgainst: read2(
                    "Pay as you go on the Kimi API Platform, which has no subscription plan. This is separate from Kimi Membership and Kimi Code.",
                    KIMI_PRODUCTS
                ),
                drives: ["Kimi Code CLI"],
                key: {
                    calledOnVendorSite: "API Key",
                    createAt: read2("https://platform.kimi.ai/console/api-keys", KIMI_PLATFORM),
                },
                pickWhen: "Have a Kimi API Platform key? Add it.",
            },
            {
                kind: "inside",
                title: "Kimi Code signs in inside the agent",
                what: read2("Kimi Code has its own sign-in: open a Kimi agent and run /login.", KIMI_CODE_PROVIDERS),
            },
        ],
        pointers: [],
    },
];

/** A key an agent that is not one of the tiles asks for, and the vendor page that makes one. */
export interface AgentKeyPointer {
    label: string;
    createAt: Fact<string>;
    /** What the vendor says about it (its start, how it is paid for). */
    notes: Fact<string>[];
}

/** An agent with no tile of its own: one line on how it signs in, from the page that says so. Its name and its
 *  docs link come from the agent catalog (providers/catalog.ts), so a rename or a new link is one change there. */
export interface OtherAgent {
    /** The agent's id in the catalog. */
    id: string;
    /** What its own docs say about signing in or giving it a key. */
    line: Fact<string>;
    /** For an agent that runs on a model vendor's key: where that key is made (a test holds it to the vendor the catalog names). */
    key?: AgentKeyPointer;
}

const OPENROUTER_KEY: AgentKeyPointer = {
    label: "OpenRouter key",
    createAt: read2("https://openrouter.ai/keys", OPENROUTER_AUTH),
    notes: [
        read2("A key starts sk-or-v1- and can have a credit limit.", OPENROUTER_AUTH),
        read2("Usage is paid from your OpenRouter credits.", OPENROUTER_FAQ),
    ],
};

const DEEPSEEK_KEY: AgentKeyPointer = {
    label: "DeepSeek key",
    createAt: read2("https://platform.deepseek.com/api_keys", DEEPSEEK_DOCS),
    notes: [read2("Billed per token, from your topped-up or granted balance.", DEEPSEEK_PRICING)],
};

/** The agents under "Other agents", in the order the list shows them. Each line says only what that agent's own page says:
 *  some prompt the first time they run, some have a command that does it. */
export const OTHER_AGENTS: OtherAgent[] = [
    {
        id: "muse",
        line: read2("Asks you to sign in with your browser or an API key the first time it runs.", "https://dev.meta.ai/docs/muse-code"),
    },
    {
        id: "qwen",
        line: read2(
            "Opens an /auth menu with its sign-in options the first time it runs.",
            "https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/"
        ),
        key: OPENROUTER_KEY,
    },
    {
        id: "dsh",
        line: read2("Run dsh web to open its web interface.", "https://github.com/deepseek-ai/deepseek-harness"),
        key: DEEPSEEK_KEY,
    },
    {
        id: "pi",
        line: read2("Run /login inside Pi to connect a subscription or an API key.", "https://github.com/badlogic/pi-mono"),
    },
    {
        id: "kilo",
        line: read2(
            "Run kilo, then /connect to add your first provider's credentials.",
            "https://kilo.ai/docs/code-with-ai/platforms/cli"
        ),
    },
    {
        id: "cline",
        line: read2("Run cline auth for an interactive setup.", "https://docs.cline.bot/cli/cli-reference"),
    },
    {
        id: "opencode",
        line: read2("Use /connect inside OpenCode to add a provider.", "https://opencode.ai/docs/"),
    },
    {
        id: "goose",
        line: read2("Asks you to set up a provider the first time you use it.", "https://goose-docs.ai/docs/getting-started/installation"),
    },
    {
        id: "aider",
        line: read2(
            "Takes keys from the command line, environment variables, a .env file or its YAML config.",
            "https://aider.chat/docs/config/api-keys.html"
        ),
    },
    {
        id: "crush",
        line: read2("Press ctrl+l to pick a model, then paste your API key.", "https://github.com/charmbracelet/crush"),
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
