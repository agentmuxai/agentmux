// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { setBlockMeta } from "@/app/store/block-meta";

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

/** Derive the vivid active-border color from a hue (0–360). */
export function hueToActiveBorder(hue: number): string {
    return `hsl(${hue}, 65%, 52%)`;
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
    void setBlockMeta(blockId, { "frame:hue": hue } as any);
    if (hue != null && agentId) {
        void RpcApi.SetAgentContentCommand(TabRpcClient, {
            agent_id: agentId,
            content_type: "ui:color",
            content: hueToAgentIdentityColor(hue),
        });
    }
}
