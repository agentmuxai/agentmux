// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Brand icons sourced from @lobehub/icons-static-svg (MIT) and one
// vendor PNG. The SVGs use width="1em"/height="1em", so the wrapping
// span's font-size drives the rendered pixel size. Mono icons inherit
// fill="currentColor" — set `color` on the wrapper to flip them with
// the theme.

import claudeColorSvg from "@lobehub/icons-static-svg/icons/claude-color.svg?raw";
import anthropicSvg from "@lobehub/icons-static-svg/icons/anthropic.svg?raw";
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
import brainSvg from "@/app/asset/logo-brain.svg?raw";
import type { JSX } from "solid-js";

// Several of the raw SVGs above (notably logo-brain.svg, whose 16-stop
// gradient illustration renders AgentMux's brand mark) define internal
// `<linearGradient id="...">`s with plain, non-unique ids and reference
// them via `fill="url(#...)"`. `innerHTML`-mounting the SAME raw markup
// more than once on one page (e.g. the Armory Accounts gallery tile AND
// the connected-accounts row both showing the AgentMux icon at once)
// creates duplicate ids — the browser resolves every `url(#id)` reference
// to whichever element with that id appears FIRST in the document,
// regardless of which copy defined it. Since these are userSpace-style
// gradients positioned relative to their OWN originally-inlined copy's
// geometry, every instance after the first paints with a gradient
// positioned for a shape it doesn't belong to — in practice, invisible
// (live-confirmed via CDP: a second inlined copy's path resolved its
// gradient fill to the FIRST copy's element). Live symptom: the AgentMux
// icon renders blank in Armory Accounts once both the tile and the
// connected row are visible together (post-`SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT`,
// both are now always in the same continuous scroll, not hidden in a
// separate scroll region).
//
// Fix: give every mounted instance's internal ids a unique suffix before
// setting innerHTML, so each copy's gradients/references only ever
// resolve within itself. Scoped to actual defined ids only (collected
// from `id="..."` attributes first) so this can't accidentally rewrite
// unrelated `#` occurrences elsewhere in the markup (e.g. hex color
// literals like `fill="#ff0000"`, which never match `id="..."`/`url(#...)`/
// `href="#..."`).
let nextSvgInstanceId = 0;

function uniquifySvgIds(svg: string): string {
    const ids = new Set<string>();
    for (const m of svg.matchAll(/\bid="([\w-]+)"/g)) ids.add(m[1]);
    if (ids.size === 0) return svg;
    const suffix = `pl${nextSvgInstanceId++}`;
    let out = svg;
    for (const id of ids) {
        const uniq = `${id}-${suffix}`;
        // split/join instead of replaceAll — this codebase's TS lib target
        // predates ES2021, and these are literal-string searches (no regex
        // escaping needed).
        out = out.split(`id="${id}"`).join(`id="${uniq}"`);
        out = out.split(`url(#${id})`).join(`url(#${uniq})`);
        out = out.split(`href="#${id}"`).join(`href="#${uniq}"`);
    }
    return out;
}

export interface ProviderLogoProps {
    provider: string;
    size?: number;
    class?: string;
}

export const ProviderLogo = (props: ProviderLogoProps): JSX.Element => {
    const size = () => props.size ?? 24;

    const inner = (): { html?: string; png?: string; jsx?: JSX.Element } => {
        const p = (props.provider ?? "").toLowerCase();

        // Distinct icons on purpose: "claude" is the harness (Claude Code,
        // the CLI), "anthropic" is the model vendor serving responses. Using
        // the same icon for both would defeat the point of DualProviderLogo,
        // which exists specifically to show harness and vendor as separate
        // things.
        if (p === "claude") return { html: claudeColorSvg };
        if (p === "anthropic") return { html: anthropicSvg };
        if (p === "codex") return { html: codexColorSvg };
        if (p === "openai") return { html: openaiSvg };
        if (p === "gemini" || p === "antigravity" || p === "agy") return { html: geminiColorSvg };
        if (p === "github") return { html: githubSvg };
        if (p === "copilot" || p === "githubcopilot" || p === "github-copilot") return { html: copilotColorSvg };
        if (p === "kimi") return { html: kimiSvg };
        if (p === "moonshot") return { html: moonshotSvg };
        if (p === "openclaw") return { html: openclawColorSvg };
        if (p === "aws") return { html: awsSvg };
        if (p === "pi" || p === "plandex") return { png: plandexUrl };

        if (p === "muxcode" || p === "mux-code" || p === "mux_code") return { html: brainSvg };

        // AgentMux's own brand mark — the brain-alternate logo. `brainSvg`
        // (@/app/asset/logo-brain.svg) is byte-identical to the source
        // frontend/logos/agentmux-logo-brain-alternate.svg.
        if (p === "agentmux") return { html: brainSvg };

        // Handcrafted multi-color Slack mark (the package ships only a mono
        // variant; the brand is recognized by its four colors).
        if (p === "slack") {
            return {
                jsx: (
                    <svg width={size()} height={size()} viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M5.04 15.16a2.52 2.52 0 1 1-2.52-2.52h2.52v2.52z" fill="#E01E5A" />
                        <path d="M6.31 15.16a2.52 2.52 0 0 1 5.04 0v6.32a2.52 2.52 0 0 1-5.04 0v-6.32z" fill="#E01E5A" />
                        <path d="M8.83 5.04a2.52 2.52 0 1 1 2.52-2.52v2.52H8.83z" fill="#36C5F0" />
                        <path d="M8.83 6.31a2.52 2.52 0 0 1 0 5.04H2.52a2.52 2.52 0 0 1 0-5.04h6.31z" fill="#36C5F0" />
                        <path d="M18.96 8.83a2.52 2.52 0 1 1 2.52 2.52h-2.52V8.83z" fill="#2EB67D" />
                        <path d="M17.69 8.83a2.52 2.52 0 0 1-5.04 0V2.52a2.52 2.52 0 0 1 5.04 0v6.31z" fill="#2EB67D" />
                        <path d="M15.17 18.96a2.52 2.52 0 1 1-2.52 2.52v-2.52h2.52z" fill="#ECB22E" />
                        <path d="M15.17 17.69a2.52 2.52 0 0 1 0-5.04h6.31a2.52 2.52 0 0 1 0 5.04h-6.31z" fill="#ECB22E" />
                    </svg>
                ),
            };
        }

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
        // 1em-based sizing inside lobehub's SVGs. uniquifySvgIds prevents
        // gradient/id collisions when the same icon mounts more than once
        // on one page — see its own doc comment above.
        return (
            <span
                class={cls}
                style={{ "font-size": `${size()}px`, "line-height": 0, display: "inline-flex" }}
                aria-hidden="true"
                innerHTML={uniquifySvgIds(r.html)}
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
