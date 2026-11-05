// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which of the shipped languages to show a person: the `app:locale` setting, or the system's languages when
// the setting is "system". Matching is by language and script, never by region alone, so `pt-PT` reads the
// Brazilian dictionary and `zh-TW` does not read the Simplified one (it gets English until Traditional ships).

/** The tag the source dictionary is written in, and the language every missing key falls back to. */
export const SOURCE_LOCALE = "en";

/** The `app:locale` value that follows the system. */
export const SYSTEM_LOCALE = "system";

/** The pseudo-locale for layout checks: accented, longer text. Offered only in a dev build. */
export const PSEUDO_LOCALE = "en-XA";

interface Parts {
    language: string;
    script: string | undefined;
    region: string | undefined;
}

function parts(tag: string): Parts | null {
    try {
        const { language, script, region } = new Intl.Locale(tag).maximize();
        return { language, script, region };
    } catch {
        return null;
    }
}

function matchOne(want: string, supported: readonly string[]): string | null {
    const exact = supported.find((s) => s.toLowerCase() === want.toLowerCase());
    if (exact) return exact;
    const w = parts(want);
    if (!w) return null;
    const sameLanguage = supported.filter((s) => {
        const p = parts(s);
        return p !== null && p.language === w.language && p.script === w.script;
    });
    return sameLanguage.find((s) => parts(s)?.region === w.region) ?? sameLanguage[0] ?? null;
}

/**
 * The first of `requested` (in order of preference) that `supported` can show, or English. A tag that is not
 * a valid BCP 47 tag is skipped, never thrown on: it came from a settings file or the OS.
 */
export function negotiateLocale(requested: readonly string[], supported: readonly string[]): string {
    for (const want of requested) {
        const hit = matchOne(want, supported);
        if (hit) return hit;
    }
    return SOURCE_LOCALE;
}

/** What the `app:locale` setting asks for: its tag, or the system's languages for "system" or anything unusable. */
export function requestedLocales(setting: unknown, system: readonly string[]): readonly string[] {
    const tag = typeof setting === "string" ? setting.trim() : "";
    return tag && tag !== SYSTEM_LOCALE ? [tag] : system;
}

/**
 * The locale to format dates, times and numbers in: the tag the person chose, or undefined for "system" so the
 * browser's own regional format applies (an English-speaking person on a British system keeps "3 Oct"). The
 * dictionary language is a separate question, answered by negotiation against the shipped languages.
 */
export function formatLocaleOf(setting: unknown): string | undefined {
    const tag = typeof setting === "string" ? setting.trim() : "";
    if (!tag || tag === SYSTEM_LOCALE || tag === PSEUDO_LOCALE) return undefined;
    return parts(tag) ? tag : undefined;
}

/** A language's name in that language, with a capital where the language has capitals: `日本語`, `Português (Brasil)`. */
export function nativeName(tag: string): string {
    try {
        const name = new Intl.DisplayNames(tag, { type: "language" }).of(tag) ?? tag;
        return name.charAt(0).toLocaleUpperCase(tag) + name.slice(1);
    } catch {
        return tag;
    }
}
