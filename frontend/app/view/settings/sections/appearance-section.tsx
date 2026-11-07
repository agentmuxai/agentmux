// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TextInput } from "@/app/element/ui";
import type { JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader, set, SettingRow } from "../settings-controls";
import { MagnifyRows, PaneGapRow, ThemeRows, TransparencyRows } from "./appearance-section-parts";
import { LANGUAGE_SETTINGS } from "./language-row";
import { WidgetColorsSettings } from "./widget-colors";

// ── Search index — one entry per row below, named-key so re-ordering rows
// can't silently misalign an entry with the wrong row (see settings-model.ts's
// SettingsIndexEntry doc comment). label/description here are the single
// source of truth the JSX reads from — see each row's `label={...}` below. ──

export const APPEARANCE_SETTINGS = {
    theme: {
        id: "appearance.theme",
        label: "Theme",
        description: "Default UI color theme, for windows without one of their own (the ☰ menu sets one window's)",
        section: "appearance",
        keywords: ["dark mode", "light mode", "color scheme", "appearance mode", "window:theme"],
    },
    splash: {
        id: "appearance.splash",
        label: "Startup splash screen",
        description: "Show the AgentMux splash while the app starts. Applies at the next launch — the launcher reads this before the app itself is running.",
        section: "appearance",
        keywords: ["splash screen", "boot screen", "loading screen", "disable splash", "splash:disabled"],
    },
    transparency: {
        id: "appearance.transparency",
        label: "Window transparency",
        description: "Enable background transparency and blur",
        section: "appearance",
        keywords: ["transparent window", "glass effect", "translucent", "window:transparent"],
    },
    opacity: {
        id: "appearance.opacity",
        label: "Opacity",
        description: "Default window opacity (35–100%), for windows without one of their own (the ☰ menu sets one window's)",
        section: "appearance",
        keywords: ["window opacity", "transparency level", "glass opacity", "window:opacity"],
    },
    blur: {
        id: "appearance.blur",
        label: "Background blur",
        description: "Blur the content behind the window",
        section: "appearance",
        keywords: ["blur effect", "frosted glass", "backdrop blur", "window:blur"],
    },
    paneGap: {
        id: "appearance.pane_gap",
        label: "Pane gap size",
        description: "Pixels between tiled panes (0–20)",
        section: "appearance",
        keywords: ["pane spacing", "tile gap", "split gap", "pane margin", "window:tilegapsize"],
    },
    bgColor: {
        id: "appearance.bg_color",
        label: "Background color",
        description: "Custom background color override (hex). Leave blank to use the theme default.",
        section: "appearance",
        keywords: ["custom background", "hex color", "window color", "window:bgcolor"],
    },
    magnifiedOpacity: {
        id: "appearance.magnified_opacity",
        label: "Magnified opacity",
        description: "Background opacity of a pane while magnified (0–1)",
        section: "appearance",
        group: "Pane hover-magnify",
        keywords: ["zoom opacity", "focus mode opacity", "pane zoom", "window:magnifiedblockopacity"],
    },
    magnifiedSize: {
        id: "appearance.magnified_size",
        label: "Magnified size",
        description: "Scale factor applied to a pane while magnified",
        section: "appearance",
        group: "Pane hover-magnify",
        keywords: ["zoom scale", "pane zoom factor", "magnify scale", "window:magnifiedblocksize"],
    },
    magnifiedBlurPrimary: {
        id: "appearance.magnified_blur_primary",
        label: "Magnified blur (primary)",
        description: "Backdrop blur, in pixels, applied to the magnified pane itself",
        section: "appearance",
        group: "Pane hover-magnify",
        keywords: ["focused pane blur", "zoom blur", "window:magnifiedblockblurprimarypx"],
    },
    magnifiedBlurSecondary: {
        id: "appearance.magnified_blur_secondary",
        label: "Magnified blur (secondary)",
        description: "Backdrop blur, in pixels, applied to the other panes behind it",
        section: "appearance",
        group: "Pane hover-magnify",
        keywords: ["background pane blur", "other panes blur", "window:magnifiedblockblursecondarypx"],
    },
    widgetColors: {
        id: "appearance.widget_colors",
        label: "Widget colors",
        description: "The color of every pane of each widget type: its border, tabs and header. A pane's own Pane Color and an agent's color still win.",
        section: "appearance",
        group: "Widget colors",
        keywords: ["color", "colour", "widget", "pane", "tab", "border", "pane:colors"],
    },
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
                        placeholder="#1a1a1a"
                        onBlur={(e) => set("window:bgcolor", e.currentTarget.value || null)}
                    />
                }
            />
            <SectionHeader label="Pane hover-magnify" />
            <MagnifyRows s={s} rows={rows} />
            <SectionHeader label="Widget colors" />
            <WidgetColorsSettings
                id={rows.widgetColors.id}
                label={rows.widgetColors.label}
                description={rows.widgetColors.description}
            />
        </div>
    );
}
