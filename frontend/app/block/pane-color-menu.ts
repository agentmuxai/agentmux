// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS } from "@/app/store/global";

export interface PaneHueOption {
    label: string;
    hue: number;
}

export const PANE_HUE_OPTIONS: ReadonlyArray<PaneHueOption> = [
    { label: "Cobalt",  hue: 218 },
    { label: "Emerald", hue: 150 },
    { label: "Amber",   hue:  38 },
    { label: "Rose",    hue: 352 },
    { label: "Violet",  hue: 270 },
    { label: "Cyan",    hue: 188 },
    { label: "Coral",   hue:  14 },
    { label: "Mint",    hue: 163 },
];

/** Derive the muted header background from a hue (0–360). */
export function hueToHeaderBg(hue: number): string {
    return `hsl(${hue}, 28%, 16%)`;
}

/** Derive the vivid active-border color from a hue (0–360). */
export function hueToActiveBorder(hue: number): string {
    return `hsl(${hue}, 65%, 52%)`;
}

function setHue(blockId: string, hue: number | null): void {
    void RpcApi.SetMetaCommand(TabRpcClient, {
        oref: WOS.makeORef("block", blockId),
        meta: { "frame:hue": hue } as any,
    });
}

export function buildPaneColorSubmenu(
    blockData: Block | null,
    blockId: string,
): ContextMenuItem {
    const currentHue = (blockData?.meta?.["frame:hue"] as number | undefined) ?? null;

    const items: ContextMenuItem[] = [
        {
            label: "None",
            type: "radio",
            checked: currentHue === null,
            click: () => setHue(blockId, null),
        },
        { type: "separator" },
        ...PANE_HUE_OPTIONS.map(({ label, hue }) => ({
            label,
            type: "radio" as const,
            checked: currentHue === hue,
            click: () => setHue(blockId, hue),
        })),
    ];

    return {
        label: "Pane Color",
        type: "submenu",
        submenu: items,
    };
}
