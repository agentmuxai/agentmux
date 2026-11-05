// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Language row of Settings › Appearance: the app's own screens, never what an agent replies in. It is
// shown once a language ships (and always in dev mode, for the pseudo-locale), so a release with English only
// has no picker with nothing to pick.
import { Select } from "@/app/element/ui";
import { settingsAtom } from "@/app/store/global";
import { devModeOn } from "@/app/store/dev-mode";
import { SHIPPED_LOCALES, t } from "@/app/i18n/i18n";
import { nativeName, PSEUDO_LOCALE, SOURCE_LOCALE, SYSTEM_LOCALE } from "@/app/i18n/locale";
import { Show, type JSX } from "solid-js";
import { set, SettingRow } from "../settings-controls";
import type { SettingsIndexEntry } from "../settings-model";

export const LANGUAGE_SETTINGS = {
    language: {
        id: "appearance.language",
        label: "Language",
        description:
            "The language of AgentMux's own screens. It does not change what agents reply in, and text from a hosted agent's terminal or tools stays in its own language. A language marked Beta shows English for what is not translated yet.",
        section: "appearance",
        // Until a language ships, only the pseudo-locale exists, and that is for developers.
        devOnly: SHIPPED_LOCALES.length === 0,
        keywords: ["language", "locale", "translation", "idioma", "langue", "sprache", "言語", "语言", "언어", "idioma", "app:locale"],
    },
} satisfies Record<string, SettingsIndexEntry>;

/** The values the picker offers, in order: the system, English, each shipped language, then the pseudo-locale in dev mode. */
export function languageChoices(dev: boolean, shipped: readonly string[] = SHIPPED_LOCALES): string[] {
    return [SYSTEM_LOCALE, SOURCE_LOCALE, ...shipped, ...(dev ? [PSEUDO_LOCALE] : [])];
}

/** The picker's text for a value. A shipped language is Beta until its coverage gate says otherwise. */
export function languageLabel(value: string, beta: string, systemLabel: string): string {
    if (value === SYSTEM_LOCALE) return systemLabel;
    if (value === PSEUDO_LOCALE) return "Pseudo (developers)";
    return value === SOURCE_LOCALE ? nativeName(value) : `${nativeName(value)} (${beta})`;
}

export function LanguageRow(): JSX.Element {
    const current = () => {
        const v = settingsAtom()?.["app:locale"];
        return typeof v === "string" && v ? v : SYSTEM_LOCALE;
    };
    const options = () =>
        languageChoices(devModeOn()).map((value) => ({
            value,
            label: languageLabel(value, t("settings.language.beta"), t("settings.language.system")),
        }));
    return (
        <Show when={SHIPPED_LOCALES.length > 0 || devModeOn()}>
            <SettingRow
                id={LANGUAGE_SETTINGS.language.id}
                label={LANGUAGE_SETTINGS.language.label}
                description={LANGUAGE_SETTINGS.language.description}
                control={
                    <Select class="setting-select" value={current()} onChange={(v) => set("app:locale", v)} options={options()} />
                }
            />
        </Show>
    );
}
