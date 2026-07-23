// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { set, SettingRow, ToggleControl } from "../settings-controls";

// ── Section: Window & Panes ───────────────────────────────────────────────────

export function WindowPanesSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Show block IDs"
                description="Show each pane's internal block ID in its header (debugging aid)"
                control={
                    <ToggleControl
                        checked={!!(s()["blockheader:showblockids"] as boolean)}
                        onChange={(v) => set("blockheader:showblockids", v)}
                    />
                }
            />
            <SettingRow
                label="Default new block"
                description="View type opened by default for new tabs/panes (e.g. term, agent)"
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["app:defaultnewblock"] as string) ?? ""}
                        placeholder="term"
                        onBlur={(e) => set("app:defaultnewblock", e.currentTarget.value || null)}
                    />
                }
            />
            <SettingRow
                label="Show pane number overlay"
                description="Show numbered overlays for quick pane-jump shortcuts"
                control={
                    <ToggleControl
                        checked={!!(s()["app:showoverlayblocknums"] as boolean)}
                        onChange={(v) => set("app:showoverlayblocknums", v)}
                    />
                }
            />
            <SettingRow
                label="Skip tab close confirmation"
                description="Don't prompt for confirmation when closing a tab"
                control={
                    <ToggleControl
                        checked={!!(s()["tab:skipcloseconfirm"] as boolean)}
                        onChange={(v) => set("tab:skipcloseconfirm", v)}
                    />
                }
            />
        </div>
    );
}
