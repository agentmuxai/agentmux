// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The reactive side of the translations: which language is in effect, its dictionary once loaded, and `t`.
// Everything the store touches from outside (the setting, the system's languages, how a dictionary is
// fetched) comes in through `I18nEnv`, so the tests drive it with plain functions.
import { createEffect, createMemo, createSignal } from "solid-js";
import { negotiateLocale, requestedLocales, SOURCE_LOCALE } from "./locale";
import { translate, type Dict, type Layer, type Params } from "./translate";

export interface I18nEnv {
    /** The `app:locale` setting, read where a change is tracked. */
    setting: () => unknown;
    /** The system's languages, most preferred first (`navigator.languages`). */
    systemLanguages: () => readonly string[];
    /** The English source dictionary, always bundled. */
    source: Dict;
    /** The tags that have a dictionary besides English. */
    supported: readonly string[];
    /** Fetches one language's dictionary; called once per language. */
    load: (tag: string) => Promise<Dict>;
    /** Called with a key that no dictionary has. */
    onMissing?: (key: string) => void;
}

export interface I18n {
    /** The text for a key in the effective language, falling back to English key by key. Reactive. */
    t: (key: string, params?: Params) => string;
    /** The language in effect. Reactive. */
    locale: () => string;
    /** False while the effective language's dictionary is loading: `t` answers in English until it is true. */
    ready: () => boolean;
}

/** Call inside a reactive root (the app's, or `createRoot` in a test). */
export function createI18n(env: I18nEnv): I18n {
    const [loaded, setLoaded] = createSignal<Readonly<Record<string, Dict>>>({});
    const locale = createMemo(() =>
        negotiateLocale(requestedLocales(env.setting(), env.systemLanguages()), [SOURCE_LOCALE, ...env.supported])
    );
    const requested = new Set<string>();

    createEffect(() => {
        const tag = locale();
        if (tag === SOURCE_LOCALE || requested.has(tag)) return;
        requested.add(tag);
        env.load(tag).then(
            (dict) => setLoaded((prev) => ({ ...prev, [tag]: dict })),
            // A dictionary that fails to load leaves the person on English, and the next visit tries again.
            () => requested.delete(tag)
        );
    });

    const layers = createMemo((): readonly Layer[] => {
        const tag = locale();
        const english: Layer = { locale: SOURCE_LOCALE, dict: env.source };
        const dict = loaded()[tag];
        return dict ? [{ locale: tag, dict }, english] : [english];
    });

    return {
        t: (key, params) => translate(layers(), key, params, env.onMissing),
        locale,
        ready: () => locale() === SOURCE_LOCALE || loaded()[locale()] !== undefined,
    };
}
