// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The app's translations: `t("area.thing.label")` in a component is reactive, so a change of language
// re-renders it. English is bundled; every other language is its own file in `locales/` and is fetched when
// first needed, so a person loads one language and no other.
import { createRoot } from "solid-js";
import { isDev } from "@/app/store/misc-utils";
import { getSettingsKeyAtom } from "@/store/global";
import en from "./locales/en.json";
import { createI18n } from "./i18n-store";
import { PSEUDO_LOCALE } from "./locale";
import { pseudoDict } from "./pseudo";
import type { Dict } from "./translate";

/** The setting that picks the app's language: a BCP 47 tag, or "system". */
export const APP_LOCALE_KEY = "app:locale";

// Every language but English, as a lazy chunk named by its tag (`locales/ja.json` is `ja`).
const files = import.meta.glob<{ default: Dict }>(["./locales/*.json", "!./locales/en.json"]);

function tagOf(path: string): string {
    return path.replace(/^.*\//, "").replace(/\.json$/, "");
}

const loaders = new Map(Object.entries(files).map(([path, load]) => [tagOf(path), load]));

/** The languages that can be picked besides English: the files present, and the pseudo-locale in a dev build. */
export const SUPPORTED_LOCALES: readonly string[] = [...loaders.keys(), ...(isDev() ? [PSEUDO_LOCALE] : [])];

const i18n = createRoot(() =>
    createI18n({
        setting: getSettingsKeyAtom(APP_LOCALE_KEY),
        systemLanguages: () => (typeof navigator === "undefined" ? [] : navigator.languages),
        source: en,
        supported: SUPPORTED_LOCALES,
        load: async (tag) => {
            if (tag === PSEUDO_LOCALE) return pseudoDict(en);
            const load = loaders.get(tag);
            if (!load) throw new Error(`no dictionary for ${tag}`);
            return (await load()).default;
        },
        onMissing: (key) => {
            if (isDev()) console.warn(`[i18n] missing key: ${key}`);
        },
    })
);

export const t = i18n.t;

/** The language in effect, for `<html lang>` and for the formatters. Reactive. */
export const appLocale = i18n.locale;
