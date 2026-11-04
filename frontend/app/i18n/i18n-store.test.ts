import { createRoot, createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createI18n, type I18nEnv } from "./i18n-store";
import type { Dict } from "./translate";

const source: Dict = { hello: "Hello", "left.one": "{{count}} left", "left.other": "{{count}} left" };
const ja: Dict = { hello: "こんにちは" };

function make(over: Partial<I18nEnv> = {}) {
    const load = vi.fn(async (tag: string): Promise<Dict> => {
        if (tag === "ja") return ja;
        throw new Error("no dictionary");
    });
    const env: I18nEnv = {
        setting: () => "system",
        systemLanguages: () => ["en-US"],
        source,
        supported: ["ja"],
        load,
        ...over,
    };
    return { load: over.load ?? load, ...createRoot(() => createI18n(env)) };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("createI18n", () => {
    it("answers in English, and loads nothing, when English is in effect", async () => {
        const { t, locale, ready, load } = make();
        await settle();
        expect(locale()).toBe("en");
        expect(t("hello")).toBe("Hello");
        expect(ready()).toBe(true);
        expect(load).not.toHaveBeenCalled();
    });

    it("follows the system languages, then switches to the dictionary once it has loaded", async () => {
        const { t, locale, ready, load } = make({ systemLanguages: () => ["ja-JP", "en"] });
        expect(locale()).toBe("ja");
        expect(ready()).toBe(false);
        expect(t("hello"), "English until the dictionary arrives").toBe("Hello");
        await settle();
        expect(ready()).toBe(true);
        expect(t("hello")).toBe("こんにちは");
        expect(t("left", { count: 3 }), "a key the dictionary lacks falls back to English").toBe("3 left");
        expect(load).toHaveBeenCalledTimes(1);
    });

    it("follows a change of the setting, and loads a language once", async () => {
        const [setting, setSetting] = createSignal<unknown>("en");
        const { t, locale, load } = make({ setting });
        expect(t("hello")).toBe("Hello");
        setSetting("ja");
        await settle();
        expect(locale()).toBe("ja");
        expect(t("hello")).toBe("こんにちは");
        setSetting("en");
        setSetting("ja");
        await settle();
        expect(load).toHaveBeenCalledTimes(1);
    });

    it("stays on English when a dictionary fails to load, and tries again on the next ask", async () => {
        const [setting, setSetting] = createSignal<unknown>("ja");
        const load = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(ja);
        const { t, ready } = make({ setting, load });
        await settle();
        expect(ready()).toBe(false);
        expect(t("hello")).toBe("Hello");
        setSetting("en");
        setSetting("ja");
        await settle();
        expect(t("hello")).toBe("こんにちは");
        expect(load).toHaveBeenCalledTimes(2);
    });

    it("keeps English for a language with no dictionary", () => {
        const { locale } = make({ setting: () => "ko" });
        expect(locale()).toBe("en");
    });
});
