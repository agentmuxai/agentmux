// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a translated dictionary must keep from the English one, checked per file by dictionary.test.ts: no key
// English lacks (a renamed or deleted source key leaves an orphan), the same `{{placeholders}}` in every
// string, and only plural suffixes the language uses. A key a translation lacks is not a problem here: it falls
// back to English at runtime, and coverage is reported separately.
import { pluralCategory, type Dict } from "./translate";

const PLACEHOLDER = /\{\{\s*([\w.]+)\s*\}\}/g;

const CATEGORIES = ["zero", "one", "two", "few", "many", "other"];

function placeholders(text: string): string[] {
    return [...text.matchAll(PLACEHOLDER)].map((m) => m[1]).sort();
}

/** The key without its plural suffix, and the suffix, when it has one. */
function splitPlural(key: string): { base: string; category: string | null } {
    const dot = key.lastIndexOf(".");
    const tail = dot === -1 ? "" : key.slice(dot + 1);
    return CATEGORIES.includes(tail) ? { base: key.slice(0, dot), category: tail } : { base: key, category: null };
}

/** The plural categories `locale` uses, from the categories its rules give for a spread of numbers. */
export function usedCategories(locale: string): Set<string> {
    const used = new Set<string>();
    for (const n of [0, 1, 2, 3, 4, 5, 6, 10, 11, 12, 20, 21, 22, 100, 101, 1000, 1.5]) used.add(pluralCategory(locale, n));
    return used;
}

/** Everything wrong with `translation` against `source`, one sentence each; empty when it is sound. */
export function dictionaryProblems(source: Dict, translation: Dict, locale: string): string[] {
    const problems: string[] = [];
    const sourceBases = new Set(Object.keys(source).map((k) => splitPlural(k).base));
    const used = usedCategories(locale);
    for (const [key, text] of Object.entries(translation)) {
        const { base, category } = splitPlural(key);
        if (!sourceBases.has(base)) {
            problems.push(`${key}: no such key in English`);
            continue;
        }
        if (category !== null && !used.has(category)) problems.push(`${key}: ${locale} has no "${category}" form`);
        // The English text this was written against: the same plural form if there is one, else the variant it has.
        const english = source[key] ?? source[`${base}.other`] ?? source[`${base}.one`] ?? source[base];
        if (english === undefined) continue;
        const want = placeholders(english).join(",");
        const got = placeholders(text).join(",");
        if (want !== got) problems.push(`${key}: placeholders {{${got.replace(/,/g, "}}, {{")}}} do not match the English {{${want.replace(/,/g, "}}, {{")}}}`);
    }
    return problems;
}

/** The share of English keys (plural forms counted as their base key) a translation has. 1 when English is empty. */
export function coverage(source: Dict, translation: Dict): number {
    const bases = (dict: Dict) => new Set(Object.keys(dict).map((k) => splitPlural(k).base));
    const want = bases(source);
    if (want.size === 0) return 1;
    const have = bases(translation);
    return [...want].filter((k) => have.has(k)).length / want.size;
}
