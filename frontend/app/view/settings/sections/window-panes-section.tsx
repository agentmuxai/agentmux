// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TextInput } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";

import { devModeOn } from "@/app/store/dev-mode";
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
        devOnly: true,
        keywords: ["pane id", "block id", "debug pane", "blockheader:showblockids"],
    },
    defaultNewBlock: {
        id: "window.default_new_block",
        label: "Default new block",
        description: "View type opened by default for new tabs/panes (e.g. term, agent, shortcuts)",
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
    mouseSideButtons: {
        id: "window.mouse_side_buttons",
        label: "Mouse side buttons select the previous / next pane",
        description: "Back and Forward re-select the pane you had selected before or after, in any tab. Ctrl + button is page back / forward over a browser pane",
        section: "window",
        keywords: ["mouse", "side button", "back", "forward", "history", "mouse:sidebuttons"],
    },
    showStatusBar: {
        id: "window.show_status_bar",
        label: "Show the status bar",
        description:
            "The bar along the bottom with the backend dot, system stats, token and history totals, and the version. Hiding it frees a row of space; it comes back by itself if the backend goes down, and \"Toggle Status Bar\" in the command palette flips it",
        section: "window",
        keywords: ["hide status bar", "footer", "bottom bar", "statusbar", "window:showstatusbar"],
    },
    confirmTabClose: {
        id: "window.confirm_tab_close",
        label: "Confirm before closing a tab",
        description: "Ask before closing a tab, instead of closing it at once",
        section: "window",
        keywords: ["close tab prompt", "confirm close", "tab:confirmclose"],
    },
    confirmCloseWorking: {
        id: "window.confirm_close_working",
        label: "Warn before closing a window with working agents",
        description:
            "Ask before closing a window while an agent in it is mid-turn. Closing stops its agents; the window stays in the workspace, to reopen from the version panel",
        section: "window",
        keywords: ["close window prompt", "running agents", "quit warning", "window:confirmcloseworking"],
    },
} satisfies Record<string, SettingsIndexEntry>;

// ── Section: Window & Panes ───────────────────────────────────────────────────

export function WindowPanesSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

    return (
        <div class="settings-section-body">
            <Show when={devModeOn()}>
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
            </Show>
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
                id={WINDOW_SETTINGS.mouseSideButtons.id}
                label={WINDOW_SETTINGS.mouseSideButtons.label}
                description={WINDOW_SETTINGS.mouseSideButtons.description}
                control={
                    <ToggleControl
                        checked={(s()["mouse:sidebuttons"] ?? true) as boolean}
                        onChange={(v) => set("mouse:sidebuttons", v)}
                    />
                }
            />
            <SettingRow
                id={WINDOW_SETTINGS.showStatusBar.id}
                label={WINDOW_SETTINGS.showStatusBar.label}
                description={WINDOW_SETTINGS.showStatusBar.description}
                control={
                    <ToggleControl
                        checked={s()["window:showstatusbar"] !== false}
                        onChange={(v) => set("window:showstatusbar", v)}
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
            <SettingRow
                id={WINDOW_SETTINGS.confirmCloseWorking.id}
                label={WINDOW_SETTINGS.confirmCloseWorking.label}
                description={WINDOW_SETTINGS.confirmCloseWorking.description}
                control={
                    <ToggleControl
                        checked={s()["window:confirmcloseworking"] !== false}
                        onChange={(v) => set("window:confirmcloseworking", v)}
                    />
                }
            />
        </div>
    );
}
