import { describe, expect, it } from "vitest";
import { formatLocaleOf, negotiateLocale, nativeName, requestedLocales } from "./locale";

const SHIPPED = ["en", "ja", "zh-Hans", "pt-BR"];

describe("negotiateLocale", () => {
    it("takes an exact tag, whatever its case", () => {
        expect(negotiateLocale(["ja"], SHIPPED)).toBe("ja");
        expect(negotiateLocale(["PT-br"], SHIPPED)).toBe("pt-BR");
    });

    it("takes the same language and script from another region", () => {
        expect(negotiateLocale(["ja-JP"], SHIPPED)).toBe("ja");
        expect(negotiateLocale(["pt-PT"], SHIPPED)).toBe("pt-BR");
        expect(negotiateLocale(["pt"], SHIPPED)).toBe("pt-BR");
        expect(negotiateLocale(["en-GB"], SHIPPED)).toBe("en");
    });

    it("reads the Chinese script from the language and region", () => {
        expect(negotiateLocale(["zh-CN"], SHIPPED)).toBe("zh-Hans");
        expect(negotiateLocale(["zh"], SHIPPED)).toBe("zh-Hans");
        expect(negotiateLocale(["zh-SG"], SHIPPED)).toBe("zh-Hans");
    });

    it("never gives Traditional Chinese the Simplified dictionary", () => {
        expect(negotiateLocale(["zh-TW"], SHIPPED)).toBe("en");
        expect(negotiateLocale(["zh-Hant"], SHIPPED)).toBe("en");
        expect(negotiateLocale(["zh-HK", "ja"], SHIPPED)).toBe("ja");
    });

    it("walks the list in order and falls back to English", () => {
        expect(negotiateLocale(["ko", "pt-BR", "ja"], SHIPPED)).toBe("pt-BR");
        expect(negotiateLocale(["ko", "ru"], SHIPPED)).toBe("en");
        expect(negotiateLocale([], SHIPPED)).toBe("en");
    });

    it("skips a tag that is not a tag", () => {
        expect(negotiateLocale(["not a tag!!", "ja"], SHIPPED)).toBe("ja");
        expect(negotiateLocale([""], SHIPPED)).toBe("en");
    });

    it("prefers the same region when two dictionaries share a language and script", () => {
        expect(negotiateLocale(["es-ES"], ["en", "es-419", "es-ES"])).toBe("es-ES");
        expect(negotiateLocale(["es-MX"], ["en", "es-419", "es-ES"])).toBe("es-419");
    });
});

describe("requestedLocales", () => {
    const system = ["fr-FR", "en"];
    it("uses the system's languages for system and for anything unusable", () => {
        expect(requestedLocales("system", system)).toEqual(system);
        expect(requestedLocales(undefined, system)).toEqual(system);
        expect(requestedLocales("  ", system)).toEqual(system);
        expect(requestedLocales(42, system)).toEqual(system);
    });
    it("uses the chosen tag alone", () => {
        expect(requestedLocales("ja", system)).toEqual(["ja"]);
        expect(requestedLocales(" pt-BR ", system)).toEqual(["pt-BR"]);
    });
});

describe("formatLocaleOf", () => {
    it("is the chosen tag, and undefined for system, the pseudo-locale and anything unusable", () => {
        expect(formatLocaleOf("ja")).toBe("ja");
        expect(formatLocaleOf(" pt-BR ")).toBe("pt-BR");
        expect(formatLocaleOf("system")).toBeUndefined();
        expect(formatLocaleOf(undefined)).toBeUndefined();
        expect(formatLocaleOf("en-XA")).toBeUndefined();
        expect(formatLocaleOf("not a tag!!")).toBeUndefined();
    });
});

describe("nativeName", () => {
    it("names a language in itself", () => {
        expect(nativeName("ja")).toBe("日本語");
        expect(nativeName("de")).toBe("Deutsch");
        expect(nativeName("pt-BR")).toMatch(/^Português/);
        expect(nativeName("en")).toBe("English");
    });
    it("falls back to the tag for one it cannot name", () => {
        expect(nativeName("not a tag!!")).toBe("not a tag!!");
    });
});
