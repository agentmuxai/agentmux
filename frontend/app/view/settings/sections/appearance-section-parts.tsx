// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Appearance settings, the views: the rows of appearance-section.tsx in groups, each reading its value from the settings.

import { Select } from "@/app/element/ui";
import { For, Show, type JSX } from "solid-js";

import { THEME_OPTIONS } from "@/app/menu/base-menus";
import { hostHas } from "@/util/host-caps";
import { NumberControl, SettingRow, SliderControl, ToggleControl, set } from "../settings-controls";
import type { APPEARANCE_SETTINGS as AppearanceSettings } from "./appearance-section";
import { LanguageRow } from "./language-row";

type Rows = { s: () => any; rows: typeof AppearanceSettings };

/** The theme, the language and the startup splash. */
export function ThemeRows(props: Rows): JSX.Element {
    const { s, rows } = props;
    return (
        <>
            <SettingRow
                id={rows.theme.id}
                label={rows.theme.label}
                description={rows.theme.description}
                control={
                    <Select
                        class="setting-select"
                        value={(s()["window:theme"] as string) ?? "default"}
                        onChange={(v) => set("window:theme", v)}
                    >
                        <For each={THEME_OPTIONS}>{(t) => <option value={t.id}>{t.label}</option>}</For>
                    </Select>
                }
            />
            <LanguageRow />
            <SettingRow
                id={rows.splash.id}
                label={rows.splash.label}
                description={rows.splash.description}
                control={
                    <ToggleControl
                        // Stored inverted (`splash:disabled`, false by default) because
                        // the launcher reads the raw file before any of this code runs;
                        // the label stays positive so the toggle reads the way the
                        // splash behaves.
                        checked={!(s()["splash:disabled"] as boolean)}
                        onChange={(v) => set("splash:disabled", !v)}
                    />
                }
            />
        </>
    );
}

/** Window transparency, and, while it is on, its opacity and blur; none of it on a host that cannot do it. */
export function TransparencyRows(props: Rows): JSX.Element {
    const { s, rows } = props;
    const transparent = () => !!(s()["window:transparent"] as boolean);
    return (
        <Show when={hostHas("windowTransparency")}>
            <SettingRow
                id={rows.transparency.id}
                label={rows.transparency.label}
                description={rows.transparency.description}
                control={<ToggleControl checked={transparent()} onChange={(v) => set("window:transparent", v)} />}
            />
            <Show when={transparent()}>
                <SettingRow
                    id={rows.opacity.id}
                    indent
                    label={rows.opacity.label}
                    description={rows.opacity.description}
                    control={
                        <SliderControl
                            min={0.35}
                            max={1}
                            step={0.05}
                            value={(s()["window:opacity"] as number) ?? 0.8}
                            onChange={(v) => set("window:opacity", v)}
                        />
                    }
                />
                <SettingRow
                    id={rows.blur.id}
                    indent
                    label={rows.blur.label}
                    description={rows.blur.description}
                    control={
                        <ToggleControl
                            checked={!!(s()["window:blur"] as boolean)}
                            onChange={(v) => set("window:blur", v)}
                        />
                    }
                />
            </Show>
        </Show>
    );
}

/** The gap between tiled panes. */
export function PaneGapRow(props: Rows): JSX.Element {
    const { s, rows } = props;
    return (
        <SettingRow
            id={rows.paneGap.id}
            label={rows.paneGap.label}
            description={rows.paneGap.description}
            control={
                <NumberControl
                    class="setting-number"
                    min={0}
                    max={20}
                    step={1}
                    parse="int"
                    value={(s()["window:tilegapsize"] as number) ?? 3}
                    onChange={(v) => set("window:tilegapsize", v)}
                />
            }
        />
    );
}

/** The four numbers of a magnified pane (the section's header is the caller's). */
export function MagnifyRows(props: Rows): JSX.Element {
    const { s, rows } = props;
    return (
        <>
            <SettingRow
                id={rows.magnifiedOpacity.id}
                label={rows.magnifiedOpacity.label}
                description={rows.magnifiedOpacity.description}
                control={
                    <SliderControl
                        min={0}
                        max={1}
                        step={0.05}
                        value={(s()["window:magnifiedblockopacity"] as number) ?? 1}
                        onChange={(v) => set("window:magnifiedblockopacity", v)}
                    />
                }
            />
            <SettingRow
                id={rows.magnifiedSize.id}
                label={rows.magnifiedSize.label}
                description={rows.magnifiedSize.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={1}
                        step={0.1}
                        value={(s()["window:magnifiedblocksize"] as number) ?? 1.5}
                        onChange={(v) => set("window:magnifiedblocksize", v)}
                    />
                }
            />
            <SettingRow
                id={rows.magnifiedBlurPrimary.id}
                label={rows.magnifiedBlurPrimary.label}
                description={rows.magnifiedBlurPrimary.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0}
                        step={1}
                        parse="int"
                        value={(s()["window:magnifiedblockblurprimarypx"] as number) ?? 0}
                        onChange={(v) => set("window:magnifiedblockblurprimarypx", v)}
                    />
                }
            />
            <SettingRow
                id={rows.magnifiedBlurSecondary.id}
                label={rows.magnifiedBlurSecondary.label}
                description={rows.magnifiedBlurSecondary.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0}
                        step={1}
                        parse="int"
                        value={(s()["window:magnifiedblockblursecondarypx"] as number) ?? 0}
                        onChange={(v) => set("window:magnifiedblockblursecondarypx", v)}
                    />
                }
            />
        </>
    );
}
