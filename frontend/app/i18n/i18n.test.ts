// @vitest-environment jsdom
// The app's own instance: English from the bundled file, the page's language, and the format locale.
import { createRoot, createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ dev: true }));
vi.mock("@/app/store/misc-utils", () => ({ isDev: () => state.dev }));

describe("the app's translations", () => {
    it("answers from the English dictionary and names its language", async () => {
        const { t, appLocale, APP_LOCALE_KEY } = await import("./i18n");
        expect(APP_LOCALE_KEY).toBe("app:locale");
        expect(appLocale()).toBe("en");
        expect(t("settings.language.label")).toBe("Language");
    });

    it("returns the key for one that no dictionary has, and says so in a dev build", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const { t } = await import("./i18n");
        expect(t("no.such.key")).toBe("no.such.key");
        expect(warn).toHaveBeenCalledWith("[i18n] missing key: no.such.key");
        warn.mockRestore();
    });

    it("lets app:locale name the pseudo-locale, and ships no language yet", async () => {
        const { SUPPORTED_LOCALES, SHIPPED_LOCALES } = await import("./i18n");
        expect(SUPPORTED_LOCALES).toContain("en-XA");
        expect(SHIPPED_LOCALES).toEqual([]);
    });

    it("sets the page's language", async () => {
        await import("./i18n");
        expect(document.documentElement.lang).toBe("en");
    });

    it("follows the setting it is bound to: the pseudo-locale's text, the page's language, the format locale", async () => {
        const { bindAppLocaleSetting, t, appLocale, formatLocale } = await import("./i18n");
        const [setting, setSetting] = createSignal<unknown>("system");
        bindAppLocaleSetting(setting);
        expect(formatLocale(), "system leaves the regional format to the browser").toBeUndefined();
        setSetting("ja");
        expect(formatLocale()).toBe("ja");
        expect(appLocale(), "no Japanese dictionary ships, so the app text is English").toBe("en");
        setSetting("en-XA");
        await new Promise((resolve) => setTimeout(resolve, 0));
        expect(appLocale()).toBe("en-XA");
        expect(t("settings.language.label")).toBe("[Ļáñĝúáĝé ~ ~]");
        expect(document.documentElement.lang).toBe("en-XA");
        setSetting("system");
    });
});
