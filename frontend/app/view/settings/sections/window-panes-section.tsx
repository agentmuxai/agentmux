// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import type { SettingsIndexEntry } from "../settings-model";
import { set, SettingRow, ToggleControl } from "../settings-controls";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const WINDOW_SETTINGS = {
    showBlockIds: {
        id: "window.show_block_ids",
        label: "Show block IDs",
        description: "Show each pane's internal block ID in its header (debugging aid)",
        section: "window",
        keywords: ["pane id", "block id", "debug pane", "blockheader:showblockids"],
    },
    defaultNewBlock: {
        id: "window.default_new_block",
        label: "Default new block",
        description: "View type opened by default for new tabs/panes (e.g. term, agent)",
        section: "window",
        keywords: ["new tab default", "new pane default", "default view", "app:defaultnewblock"],
    },
    showPaneNumberOverlay: {
        id: "window.pane_number_overlay",
        label: "Show pane number overlay",
        description: "Show numbered overlays for quick pane-jump shortcuts",
        section: "window",
        keywords: ["pane jump", "pane numbers", "quick switch", "app:showoverlayblocknums"],
    },
    skipTabCloseConfirm: {
        id: "window.skip_tab_close_confirm",
        label: "Skip tab close confirmation",
        description: "Don't prompt for confirmation when closing a tab",
        section: "window",
        keywords: ["close tab prompt", "confirm close", "tab:skipcloseconfirm"],
    },
} satisfies Record<string, SettingsIndexEntry>;

// ── Section: Window & Panes ───────────────────────────────────────────────────

export function WindowPanesSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <SettingRow
                id={WINDOW_SETTINGS.showBlockIds.id}
                label={WINDOW_SETTINGS.showBlockIds.label}
                description={WINDOW_SETTINGS.showBlockIds.description}
                control={
                    <ToggleControl
                        checked={!!(s()["blockheader:showblockids"] as boolean)}
                        onChange={(v) => set("blockheader:showblockids", v)}
                    />
                }
            />
            <SettingRow
                id={WINDOW_SETTINGS.defaultNewBlock.id}
                label={WINDOW_SETTINGS.defaultNewBlock.label}
                description={WINDOW_SETTINGS.defaultNewBlock.description}
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
                id={WINDOW_SETTINGS.showPaneNumberOverlay.id}
                label={WINDOW_SETTINGS.showPaneNumberOverlay.label}
                description={WINDOW_SETTINGS.showPaneNumberOverlay.description}
                control={
                    <ToggleControl
                        checked={!!(s()["app:showoverlayblocknums"] as boolean)}
                        onChange={(v) => set("app:showoverlayblocknums", v)}
                    />
                }
            />
            <SettingRow
                id={WINDOW_SETTINGS.skipTabCloseConfirm.id}
                label={WINDOW_SETTINGS.skipTabCloseConfirm.label}
                description={WINDOW_SETTINGS.skipTabCloseConfirm.description}
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
