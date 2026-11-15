// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The hook that keeps a pane's backdrop registered as its scope, follows the app theme, and steps aside for high contrast. */

import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const theme = vi.hoisted(() => ({ read: (): string => "default" }));
vi.mock("../store/window-appearance", () => ({ currentWindowTheme: () => theme.read() }));
vi.mock("../block/pane-identity", () => ({ isLightThemeActive: () => theme.read() === "light" }));

import { getPaneScope } from "./scope";
import { usePaneBackdrop } from "./usePaneBackdrop";

const teal = { kind: "builtin", id: "teal", polarity: "follow", version: 1 };
const disposers: Array<() => void> = [];
const realMatchMedia = Object.getOwnPropertyDescriptor(window, "matchMedia");
afterEach(() => {
    while (disposers.length) disposers.pop()!();
    theme.read = () => "default";
    if (realMatchMedia) Object.defineProperty(window, "matchMedia", realMatchMedia);
    else delete (window as { matchMedia?: unknown }).matchMedia;
});

function mount(blockId: string, meta: () => { backdrop?: unknown; bg?: unknown } | undefined) {
    return createRoot((dispose) => {
        disposers.push(dispose);
        return usePaneBackdrop(blockId, meta);
    });
}

describe("usePaneBackdrop", () => {
    it("registers nothing for a pane with no backdrop", () => {
        const r = mount("hb-none", () => ({}));
        expect(r()).toBeUndefined();
        expect(getPaneScope("hb-none")()).toBeUndefined();
    });

    it("registers the pane's scope from its record, and releases it when the pane is gone", () => {
        const dispose = createRoot((d) => {
            usePaneBackdrop("hb-teal", () => ({ backdrop: teal }));
            return d;
        });
        expect(getPaneScope("hb-teal")()?.["--surface"]).toMatch(/^#[0-9a-f]{6}$/);
        expect(getPaneScope("hb-teal")()?.["--main-text-color"]).toBe("var(--ink-text)");
        dispose();
        expect(getPaneScope("hb-teal")()).toBeUndefined();
    });

    it("follows a change of the record, and Reset (the record gone)", () => {
        const [meta, setMeta] = createSignal<{ backdrop?: unknown }>({ backdrop: teal });
        const r = mount("hb-change", meta);
        const teal1 = getPaneScope("hb-change")()?.["--surface"];
        setMeta({ backdrop: { ...teal, id: "crimson" } });
        expect(r()?.id).toBe("builtin:crimson");
        expect(getPaneScope("hb-change")()?.["--surface"]).not.toBe(teal1);
        setMeta({});
        expect(r()).toBeUndefined();
        expect(getPaneScope("hb-change")()).toBeUndefined();
    });

    it("re-derives a built-in for the other side when the app theme switches", () => {
        const [themeId, setThemeId] = createSignal("default");
        theme.read = themeId;
        mount("hb-theme", () => ({ backdrop: teal }));
        const dark = getPaneScope("hb-theme")()?.["--surface"];
        expect(getPaneScope("hb-theme")()?.["color-scheme"]).toBe("dark");
        setThemeId("light");
        expect(getPaneScope("hb-theme")()?.["--surface"]).not.toBe(dark);
        expect(getPaneScope("hb-theme")()?.["color-scheme"]).toBe("light");
    });

    it("has no backdrop under the high-contrast theme, and gets it back after", () => {
        const [themeId, setThemeId] = createSignal("high-contrast");
        theme.read = themeId;
        const r = mount("hb-hc", () => ({ backdrop: teal }));
        expect(r()).toBeUndefined();
        expect(getPaneScope("hb-hc")()).toBeUndefined();
        setThemeId("default");
        expect(r()?.id).toBe("builtin:teal");
    });

    it("has no backdrop while forced colours are on", () => {
        Object.defineProperty(window, "matchMedia", {
            configurable: true,
            writable: true,
            value: (q: string) => ({ matches: q === "(forced-colors: active)" }),
        });
        const r = mount("hb-forced", () => ({ backdrop: teal }));
        expect(r()).toBeUndefined();
        expect(getPaneScope("hb-forced")()).toBeUndefined();
    });
});
