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

export interface DualProviderLogoProps {
    /** Harness/provider id (e.g. "claude", "antigravity", "codex", "openclaw"). */
    harness: string;
    /** Model vendor id (e.g. "anthropic", "google", "openai", "custom"). */
    vendor?: string;
    /** Base pixel size of the primary harness logo (default: 24). */
    size?: number;
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

export const DualProviderLogo = (props: DualProviderLogoProps): JSX.Element => {
    const size = () => props.size ?? 24;
    const vendorSize = () => Math.max(10, Math.round(size() * 0.45));

    const harnessName = () => HARNESS_NAMES[(props.harness ?? "").toLowerCase()] ?? props.harness;
    const vendorName = () => (props.vendor ? VENDOR_NAMES[props.vendor.toLowerCase()] ?? props.vendor : "");

    const showBadge = () =>
        !!props.vendor && props.vendor.toLowerCase() !== (props.harness ?? "").toLowerCase();

    const tooltip = () => {
        const h = harnessName();
        return showBadge() ? `${h} harness running on ${vendorName()}` : `${h} harness`;
    };

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
            title={tooltip()}
            aria-label={tooltip()}
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
                        padding: "1px",
                        border: "1px solid var(--border-color)",
                        "box-shadow": "0 1px 3px rgba(0, 0, 0, 0.3)",
                    }}
                >
                    <ProviderLogo provider={props.vendor!} size={vendorSize()} />
                </span>
            )}
        </span>
    );
};

DualProviderLogo.displayName = "DualProviderLogo";
