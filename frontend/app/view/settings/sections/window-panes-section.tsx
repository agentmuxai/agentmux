// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TextInput } from "@/app/element/ui";
import { Show, type JSX } from "solid-js";

import { devModeOn } from "@/app/store/dev-mode";
import { settingsAtom } from "@/app/store/global";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { set, SettingRow, ToggleControl } from "../settings-controls";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const WINDOW_SETTINGS = {
    showBlockIds: settingEntry({
        id: "window.show_block_ids",
        labelKey: "settings.window.showBlockIds.label",
        descriptionKey: "settings.window.showBlockIds.description",
        section: "window",
        devOnly: true,
        keywords: ["pane id", "block id", "debug pane", "blockheader:showblockids"],
    }),
    defaultNewBlock: settingEntry({
        id: "window.default_new_block",
        labelKey: "settings.window.defaultNewBlock.label",
        descriptionKey: "settings.window.defaultNewBlock.description",
        section: "window",
        keywords: ["new tab default", "new pane default", "default view", "app:defaultnewblock"],
    }),
    showPaneNumberOverlay: settingEntry({
        id: "window.pane_number_overlay",
        labelKey: "settings.window.showPaneNumberOverlay.label",
        descriptionKey: "settings.window.showPaneNumberOverlay.description",
        section: "window",
        keywords: ["pane jump", "pane numbers", "quick switch", "app:showoverlayblocknums"],
    }),
    mouseSideButtons: settingEntry({
        id: "window.mouse_side_buttons",
        labelKey: "settings.window.mouseSideButtons.label",
        descriptionKey: "settings.window.mouseSideButtons.description",
        section: "window",
        keywords: ["mouse", "side button", "back", "forward", "history", "mouse:sidebuttons"],
    }),
    showStatusBar: settingEntry({
        id: "window.show_status_bar",
        labelKey: "settings.window.showStatusBar.label",
        descriptionKey: "settings.window.showStatusBar.description",
        section: "window",
        keywords: ["hide status bar", "footer", "bottom bar", "statusbar", "window:showstatusbar"],
    }),
    confirmTabClose: settingEntry({
        id: "window.confirm_tab_close",
        labelKey: "settings.window.confirmTabClose.label",
        descriptionKey: "settings.window.confirmTabClose.description",
        section: "window",
        keywords: ["close tab prompt", "confirm close", "tab:confirmclose"],
    }),
    confirmCloseWorking: settingEntry({
        id: "window.confirm_close_working",
        labelKey: "settings.window.confirmCloseWorking.label",
        descriptionKey: "settings.window.confirmCloseWorking.description",
        section: "window",
        keywords: ["close window prompt", "running agents", "quit warning", "window:confirmcloseworking"],
    }),
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
