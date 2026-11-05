// The app's own instance: English from the bundled file, and the dev-only pseudo-locale.
import { describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ setting: "system" as unknown, dev: true }));
vi.mock("@/app/store/misc-utils", () => ({ isDev: () => state.dev }));
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => state.setting }));

describe("the app's translations", () => {
    it("answers from the English dictionary and names its language", async () => {
        const { t, appLocale, APP_LOCALE_KEY } = await import("./i18n");
        expect(APP_LOCALE_KEY).toBe("app:locale");
        expect(appLocale()).toBe("en");
        expect(t("settings.language.label")).toBe("Language");
    });

    it("returns the key for one that no dictionary has", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const { t } = await import("./i18n");
        expect(t("no.such.key")).toBe("no.such.key");
        expect(warn).toHaveBeenCalledWith("[i18n] missing key: no.such.key");
        warn.mockRestore();
    });

    it("offers the pseudo-locale in a dev build", async () => {
        const { SUPPORTED_LOCALES } = await import("./i18n");
        expect(SUPPORTED_LOCALES).toContain("en-XA");
    });
});
