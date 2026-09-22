// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { MOS } from "@/app/store/global";

export interface PaneHueOption {
    label: string;
    hue: number;
}

// 12 hues at 30° intervals — full spectrum, evenly spaced
export const PANE_HUE_OPTIONS: ReadonlyArray<PaneHueOption> = [
    { label: "Crimson",    hue:   0 },
    { label: "Coral",      hue:  30 },
    { label: "Amber",      hue:  60 },
    { label: "Chartreuse", hue:  90 },
    { label: "Green",      hue: 120 },
    { label: "Emerald",    hue: 150 },
    { label: "Teal",       hue: 180 },
    { label: "Sky",        hue: 210 },
    { label: "Blue",       hue: 240 },
    { label: "Violet",     hue: 270 },
    { label: "Fuchsia",    hue: 300 },
    { label: "Pink",       hue: 330 },
];

/** Derive the muted header background from a hue (0–360). */
export function hueToHeaderBg(hue: number): string {
    return `hsl(${hue}, 28%, 16%)`;
}

/** Derive a pane-tab pill's own darkened background from a hue (0–360).
 * Deliberately more saturated/lighter than hueToHeaderBg's 28%/16% — that
 * treatment was tuned for a large, full-width header bar, where even a
 * subtle tint reads clearly. On a small ~20px pane-tab pill the identical
 * value is nearly indistinguishable from black and from a neighboring
 * uncolored (fully transparent) pill, which live-reproduced as "every
 * pill's color collapses to the same one" even though the underlying
 * computed colors were, in fact, all distinct
 * (ANALYSIS_PANE_TAB_COLOR_COLLAPSE_2026_09_21.md) — a contrast bug, not a
 * data bug. */
export function hueToPaneTabBg(hue: number): string {
    return `hsl(${hue}, 42%, 24%)`;
}

/** Derive the vivid active-border color from a hue (0–360). */
export function hueToActiveBorder(hue: number): string {
    return `hsl(${hue}, 65%, 52%)`;
}

/** Derive the dimmed unfocused-border color from a hue (0–360). Lightness
 * scaled by the same 0.55 dim factor as agent-color.ts::dimAgentColor, so
 * an explicit hue pick dims the same way the auto-assigned agent color
 * does. */
export function hueToBorder(hue: number): string {
    return `hsl(${hue}, 65%, 29%)`;
}

/** HSL -> `#rrggbb`. Standard conversion (Illuminae/W3C formula) — needed
 * because the agent identity color (`ui:color`, agent-color.ts) is a
 * strict hex string, while the pane-color picker works in hue-space. Used
 * to translate an explicit hue pick into that format when persisting it
 * to an agent's identity (see `setHue` below). */
function hslToHex(h: number, s: number, l: number): string {
    const sFrac = s / 100;
    const lFrac = l / 100;
    const k = (n: number) => (n + h / 30) % 12;
    const a = sFrac * Math.min(lFrac, 1 - lFrac);
    const f = (n: number) => lFrac - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
    const toHex = (x: number) => Math.round(255 * x).toString(16).padStart(2, "0");
    return `#${toHex(f(0))}${toHex(f(8))}${toHex(f(4))}`;
}

/** The hex an explicit hue pick persists as an agent's `ui:color` — same
 * hue/saturation/lightness as `hueToActiveBorder`, so the color an agent
 * keeps going forward is the one the user actually saw and chose. */
export function hueToAgentIdentityColor(hue: number): string {
    return hslToHex(hue, 65, 52);
}

/** `#rrggbb` -> hue (0–360). Inverse of `hslToHex`'s hue axis — needed so
 * a color that started life as a hex (an agent's persisted identity color)
 * can go through the same hue-based header treatment
 * (`headerBgForEffectiveColor`) as one that started life as an explicit hue
 * pick. Grayscale input (delta === 0, no defined hue) returns 0 — never hit
 * in practice since every agent identity color comes from
 * agent-color.ts's own vivid, non-gray palette. */
function hexToHue(hex: string): number {
    const r = parseInt(hex.slice(1, 3), 16) / 255;
    const g = parseInt(hex.slice(3, 5), 16) / 255;
    const b = parseInt(hex.slice(5, 7), 16) / 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const delta = max - min;
    if (delta === 0) return 0;
    let hue: number;
    if (max === r) hue = ((g - b) / delta) % 6;
    else if (max === g) hue = (b - r) / delta + 2;
    else hue = (r - g) / delta + 4;
    hue *= 60;
    return hue < 0 ? hue + 360 : hue;
}

/**
 * One header-background rule for both color sources a pane can have —
 * this IS the "single system": an explicit "Pane Color" hue pick
 * (`frame:hue`) and an agent's passive persisted identity color
 * (`frame:activebordercolor`, a hex) now produce the header treatment the
 * SAME way, by both going through `hueToHeaderBg`. Before this, an explicit
 * hue got the darkened/muted header (`hueToHeaderBg`) while a plain agent
 * identity color was applied to the header at full, vivid strength — the
 * same value the border used — so only explicitly-colored panes got the
 * dark-header/vivid-border look; every other agent pane's header matched
 * its border exactly. `computeFocusRingBorderColor` is untouched: the
 * border side of "one system" was already correct (full-strength color from
 * either source) — only the header side needed unifying.
 *
 * `isLightTheme` (user request 2026-09-21): the darkened/muted treatment
 * above is a dark-theme look — a near-black, low-lightness header reads as
 * muddy/broken against a light UI. On a light theme this instead keeps the
 * pre-2026-09-21 "bright" behavior: the header matches the border's
 * full-strength color exactly, the same as `computeFocusRingBorderColor`'s
 * `hueToActiveBorder`/raw hex.
 */
export function headerBgForEffectiveColor(
    hue: number | undefined,
    activeBorderHex: string | undefined,
    isLightTheme: boolean,
    // Which dark-theme muted-background deriver to use — defaults to the
    // header's own hueToHeaderBg. paneTabBgForEffectiveColor (below) reuses
    // this exact same light/dark precedence, swapping in hueToPaneTabBg's
    // higher-contrast treatment instead, rather than duplicating the
    // branching logic itself.
    darkBgFromHue: (hue: number) => string = hueToHeaderBg,
): string | undefined {
    if (isLightTheme) {
        if (typeof hue === "number") return hueToActiveBorder(hue);
        return activeBorderHex;
    }
    if (typeof hue === "number") return darkBgFromHue(hue);
    if (activeBorderHex) return darkBgFromHue(hexToHue(activeBorderHex));
    return undefined;
}

/** Same rule as headerBgForEffectiveColor, for a pane-tab pill's own
 * background instead of the pane header's — see hueToPaneTabBg's own doc
 * comment for why the dark-theme treatment needs to be more visible than
 * the header's at that much smaller element size. */
export function paneTabBgForEffectiveColor(
    hue: number | undefined,
    activeBorderHex: string | undefined,
    isLightTheme: boolean,
): string | undefined {
    return headerBgForEffectiveColor(hue, activeBorderHex, isLightTheme, hueToPaneTabBg);
}

/**
 * Set (or clear, `hue: null`) this pane's explicit "Pane Color" pick.
 *
 * `agentId`, when provided, additionally persists the pick as that agent's
 * identity color (`ui:color`, SPEC_AGENT_COLOR_2026_08_08.md) — the
 * "persist explicit picks to agent identity" half of
 * SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md's deferred
 * consolidation. This changes the DEFAULT color future panes for that
 * agent are born with; it does not repaint any of the agent's other
 * already-open panes, since their `frame:activebordercolor`/
 * `frame:bordercolor` are a one-time snapshot taken at their own launch,
 * not a live reference to `ui:color`. Only fires on an actual pick, not on
 * "Default" (`hue: null`) — clearing this one pane's color should not also
 * reset the agent's identity color everywhere else.
 */
export function setHue(blockId: string, hue: number | null, agentId?: string): void {
    void RpcApi.SetMetaCommand(TabRpcClient, {
        oref: MOS.makeORef("block", blockId),
        meta: { "frame:hue": hue } as any,
    });
    if (hue != null && agentId) {
        void RpcApi.SetAgentContentCommand(TabRpcClient, {
            agent_id: agentId,
            content_type: "ui:color",
            content: hueToAgentIdentityColor(hue),
        });
    }
}
