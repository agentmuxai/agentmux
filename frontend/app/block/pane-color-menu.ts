// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS } from "@/app/store/global";

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

export function setHue(blockId: string, hue: number | null): void {
    void RpcApi.SetMetaCommand(TabRpcClient, {
        oref: WOS.makeORef("block", blockId),
        meta: { "frame:hue": hue } as any,
    });
}
