// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Looking a key up in a dictionary: `{{name}}` interpolation, plural variants chosen by the language's own
// plural rules, and a fallback from the person's language to the English source, key by key. Pure functions,
// no state; the reactive wrapper is in i18n-store.ts.

/** One language's strings: flat dot-path keys to text. Plural variants are `<key>.one`, `<key>.other`, ... */
export type Dict = Readonly<Record<string, string>>;

export type Params = Readonly<Record<string, string | number>>;

/** A dictionary and the language its text is in, which picks its plural rules and number format. */
export interface Layer {
    locale: string;
    dict: Dict;
}

/** The language's plural categories, e.g. `one` and `other` for English, `few` and `many` for Polish. */
export function pluralCategory(locale: string, count: number): Intl.LDMLPluralRule {
    return new Intl.PluralRules(locale).select(count);
}

const TOKEN = /\{\{\s*([\w.]+)\s*\}\}/g;

/** Fills `{{name}}`; a number is written in the language's number format; an unknown name is left as written. */
export function interpolate(template: string, params: Params | undefined, locale: string): string {
    if (!params) return template;
    const numbers = new Intl.NumberFormat(locale);
    return template.replace(TOKEN, (token, name: string) => {
        if (!Object.hasOwn(params, name)) return token;
        const value = params[name];
        return typeof value === "number" ? numbers.format(value) : value;
    });
}

function find(layer: Layer, key: string, count: number | undefined): string | undefined {
    if (count !== undefined) {
        const plural = layer.dict[`${key}.${pluralCategory(layer.locale, count)}`] ?? layer.dict[`${key}.other`];
        if (plural !== undefined) return plural;
    }
    return layer.dict[key];
}

/**
 * The text for `key` from the first layer that has it (the person's language first, English after), with
 * `params` filled in. When `params.count` is a number the plural variant for it is used. A key no layer has
 * is returned as written, so a gap shows as the key and never as blank text.
 */
export function translate(layers: readonly Layer[], key: string, params?: Params, onMissing?: (key: string) => void): string {
    const count = typeof params?.count === "number" ? params.count : undefined;
    for (const layer of layers) {
        const text = find(layer, key, count);
        if (text !== undefined) return interpolate(text, params, layer.locale);
    }
    onMissing?.(key);
    return key;
}
