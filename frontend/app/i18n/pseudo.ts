// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pseudo-locale: the English dictionary with accents, about 30% more letters and brackets, so a layout
// that cannot take a longer or wider translation, and any text that did not come from the dictionary, show up
// before a real translation does.

const ACCENTED: Record<string, string> = {
    a: "á", b: "ƀ", c: "ç", d: "ð", e: "é", f: "ƒ", g: "ĝ", h: "ĥ", i: "í", j: "ĵ", k: "ķ", l: "ļ", m: "ɱ",
    n: "ñ", o: "ó", p: "þ", q: "ƣ", r: "ŕ", s: "š", t: "ţ", u: "ú", v: "ṽ", w: "ŵ", x: "ẋ", y: "ý", z: "ž",
    A: "Á", B: "Ɓ", C: "Ç", D: "Ð", E: "É", F: "Ƒ", G: "Ĝ", H: "Ĥ", I: "Í", J: "Ĵ", K: "Ķ", L: "Ļ", M: "Ṁ",
    N: "Ñ", O: "Ó", P: "Þ", Q: "Ƣ", R: "Ŕ", S: "Š", T: "Ţ", U: "Ú", V: "Ṽ", W: "Ŵ", X: "Ẋ", Y: "Ý", Z: "Ž",
};

const PLACEHOLDER = /(\{\{[^}]*\}\})/;

/** `Open agent` becomes `[Ópéñ ágéñţ ~~~~]`; `{{name}}` placeholders pass through untouched. */
export function pseudoLocalize(text: string): string {
    const letters = text.replace(/\{\{[^}]*\}\}/g, "").replace(/[^A-Za-z]/g, "").length;
    const body = text
        .split(PLACEHOLDER)
        .map((piece) => (PLACEHOLDER.test(piece) ? piece : [...piece].map((ch) => ACCENTED[ch] ?? ch).join("")))
        .join("");
    return `[${body}${" ~".repeat(Math.ceil(letters * 0.3 / 2))}]`;
}

/** The whole dictionary, pseudo-localised. */
export function pseudoDict(source: Readonly<Record<string, string>>): Record<string, string> {
    return Object.fromEntries(Object.entries(source).map(([key, text]) => [key, pseudoLocalize(text)]));
}
