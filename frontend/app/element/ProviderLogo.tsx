// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Brand icons sourced from @lobehub/icons-static-svg (MIT) and one
// vendor PNG. The SVGs use width="1em"/height="1em", so the wrapping
// span's font-size drives the rendered pixel size. Mono icons inherit
// fill="currentColor" — set `color` on the wrapper to flip them with
// the theme.

import claudeColorSvg from "@lobehub/icons-static-svg/icons/claude-color.svg?raw";
import codexColorSvg from "@lobehub/icons-static-svg/icons/codex-color.svg?raw";
import geminiColorSvg from "@lobehub/icons-static-svg/icons/gemini-color.svg?raw";
import copilotColorSvg from "@lobehub/icons-static-svg/icons/copilot-color.svg?raw";
import openclawColorSvg from "@lobehub/icons-static-svg/icons/openclaw-color.svg?raw";
import openaiSvg from "@lobehub/icons-static-svg/icons/openai.svg?raw";
import kimiSvg from "@lobehub/icons-static-svg/icons/kimi.svg?raw";
import moonshotSvg from "@lobehub/icons-static-svg/icons/moonshot.svg?raw";
import githubSvg from "@lobehub/icons-static-svg/icons/github.svg?raw";
import awsSvg from "@lobehub/icons-static-svg/icons/aws-color.svg?raw";
import plandexUrl from "@/app/element/icons/plandex.png?url";
import type { JSX } from "solid-js";

export interface ProviderLogoProps {
    provider: string;
    size?: number;
    class?: string;
}

export const ProviderLogo = (props: ProviderLogoProps): JSX.Element => {
    const size = () => props.size ?? 24;

    const inner = (): { html?: string; png?: string; jsx?: JSX.Element } => {
        const p = (props.provider ?? "").toLowerCase();

        if (p === "anthropic" || p === "claude") return { html: claudeColorSvg };
        if (p === "codex") return { html: codexColorSvg };
        if (p === "openai") return { html: openaiSvg };
        if (p === "gemini") return { html: geminiColorSvg };
        if (p === "github") return { html: githubSvg };
        if (p === "copilot" || p === "githubcopilot" || p === "github-copilot") return { html: copilotColorSvg };
        if (p === "kimi") return { html: kimiSvg };
        if (p === "moonshot") return { html: moonshotSvg };
        if (p === "openclaw") return { html: openclawColorSvg };
        if (p === "aws") return { html: awsSvg };
        if (p === "pi" || p === "plandex") return { png: plandexUrl };

        // Handcrafted multi-color Google G (more recognizable than
        // simple-icons' monochrome G).
        if (p === "google") {
            return {
                jsx: (
                    <svg width={size()} height={size()} viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" fill="#4285F4" />
                        <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853" />
                        <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05" />
                        <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335" />
                    </svg>
                ),
            };
        }

        // Default: first letter of provider name in a circle.
        const letter = (props.provider ?? "?").charAt(0).toUpperCase();
        return {
            jsx: (
                <svg width={size()} height={size()} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                    <text x="12" y="17" text-anchor="middle" font-size="14" font-family="system-ui, sans-serif" font-weight="600">
                        {letter}
                    </text>
                </svg>
            ),
        };
    };

    const cls = `provider-logo${props.class ? ` ${props.class}` : ""}`;
    const r = inner();

    if (r.png) {
        return <img src={r.png} alt={props.provider ?? ""} class={cls} width={size()} height={size()} />;
    }

    if (r.html) {
        // SolidJS innerHTML mounts the raw SVG. font-size drives the
        // 1em-based sizing inside lobehub's SVGs.
        return (
            <span
                class={cls}
                style={{ "font-size": `${size()}px`, "line-height": 0, display: "inline-flex" }}
                aria-hidden="true"
                innerHTML={r.html}
            />
        );
    }

    return (
        <span class={cls} aria-hidden="true">
            {r.jsx}
        </span>
    );
};

ProviderLogo.displayName = "ProviderLogo";
