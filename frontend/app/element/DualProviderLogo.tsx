// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DualProviderLogo — renders a primary Agent Harness icon with a small
 * overlaid Model Vendor badge, realizing the harness-vs-vendor decoupling
 * (`docs/specs/` — the `model_vendor_base_url`/`supportedVendors` work)
 * everywhere an agent is shown: primary icon = the execution harness
 * (e.g. Claude Code, Antigravity, Codex, OpenClaw); overlay badge = the
 * upstream intelligence vendor actually serving responses (e.g. Anthropic,
 * Google, OpenAI, or "custom" when the agent has a
 * `model_vendor_base_url` override — see
 * `providers/catalog.ts`'s `resolveEffectiveVendor`).
 *
 * The badge is omitted when vendor === harness (the common case for a
 * single-vendor provider like claude/codex/gemini with no override) — it
 * would be redundant, and every existing single-icon call site keeps
 * looking exactly the same.
 */

import { ProviderLogo } from "@/element/ProviderLogo";
import type { JSX } from "solid-js";
import { tip } from "@/app/element/tip";

export interface DualProviderLogoProps {
    /** Harness/provider id (e.g. "claude", "antigravity", "codex", "openclaw"). */
    harness: string;
    /** Model vendor id (e.g. "anthropic", "google", "openai", "custom"). */
    vendor?: string;
    /** Base pixel size of the primary harness logo (default: 24). */
    size?: number;
    /** What to call the vendor in the tooltip when the id alone says too little (a route's model: "DeepSeek V4.1 Flash via OpenRouter"). */
    vendorLabel?: string;
    /** No tooltip or accessible name of its own: the parent (a header button) names it with `dualLogoTitle`. */
    bare?: boolean;
    /** Additional CSS class names. */
    class?: string;
}

const HARNESS_NAMES: Record<string, string> = {
    claude: "Claude Code",
    codex: "Codex CLI",
    gemini: "Gemini CLI",
    qwen: "Qwen Code",
    kimi: "Kimi Code CLI",
    openclaw: "OpenClaw",
    pi: "Pi",
    muxcode: "Mux Code",
    copilot: "GitHub Copilot CLI",
    antigravity: "Antigravity (AGY)",
    kilo: "Kilo Code CLI",
    cline: "Cline CLI",
    opencode: "OpenCode",
    goose: "goose",
    grok: "Grok Build",
    dsh: "DeepSeek Harness",
    aider: "Aider",
    crush: "Crush",
    muse: "Muse Code",
};

const VENDOR_NAMES: Record<string, string> = {
    anthropic: "Anthropic",
    google: "Google",
    openai: "OpenAI",
    openrouter: "OpenRouter",
    moonshot: "Moonshot AI",
    github: "GitHub",
    ollama: "Ollama (local)",
    pi: "Pi",
    custom: "a custom endpoint",
};

const hasBadge = (harness: string, vendor: string | undefined): boolean =>
    !!vendor && vendor.toLowerCase() !== (harness ?? "").toLowerCase();

/** What the logo says in words: the harness, and what it runs on when the badge shows. */
export function dualLogoTitle(harness: string, vendor?: string, vendorLabel?: string): string {
    const h = HARNESS_NAMES[(harness ?? "").toLowerCase()] ?? harness;
    if (!hasBadge(harness, vendor)) return `${h} harness`;
    return `${h} harness running on ${vendorLabel ?? VENDOR_NAMES[vendor!.toLowerCase()] ?? vendor}`;
}

export const DualProviderLogo = (props: DualProviderLogoProps): JSX.Element => {
    const size = () => props.size ?? 24;
    // A list row's 40 px logo carries a ringed badge; a header's 14-16 px one a smaller, thinly ringed one.
    const small = () => size() < 20;
    const vendorSize = () => (small() ? Math.round(size() * 0.6) : Math.max(10, Math.round(size() * 0.45)));

    const showBadge = () => hasBadge(props.harness, props.vendor);
    const tooltip = () => dualLogoTitle(props.harness, props.vendor, props.vendorLabel);

    return (
        <span
            class={`dual-provider-logo${props.class ? ` ${props.class}` : ""}`}
            style={{
                position: "relative",
                display: "inline-flex",
                "align-items": "center",
                "justify-content": "center",
                width: `${size()}px`,
                height: `${size()}px`,
            }}
            {...(props.bare ? {} : tip(tooltip()))}
            aria-label={props.bare ? undefined : tooltip()}
        >
            <ProviderLogo provider={props.harness} size={size()} />
            {showBadge() && (
                <span
                    class="dual-provider-logo-badge"
                    style={{
                        position: "absolute",
                        bottom: "-2px",
                        right: "-2px",
                        display: "inline-flex",
                        "align-items": "center",
                        "justify-content": "center",
                        "background-color": "var(--main-bg-color)",
                        "border-radius": "50%",
                        padding: small() ? "0" : "1px",
                        border: small() ? "none" : "1px solid var(--border-color)",
                        "box-shadow": small() ? "0 0 0 1px var(--main-bg-color)" : "0 1px 3px rgba(0, 0, 0, 0.3)",
                    }}
                >
                    <ProviderLogo provider={props.vendor!} size={vendorSize()} />
                </span>
            )}
        </span>
    );
};

DualProviderLogo.displayName = "DualProviderLogo";
