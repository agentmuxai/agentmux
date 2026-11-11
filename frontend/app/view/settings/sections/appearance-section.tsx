// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TextInput } from "@/app/element/ui";
import type { JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { settingsAtom } from "@/app/store/global";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow } from "../settings-controls";
import { MagnifyRows, PaneGapRow, ThemeRows, TransparencyRows } from "./appearance-section-parts";
import { LANGUAGE_SETTINGS } from "./language-row";
import { WidgetColorsSettings } from "./widget-colors";

// ── Search index — one entry per row below, named-key so re-ordering rows
// can't silently misalign an entry with the wrong row (see settings-model.ts's
// SettingsIndexEntry doc comment). label/description here are the single
// source of truth the JSX reads from — see each row's `label={...}` below. ──

export const APPEARANCE_SETTINGS = {
    theme: settingEntry({
        id: "appearance.theme",
        labelKey: "settings.appearance.theme.label",
        descriptionKey: "settings.appearance.theme.description",
        section: "appearance",
        keywords: ["dark mode", "light mode", "color scheme", "appearance mode", "window:theme"],
    }),
    splash: settingEntry({
        id: "appearance.splash",
        labelKey: "settings.appearance.splash.label",
        descriptionKey: "settings.appearance.splash.description",
        section: "appearance",
        keywords: ["splash screen", "boot screen", "loading screen", "disable splash", "splash:disabled"],
    }),
    transparency: settingEntry({
        id: "appearance.transparency",
        labelKey: "settings.appearance.transparency.label",
        descriptionKey: "settings.appearance.transparency.description",
        section: "appearance",
        keywords: ["transparent window", "glass effect", "translucent", "window:transparent"],
    }),
    opacity: settingEntry({
        id: "appearance.opacity",
        labelKey: "settings.appearance.opacity.label",
        descriptionKey: "settings.appearance.opacity.description",
        section: "appearance",
        keywords: ["window opacity", "transparency level", "glass opacity", "window:opacity"],
    }),
    blur: settingEntry({
        id: "appearance.blur",
        labelKey: "settings.appearance.blur.label",
        descriptionKey: "settings.appearance.blur.description",
        section: "appearance",
        keywords: ["blur effect", "frosted glass", "backdrop blur", "window:blur"],
    }),
    paneGap: settingEntry({
        id: "appearance.pane_gap",
        labelKey: "settings.appearance.paneGap.label",
        descriptionKey: "settings.appearance.paneGap.description",
        section: "appearance",
        keywords: ["pane spacing", "tile gap", "split gap", "pane margin", "window:tilegapsize"],
    }),
    bgColor: settingEntry({
        id: "appearance.bg_color",
        labelKey: "settings.appearance.bgColor.label",
        descriptionKey: "settings.appearance.bgColor.description",
        section: "appearance",
        keywords: ["custom background", "hex color", "window color", "window:bgcolor"],
    }),
    magnifiedOpacity: settingEntry({
        id: "appearance.magnified_opacity",
        labelKey: "settings.appearance.magnifiedOpacity.label",
        descriptionKey: "settings.appearance.magnifiedOpacity.description",
        section: "appearance",
        groupKey: "settings.appearance.group.paneHoverMagnify",
        keywords: ["zoom opacity", "focus mode opacity", "pane zoom", "window:magnifiedblockopacity"],
    }),
    magnifiedSize: settingEntry({
        id: "appearance.magnified_size",
        labelKey: "settings.appearance.magnifiedSize.label",
        descriptionKey: "settings.appearance.magnifiedSize.description",
        section: "appearance",
        groupKey: "settings.appearance.group.paneHoverMagnify",
        keywords: ["zoom scale", "pane zoom factor", "magnify scale", "window:magnifiedblocksize"],
    }),
    magnifiedBlurPrimary: settingEntry({
        id: "appearance.magnified_blur_primary",
        labelKey: "settings.appearance.magnifiedBlurPrimary.label",
        descriptionKey: "settings.appearance.magnifiedBlurPrimary.description",
        section: "appearance",
        groupKey: "settings.appearance.group.paneHoverMagnify",
        keywords: ["focused pane blur", "zoom blur", "window:magnifiedblockblurprimarypx"],
    }),
    magnifiedBlurSecondary: settingEntry({
        id: "appearance.magnified_blur_secondary",
        labelKey: "settings.appearance.magnifiedBlurSecondary.label",
        descriptionKey: "settings.appearance.magnifiedBlurSecondary.description",
        section: "appearance",
        groupKey: "settings.appearance.group.paneHoverMagnify",
        keywords: ["background pane blur", "other panes blur", "window:magnifiedblockblursecondarypx"],
    }),
    widgetColors: settingEntry({
        id: "appearance.widget_colors",
        labelKey: "settings.appearance.widgetColors.label",
        descriptionKey: "settings.appearance.widgetColors.description",
        section: "appearance",
        groupKey: "settings.appearance.group.widgetColors",
        keywords: ["color", "colour", "widget", "pane", "tab", "border", "pane:colors"],
    }),
    ...LANGUAGE_SETTINGS,
} satisfies Record<string, SettingsIndexEntry>;

// ── Section: Appearance ───────────────────────────────────────────────────────

export function AppearanceSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const rows = APPEARANCE_SETTINGS;

    return (
        <div class="settings-section-body">
            <ThemeRows s={s} rows={rows} />
            <TransparencyRows s={s} rows={rows} />
            <PaneGapRow s={s} rows={rows} />
            <SettingRow
                id={rows.bgColor.id}
                label={rows.bgColor.label}
                description={rows.bgColor.description}
                control={
                    <TextInput
                        class="setting-text"
                        type="text"
                        value={(s()["window:bgcolor"] as string) ?? ""}
                        placeholder={"#1a1a1a"}
                        onBlur={(e) => set("window:bgcolor", e.currentTarget.value || null)}
                    />
                }
            />
            <SectionHeader label={t("settings.appearance.group.paneHoverMagnify")} />
            <MagnifyRows s={s} rows={rows} />
            <SectionHeader label={t("settings.appearance.group.widgetColors")} />
            <WidgetColorsSettings
                id={rows.widgetColors.id}
                label={rows.widgetColors.label}
                description={rows.widgetColors.description}
            />
        </div>
    );
}
