// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DualProviderLogo — renders a primary Agent Harness icon with an overlaid
 * secondary Model Vendor badge.
 *
 * Realizes the Harness vs. Model Vendor decoupling throughout the AgentMux UI:
 * - Main icon: Execution driver (e.g. Claude Code, AGY, Codex, OpenClaw, MuxCode).
 * - Overlay badge: Upstream intelligence vendor (e.g. Anthropic, Google Gemini, OpenAI, OpenRouter, Ollama).
 */

import { ProviderLogo } from "@/element/ProviderLogo";
import type { JSX } from "solid-js";

export interface DualProviderLogoProps {
    /** Harness engine or provider ID (e.g. "claude", "antigravity", "codex", "openclaw"). */
    harness: string;
    /** Model vendor ID (e.g. "anthropic", "google", "openai", "openrouter", "ollama"). */
    vendor?: string;
    /** Base pixel size of the primary harness logo (default: 24). */
    size?: number;
    /** Additional CSS class names. */
    class?: string;
}

export const DualProviderLogo = (props: DualProviderLogoProps): JSX.Element => {
    const size = () => props.size ?? 24;
    const vendorSize = () => Math.max(10, Math.round(size() * 0.45));

    const harnessName = () => {
        const h = (props.harness ?? "").toLowerCase();
        if (h === "antigravity" || h === "agy") return "Antigravity (AGY)";
        if (h === "claude") return "Claude Code";
        if (h === "codex") return "Codex CLI";
        if (h === "openclaw") return "OpenClaw";
        if (h === "muxcode") return "Mux Code";
        return props.harness;
    };

    const vendorName = () => {
        if (!props.vendor) return "";
        const v = props.vendor.toLowerCase();
        if (v === "google" || v === "gemini") return "Google Gemini";
        if (v === "anthropic") return "Anthropic";
        if (v === "openai") return "OpenAI";
        if (v === "openrouter") return "OpenRouter";
        if (v === "ollama") return "Ollama Local";
        return props.vendor;
    };

    const tooltip = () => {
        const h = harnessName();
        const v = vendorName();
        return v ? `${h} harness running on ${v}` : `${h} harness`;
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
            {/* Primary Harness Execution Engine Logo */}
            <ProviderLogo provider={props.harness} size={size()} />

            {/* Overlaid Secondary Model Vendor Badge */}
            {props.vendor && props.vendor.toLowerCase() !== props.harness.toLowerCase() && (
                <span
                    class="dual-provider-logo-badge"
                    style={{
                        position: "absolute",
                        bottom: "-2px",
                        right: "-2px",
                        display: "inline-flex",
                        "align-items": "center",
                        "justify-content": "center",
                        "background-color": "var(--bg-card, #1e1e24)",
                        "border-radius": "50%",
                        padding: "1px",
                        border: "1px solid var(--border-subtle, rgba(255, 255, 255, 0.15))",
                        "box-shadow": "0 1px 3px rgba(0, 0, 0, 0.3)",
                    }}
                >
                    <ProviderLogo provider={props.vendor} size={vendorSize()} />
                </span>
            )}
        </span>
    );
};

DualProviderLogo.displayName = "DualProviderLogo";
