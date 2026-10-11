// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TextInput } from "@/app/element/ui";
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
    confirmTabClose: {
        id: "window.confirm_tab_close",
        label: "Confirm before closing a tab",
        description: "Ask before closing a tab, instead of closing it at once",
        section: "window",
        keywords: ["close tab prompt", "confirm close", "tab:confirmclose"],
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
                    <TextInput
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
                        checked={(s()["app:showoverlayblocknums"] as boolean | undefined) ?? true}
                        onChange={(v) => set("app:showoverlayblocknums", v)}
                    />
                }
            />
            <SettingRow
                id={WINDOW_SETTINGS.confirmTabClose.id}
                label={WINDOW_SETTINGS.confirmTabClose.label}
                description={WINDOW_SETTINGS.confirmTabClose.description}
                control={
                    <ToggleControl
                        checked={(s()["tab:confirmclose"] ?? false) as boolean}
                        onChange={(v) => set("tab:confirmclose", v)}
                    />
                }
            />
        </div>
    );
}
