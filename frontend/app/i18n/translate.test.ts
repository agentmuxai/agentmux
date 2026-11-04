import { describe, expect, it, vi } from "vitest";
import { interpolate, pluralCategory, translate, type Layer } from "./translate";

const en: Layer = {
    locale: "en",
    dict: {
        greet: "Hello, {{name}}",
        "files.one": "{{count}} file",
        "files.other": "{{count}} files",
        "only.english": "Only in English",
        both: "English text",
    },
};
const pl: Layer = {
    locale: "pl",
    dict: {
        "files.one": "{{count}} plik",
        "files.few": "{{count}} pliki",
        "files.many": "{{count}} plików",
        "files.other": "{{count}} pliku",
        both: "Polski tekst",
    },
};

describe("interpolate", () => {
    it("fills a name, with or without spaces in the braces", () => {
        expect(interpolate("a {{x}} b {{ y }}", { x: "1", y: "2" }, "en")).toBe("a 1 b 2");
    });
    it("leaves an unknown name as written", () => {
        expect(interpolate("hi {{who}}", { other: "x" }, "en")).toBe("hi {{who}}");
        expect(interpolate("hi {{who}}", undefined, "en")).toBe("hi {{who}}");
    });
    it("writes a number in the language's format", () => {
        expect(interpolate("{{n}}", { n: 1234567 }, "en")).toBe("1,234,567");
        expect(interpolate("{{n}}", { n: 1234567 }, "de")).toBe("1.234.567");
    });
    it("does not read a name from the prototype", () => {
        expect(interpolate("{{toString}}", { x: "1" }, "en")).toBe("{{toString}}");
    });
});

describe("translate", () => {
    it("falls back to the next layer key by key", () => {
        expect(translate([pl, en], "both")).toBe("Polski tekst");
        expect(translate([pl, en], "only.english")).toBe("Only in English");
    });

    it("returns the key, and says so, when no layer has it", () => {
        const missing = vi.fn();
        expect(translate([pl, en], "nope.key", undefined, missing)).toBe("nope.key");
        expect(missing).toHaveBeenCalledWith("nope.key");
    });

    it("picks the plural variant by the language's own rules", () => {
        expect(translate([en], "files", { count: 1 })).toBe("1 file");
        expect(translate([en], "files", { count: 2 })).toBe("2 files");
        expect(translate([pl, en], "files", { count: 1 })).toBe("1 plik");
        expect(translate([pl, en], "files", { count: 3 })).toBe("3 pliki");
        expect(translate([pl, en], "files", { count: 5 })).toBe("5 plików");
        expect(translate([pl, en], "files", { count: 1.5 })).toBe("1,5 pliku");
    });

    it("falls through to a plain key when a layer has no plural variants", () => {
        const plain: Layer = { locale: "en", dict: { files: "{{count}} files (no variants)" } };
        expect(translate([{ locale: "pl", dict: {} }, plain], "files", { count: 2 })).toBe("2 files (no variants)");
    });

    it("fills params in the text of the layer that answered", () => {
        expect(translate([en], "greet", { name: "Mei" })).toBe("Hello, Mei");
    });

    it("has a category for every count in a language with few forms", () => {
        expect(pluralCategory("ja", 5)).toBe("other");
        expect(pluralCategory("en", 1)).toBe("one");
    });
});
