// The dictionaries on disk: English is the source of truth, every other file must stay in step with it.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { coverage, dictionaryProblems, usedCategories } from "./dictionary-check";
import glossary from "./glossary.json";
import type { Dict } from "./translate";

const dir = join(dirname(fileURLToPath(import.meta.url)), "locales");
const load = (file: string): Dict => JSON.parse(readFileSync(join(dir, file), "utf8"));
const english = load("en.json");

describe("the English dictionary", () => {
    it("is flat, with string values and dot-path keys", () => {
        for (const [key, text] of Object.entries(english)) {
            expect(typeof text, key).toBe("string");
            expect(key, key).toMatch(/^[a-z][A-Za-z0-9]*(\.[A-Za-z0-9-]+)+$/);
        }
    });

});

describe("the glossary", () => {
    it("lists the product and feature names that stay English", () => {
        expect(glossary.doNotTranslate).toEqual(expect.arrayContaining(["AgentMux", "Swarm", "Vault", "Kanban", "Jekt"]));
        expect(glossary.doNotTranslate.every((name) => name.trim() === name && name.length > 0)).toBe(true);
    });
});

describe("every other dictionary", () => {
    const others = readdirSync(dir).filter((f) => f.endsWith(".json") && f !== "en.json");
    it("are named by a BCP 47 tag", () => {
        for (const file of others) expect(() => new Intl.Locale(file.replace(/\.json$/, "")), file).not.toThrow();
    });
    it.each(others)("%s keeps to the English keys, placeholders and plural forms", (file) => {
        expect(dictionaryProblems(english, load(file), file.replace(/\.json$/, ""))).toEqual([]);
    });
});

describe("dictionaryProblems", () => {
    const source: Dict = { "a.title": "Open {{name}}", "a.count.one": "{{count}} item", "a.count.other": "{{count}} items" };

    it("accepts a sound translation, with the plural forms of its own language", () => {
        expect(dictionaryProblems(source, { "a.title": "{{name}}を開く", "a.count.other": "{{count}}件" }, "ja")).toEqual([]);
        expect(
            dictionaryProblems(source, { "a.count.one": "{{count}} plik", "a.count.few": "{{count}} pliki", "a.count.many": "{{count}} plików", "a.count.other": "{{count}} pliku" }, "pl")
        ).toEqual([]);
    });

    it("finds a key English does not have", () => {
        expect(dictionaryProblems(source, { "a.gone": "x" }, "ja")).toEqual(["a.gone: no such key in English"]);
    });

    it("finds a placeholder that was dropped, renamed or added", () => {
        expect(dictionaryProblems(source, { "a.title": "Open" }, "ja")).toHaveLength(1);
        expect(dictionaryProblems(source, { "a.title": "Open {{nom}}" }, "ja")).toHaveLength(1);
        expect(dictionaryProblems(source, { "a.title": "Open {{name}} {{extra}}" }, "ja")).toHaveLength(1);
    });

    it("finds a plural form the language does not use", () => {
        expect(dictionaryProblems(source, { "a.count.one": "{{count}}件" }, "ja")).toEqual(['a.count.one: ja has no "one" form']);
    });

    it("does not count a missing key as a problem", () => {
        expect(dictionaryProblems(source, {}, "ja")).toEqual([]);
    });
});

describe("coverage", () => {
    it("counts a plural group once", () => {
        const source: Dict = { a: "x", "b.one": "y", "b.other": "y" };
        expect(coverage(source, { "b.other": "z" })).toBe(0.5);
        expect(coverage(source, { a: "1", "b.one": "2" })).toBe(1);
        expect(coverage({}, {})).toBe(1);
    });
});

describe("usedCategories", () => {
    it("lists what a language uses", () => {
        expect([...usedCategories("ja")]).toEqual(["other"]);
        expect(usedCategories("en")).toEqual(new Set(["one", "other"]));
        expect(usedCategories("pl")).toEqual(new Set(["one", "few", "many", "other"]));
    });
});
