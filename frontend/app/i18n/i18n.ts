// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The app's translations: `t("area.thing.label")` in a component is reactive, so a change of language
// re-renders it. English is bundled; every other language is its own file in `locales/` and is fetched when
// first needed, so a person loads one language and no other.
import { createEffect, createRoot, createSignal } from "solid-js";
import { isDev } from "@/app/store/misc-utils";
import en from "./locales/en.json";
import { createI18n } from "./i18n-store";
import { formatLocaleOf, PSEUDO_LOCALE } from "./locale";
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

/** The languages that ship besides English: the dictionary files present. */
export const SHIPPED_LOCALES: readonly string[] = [...loaders.keys()];

/** What `app:locale` can name: the shipped languages and the pseudo-locale (listed in the picker only in dev mode). */
export const SUPPORTED_LOCALES: readonly string[] = [...SHIPPED_LOCALES, PSEUDO_LOCALE];

// Where the setting and the dev-mode switch are read from: bound once at startup (app-init), so this module needs no store of its own.
const [reader, setReader] = createSignal<() => unknown>(() => undefined);
const [pseudoReader, setPseudoReader] = createSignal<() => boolean>(() => false);

/**
 * Point the translations at the `app:locale` setting, a reactive getter, and say when the pseudo-locale may be used
 * (dev mode: with it off, a stored `en-XA` shows English, so no one is stuck in it). Called once, when the app starts.
 */
export function bindAppLocaleSetting(read: () => unknown, pseudoAllowed: () => boolean = () => false): void {
    setReader(() => read);
    setPseudoReader(() => pseudoAllowed);
}

const setting = (): unknown => reader()();

const i18n = createRoot(() => {
    const store = createI18n({
        setting,
        systemLanguages: () => (typeof navigator === "undefined" ? [] : navigator.languages),
        source: en,
        supported: () => (pseudoReader()() ? SUPPORTED_LOCALES : SHIPPED_LOCALES),
        load: async (tag) => {
            if (tag === PSEUDO_LOCALE) return pseudoDict(en);
            const load = loaders.get(tag);
            if (!load) throw new Error(`no dictionary for ${tag}`);
            return (await load()).default;
        },
        onMissing: (key) => {
            if (isDev()) console.warn(`[i18n] missing key: ${key}`);
        },
    });
    // The page's language follows the app's: screen readers pick a voice by it, and `:lang()` picks fonts.
    createEffect(() => {
        if (typeof document !== "undefined") document.documentElement.lang = store.locale();
    });
    return store;
});

export const t = i18n.t;

/** The language of the app's own text in effect (a shipped language, or English). Reactive. */
export const appLocale = i18n.locale;

/** The locale for dates, times and numbers: the chosen tag, or undefined to let the browser's regional format apply. Reactive. */
export const formatLocale = (): string | undefined => formatLocaleOf(setting());
