import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
const state = vi.hoisted(() => ({ settings: {} as Record<string, unknown>, dev: false, shipped: [] as string[] }));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: (...args: unknown[]) => setConfig(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => state.settings }));
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => state.dev }));
vi.mock("@/app/i18n/i18n", () => ({
    get SHIPPED_LOCALES() {
        return state.shipped;
    },
    t: (key: string) => ({ "settings.language.beta": "Beta", "settings.language.system": "Same as the system", "settings.language.label": "Language" })[key] ?? key,
}));

import { languageChoices, languageLabel, LanguageRow } from "./language-row";

describe("languageChoices and languageLabel", () => {
    it("lists the system, English, the shipped languages, then the pseudo-locale in dev mode only", () => {
        expect(languageChoices(false, [])).toEqual(["system", "en"]);
        expect(languageChoices(false, ["ja", "pt-BR"])).toEqual(["system", "en", "ja", "pt-BR"]);
        expect(languageChoices(true, ["ja"])).toEqual(["system", "en", "ja", "en-XA"]);
    });

    it("names each language in itself and marks a shipped one Beta", () => {
        expect(languageLabel("system", "Beta", "Same as the system")).toBe("Same as the system");
        expect(languageLabel("en", "Beta", "x")).toBe("English");
        expect(languageLabel("ja", "Beta", "x")).toBe("日本語 (Beta)");
        expect(languageLabel("en-XA", "Beta", "x")).toBe("Pseudo (developers)");
    });
});

describe("the Language row", () => {
    beforeEach(() => {
        setConfig.mockReset();
        state.settings = {};
        state.dev = false;
        state.shipped = [];
    });
    afterEach(() => cleanup());

    it("is not shown while no language ships and dev mode is off", () => {
        render(() => <LanguageRow />);
        expect(screen.queryByText("Language")).toBeNull();
    });

    it("is shown in dev mode, with the pseudo-locale to pick", () => {
        state.dev = true;
        render(() => <LanguageRow />);
        expect(screen.getByText("Language")).toBeTruthy();
        expect(screen.getByRole("option", { name: "Pseudo (developers)" })).toBeTruthy();
    });

    it("is shown once a language ships, and writes app:locale when one is picked", () => {
        state.shipped = ["ja"];
        render(() => <LanguageRow />);
        const select = screen.getByRole("combobox") as HTMLSelectElement;
        expect(select.value).toBe("system");
        expect(screen.getByRole("option", { name: "日本語 (Beta)" })).toBeTruthy();
        fireEvent.change(select, { target: { value: "ja" } });
        expect(setConfig).toHaveBeenCalledWith({}, { "app:locale": "ja" });
    });

    it("reads a stored value the picker does not list as the system's", () => {
        state.shipped = ["ja"];
        state.settings = { "app:locale": "en-XA" };
        render(() => <LanguageRow />);
        expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe("system");
    });

    it("shows the setting's value as chosen", () => {
        state.shipped = ["ja"];
        state.settings = { "app:locale": "ja" };
        render(() => <LanguageRow />);
        expect((screen.getByRole("combobox") as HTMLSelectElement).value).toBe("ja");
    });
});
