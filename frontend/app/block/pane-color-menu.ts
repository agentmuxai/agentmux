// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { setBlockMeta } from "@/app/store/block-meta";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { slotById } from "@/app/theme/palette";
import { widgetColorKeys } from "./pane-identity";

/**
 * Set (or clear, `slotId: null`) this pane's explicit "Pane Color" pick, a
 * palette slot id (theme/palette.ts) stored as `frame:color`. The older HSL
 * pick (`frame:hue`) is cleared with it, so a pane never holds two.
 *
 * `agentId`, when provided, additionally persists the pick as that agent's
 * identity color (`ui:color`, SPEC_AGENT_COLOR_2026_08_08.md), the slot's
 * stored hex: the "persist explicit picks to agent identity" half of
 * SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md's deferred
 * consolidation. This changes the DEFAULT color future panes for that
 * agent are born with; it does not repaint any of the agent's other
 * already-open panes, since their `frame:activebordercolor` is a one-time
 * snapshot taken at their own launch, not a live reference to `ui:color`.
 * Only fires on an actual pick, not on "Default" (`slotId: null`): clearing
 * this one pane's color should not also reset the agent's identity color
 * everywhere else.
 */
export function setPaneColor(blockId: string, slotId: string | null, agentId?: string): void {
    const slot = slotId === null ? undefined : slotById(slotId);
    void setBlockMeta(blockId, { "frame:color": slot?.id ?? null, "frame:hue": null } as any);
    if (slot && agentId) {
        void RpcApi.SetAgentContentCommand(TabRpcClient, {
            agent_id: agentId,
            content_type: "ui:color",
            content: slot.hex,
        });
    }
}

/**
 * Set the color of every pane of a widget type, the `pane:colors` setting
 * (SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.4). `view` is the key from
 * `widgetColorView`. A slot id sets it, `null` gives the widget no color, and
 * `undefined` returns it to its built-in color. The last key removed deletes
 * the setting.
 */
export function setWidgetColor(view: string, slotId: string | null | undefined): void {
    const next: Record<string, string | number | null> = { ...getSettingsKeyAtom("pane:colors")() };
    // A color saved under the widget's former id goes too, or it would keep
    // applying after a reset.
    for (const former of widgetColorKeys(view).slice(1)) delete next[former];
    if (slotId === undefined) delete next[view];
    else next[view] = slotId === null ? null : (slotById(slotId)?.id ?? null);
    // A null value deletes the key on disk; SettingsType has no null.
    const data: Record<string, unknown> = { "pane:colors": Object.keys(next).length > 0 ? next : null };
    void RpcApi.SetConfigCommand(TabRpcClient, data as SettingsType);
}

/** "Use for all <Label> panes": this pane's pick becomes its widget's color,
 *  and the pane goes back to following it (spec §3.7). */
export function applyColorToAllPanes(blockId: string, view: string, slotId: string): void {
    setWidgetColor(view, slotId);
    setPaneColor(blockId, null);
}
