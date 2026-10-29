// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a person can do with each provider at no cost, and which providers can
 * use a model on their own computer: the claims behind the Free and Local tags
 * on the agent picker's cards.
 *
 * Every claim carries the page it was read on and the day someone read it, so a
 * claim that has gone stale shows as stale (`provider-access-model.ts`: a Free
 * tag whose oldest claim is more than 90 days old turns neutral and says so)
 * instead of staying blue for as long as nobody looks. Free offers change fast:
 * Gemini CLI's personal-account free tier ended on 2026-06-18 while pages that
 * still describe it are online.
 *
 * Only list a claim a page states. Where a page is silent, say "unknown" (card,
 * phone) or leave the claim out; never fill a gap with what is usual. Re-verify
 * by reading each `source` again and editing `verifiedOn`; see
 * docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §5.
 */

/** One statement, the page it came from and the day it was read there. */
export interface AccessClaim {
    /** One short sentence, in the words a tooltip shows. */
    text: string;
    /** The page it was read on (https). */
    source: string;
    /** The day it was read there, `YYYY-MM-DD`. */
    verifiedOn: string;
}

/** A requirement a page states, or "unknown" when it doesn't say. */
export type Tri = "no" | "yes" | "unknown";

export interface FreeAccess {
    /** `signin`: the vendor's own sign-in. `key`: a free API key the person pastes. */
    route: "signin" | "key";
    /** The tooltip's first line, e.g. "Free with a Google sign-in". */
    lead: string;
    /** The quota, models or region the vendor states. */
    limits: AccessClaim;
    /** Training on data, a rotating lineup, a promotion's end. At most two are shown. */
    catches: AccessClaim[];
    /** Whether a card or a phone number is needed, as the vendor's page says. */
    card: Tri;
    phone: Tri;
    /** The one link: the sign-up or key page. */
    signupUrl: string;
    /** Two or three short lines for after the link. */
    steps: string[];
    /** A promotion with a known end (`YYYY-MM-DD`): after it, there is no Free tag. */
    expiresOn?: string;
}

export interface LocalAccess {
    /** How the CLI itself points at a local model, as its own docs say. */
    how: AccessClaim;
    /** The catch, when there is one (an unsupported path, a rough fit). */
    note?: AccessClaim;
    /** Set once AgentMux has run the CLI against a local model itself. */
    testedOn?: { model: string; date: string; ok: boolean };
}

export interface ProviderAccess {
    /** The provider id, as in `cli-catalog.ts`. */
    provider: string;
    free?: FreeAccess;
    local?: LocalAccess;
    /** A card whose free route moved to another provider. */
    successor?: { provider: string; text: AccessClaim };
    /** A plain line for a card that gets no tag but whose question people ask. */
    plain?: AccessClaim;
}

const CHECKED = "2026-10-29";

function claim(text: string, source: string, verifiedOn: string = CHECKED): AccessClaim {
    return { text, source, verifiedOn };
}

export const PROVIDER_ACCESS: readonly ProviderAccess[] = [
    {
        provider: "claude",
        local: {
            how: claim(
                "Works through Ollama's or LM Studio's Anthropic endpoint (ANTHROPIC_BASE_URL).",
                "https://docs.ollama.com/integrations/claude-code"
            ),
            note: claim(
                "Anthropic doesn't support Claude Code on non-Claude models; expect rough edges.",
                "https://code.claude.com/docs/en/llm-gateway"
            ),
        },
        plain: claim(
            "No free route from Anthropic: the free claude.ai plan doesn't include Claude Code.",
            "https://code.claude.com/docs/en/setup"
        ),
    },
    {
        provider: "codex",
        local: {
            how: claim(
                "Works with Ollama or LM Studio through the Responses API (--oss).",
                "https://learn.chatgpt.com/docs/config-file/config-advanced"
            ),
            note: claim(
                "No verified free cloud route. Rough fit: open issues with MCP tools on local servers.",
                "https://github.com/openai/codex/issues/26234"
            ),
        },
        plain: claim(
            "Whether the CLI works on a ChatGPT Free account isn't confirmed. Works with Ollama or LM Studio through the Responses API.",
            "https://learn.chatgpt.com/docs/pricing"
        ),
    },
    {
        provider: "gemini",
        free: {
            route: "key",
            lead: "Free with an AI Studio key",
            limits: claim(
                "250 requests a day, Flash models only.",
                "https://geminicli.com/docs/resources/quota-and-pricing/"
            ),
            catches: [
                claim(
                    "Free-tier content is used to improve Google's products (outside the EEA, Switzerland and the UK).",
                    "https://ai.google.dev/gemini-api/docs/pricing"
                ),
                claim(
                    "The old personal-account free tier moved to Antigravity on 2026-06-18.",
                    "https://github.com/google-gemini/gemini-cli"
                ),
            ],
            card: "unknown",
            phone: "unknown",
            signupUrl: "https://aistudio.google.com/apikey",
            steps: [
                "Create a key at the link.",
                "Paste it here. It is stored in your account keychain, never in the chat.",
                "Start the agent.",
            ],
        },
        successor: {
            provider: "antigravity",
            text: claim(
                "For a personal Google account, use Antigravity.",
                "https://github.com/google-gemini/gemini-cli"
            ),
        },
    },
    {
        provider: "antigravity",
        free: {
            route: "signin",
            lead: "Free with a Google sign-in",
            limits: claim(
                "A weekly quota that Google doesn't publish. Gemini Flash-class models; other vendors' models need a paid plan.",
                "https://antigravity.google/docs/plans"
            ),
            catches: [
                claim("Free users can't buy overage credits.", "https://antigravity.google/docs/plans"),
            ],
            card: "unknown",
            phone: "unknown",
            signupUrl: "https://antigravity.google/docs/getting-started?tab=cli",
            steps: [
                "Sign in with your Google account when the browser opens.",
                "Come back here. AgentMux sees the sign-in.",
                "Type /usage in the agent to see what is left of your weekly quota.",
            ],
        },
    },
    {
        provider: "qwen",
        local: {
            how: claim(
                "Point it at an OpenAI-compatible server (OPENAI_BASE_URL); its README names Ollama and vLLM.",
                "https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/"
            ),
            note: claim(
                "Open bugs on Ollama: zero-argument tools are rejected (#12878).",
                "https://github.com/QwenLM/qwen-code/issues/12878"
            ),
        },
        plain: claim(
            "Its own free sign-in ended on 2026-04-15.",
            "https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/"
        ),
    },
    {
        provider: "kimi",
        plain: claim(
            "Not free: Kimi Code is a membership benefit and the API needs a $1 minimum recharge.",
            "https://www.kimi.com/code/docs/en/"
        ),
    },
    {
        provider: "pi",
        local: {
            how: claim(
                "Add an Ollama or LM Studio provider to models.json (api: openai-completions).",
                "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/models.md"
            ),
        },
    },
    {
        provider: "openclaw",
        local: {
            how: claim(
                "Use Ollama's native API URL, not the /v1 one.",
                "https://docs.openclaw.ai/providers/ollama"
            ),
            note: claim(
                "Its docs say the /v1 URL breaks tool calling.",
                "https://docs.openclaw.ai/providers/ollama"
            ),
        },
    },
    {
        provider: "copilot",
        free: {
            route: "signin",
            lead: "Free with the Copilot Free plan",
            limits: claim(
                "A small AI-credit allowance (the amount isn't published); auto model only.",
                "https://docs.github.com/en/copilot/get-started/plans"
            ),
            catches: [
                claim(
                    "For individuals only, not for people who get Copilot through an organization.",
                    "https://docs.github.com/en/copilot/get-started/plans"
                ),
            ],
            card: "unknown",
            phone: "unknown",
            signupUrl: "https://github.com/features/copilot",
            steps: [
                "Choose the Free plan on that page, with your GitHub account.",
                "Sign in when the browser or device code asks.",
                "Pick Auto: it is the only model on the Free plan.",
            ],
        },
        local: {
            how: claim(
                "Finds Ollama on this computer by itself (127.0.0.1:11434); the model needs tool calls and streaming.",
                "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models"
            ),
        },
    },
    {
        provider: "kilo",
        free: {
            route: "signin",
            lead: "Free models, no card",
            limits: claim(
                "200 requests an hour per network address on the free models.",
                "https://kilo.ai/docs/gateway/models-and-providers"
            ),
            catches: [
                claim(
                    "The lineup changes: hosted promotions can end.",
                    "https://kilo.ai/landing/free-models"
                ),
                claim(
                    "Some free models require data collection.",
                    "https://kilo.ai/docs/gateway/usage-and-billing"
                ),
            ],
            card: "no",
            phone: "unknown",
            signupUrl: "https://app.kilo.ai",
            steps: [
                "Create a free account at the link.",
                "In Kilo, run /connect and sign in.",
                "Pick Auto Free, or a model marked Free.",
            ],
        },
        local: {
            how: claim(
                "An OpenCode fork with the same config: add an Ollama provider to kilo.jsonc.",
                "https://kilo.ai/docs/code-with-ai/platforms/cli"
            ),
        },
    },
    {
        provider: "cline",
        free: {
            route: "signin",
            lead: "Free models with a Cline account",
            limits: claim(
                "A rotating promotion with a quota that isn't published.",
                "https://docs.cline.bot/getting-started/free-models"
            ),
            catches: [
                claim(
                    "Free usage may be used to help improve model performance.",
                    "https://docs.cline.bot/getting-started/free-models"
                ),
            ],
            card: "unknown",
            phone: "unknown",
            signupUrl: "https://app.cline.bot",
            steps: [
                "Sign up with Google, GitHub or email.",
                "Run cline auth and sign in.",
                "Set the provider to Cline and pick a model tagged FREE.",
            ],
        },
        local: {
            how: claim(
                "Supports Ollama, LM Studio and OpenAI-compatible endpoints (cline auth -p <provider> -b <base url>).",
                "https://docs.cline.bot/cli/cli-reference.md"
            ),
        },
    },
    {
        provider: "opencode",
        free: {
            route: "signin",
            lead: "Free models through OpenCode",
            limits: claim(
                "No number is published, and users report hitting a limit.",
                "https://opencode.ai/v2/docs/console/models/"
            ),
            catches: [
                claim(
                    "The lineup rotates; every free model is listed as free for a limited time.",
                    "https://opencode.ai/v2/docs/console/models/"
                ),
                claim(
                    "Some free models may use your data to improve the model. They work only inside OpenCode.",
                    "https://opencode.ai/v2/docs/console/models/"
                ),
            ],
            card: "unknown",
            phone: "unknown",
            signupUrl: "https://opencode.ai/auth",
            steps: [
                "Run OpenCode and open /models.",
                "Pick a model marked free. The list changes.",
                "If you later want paid models, /connect takes a Console key.",
            ],
        },
        local: {
            how: claim(
                "Add an @ai-sdk/openai-compatible provider to opencode.json; v2 finds Ollama on 127.0.0.1:11434 by itself.",
                "https://opencode.ai/docs/providers/"
            ),
            note: claim(
                "Its docs warn that few local models are good at both code and tool calling.",
                "https://opencode.ai/docs/models/"
            ),
        },
    },
    {
        provider: "goose",
        local: {
            how: claim(
                "Supports Ollama (OLLAMA_HOST), LM Studio and OpenAI-compatible hosts.",
                "https://goose-docs.ai/docs/getting-started/providers/"
            ),
            note: claim(
                "It relies on tool calling: a model without it can only chat.",
                "https://goose-docs.ai/docs/getting-started/providers/"
            ),
        },
    },
    {
        provider: "grok",
        free: {
            route: "signin",
            lead: "Free to try with a Grok or X account",
            limits: claim(
                "The limits aren't published. A free Grok 4.5 promotion was \"for a limited time\".",
                "https://x.ai/build"
            ),
            catches: [
                claim(
                    "The launch announcement said SuperGrok or X Premium Plus only; the product page now says free to try.",
                    "https://x.ai/news/grok-4-5"
                ),
            ],
            card: "unknown",
            phone: "unknown",
            signupUrl: "https://x.ai/build",
            steps: [
                "Sign in with a grok.com or X account when the browser opens.",
                "On a computer with no browser, use the device-code sign-in.",
                "Pick a model with /model.",
            ],
        },
        local: {
            how: claim(
                "Add a [model.<name>] with base_url to ~/.grok/config.toml.",
                "https://github.com/xai-org/grok-build"
            ),
        },
    },
    {
        provider: "dsh",
        local: {
            how: claim(
                "ollama launch dsh sets it up; a developer preview.",
                "https://github.com/ollama/ollama/blob/main/docs/integrations/deepseek-harness.mdx"
            ),
        },
    },
    {
        provider: "aider",
        local: {
            how: claim(
                "Set OLLAMA_API_BASE and use ollama_chat/<model>; LM Studio works too.",
                "https://aider.chat/docs/llms/ollama.html"
            ),
            note: claim(
                "Edits as text, so small models do better here than in tool-calling agents. No release since 2025-08.",
                "https://aider.chat/docs/more/edit-formats.html"
            ),
        },
    },
    {
        provider: "crush",
        local: {
            how: claim(
                "Add an Ollama or OpenAI-compatible provider (provider add ... --base-url).",
                "https://github.com/charmbracelet/crush"
            ),
        },
        plain: claim(
            "Not free: its own provider is a subscription.",
            "https://github.com/charmbracelet/crush"
        ),
    },
    {
        provider: "muse",
        plain: claim(
            "Not free: a subscription or pay-per-token. No local model support found.",
            "https://dev.meta.ai/docs/muse-code/subscriptions"
        ),
    },
];

const BY_PROVIDER = new Map(PROVIDER_ACCESS.map((a) => [a.provider, a]));

/** What is known about `provider`'s free and local use; undefined when nothing is. */
export function getProviderAccess(provider: string): ProviderAccess | undefined {
    return BY_PROVIDER.get((provider || "").toLowerCase());
}
