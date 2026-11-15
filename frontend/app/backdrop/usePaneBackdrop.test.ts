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

/** A `(forced-colors: active)` media query the test can flip, counting its change listeners. */
function fakeForcedColors(initial: boolean) {
    let matches = initial;
    const listeners = new Set<(e: { matches: boolean }) => void>();
    Object.defineProperty(window, "matchMedia", {
        configurable: true,
        writable: true,
        value: (q: string) => ({
            get matches() {
                return q === "(forced-colors: active)" && matches;
            },
            addEventListener: (_: string, fn: (e: { matches: boolean }) => void) => listeners.add(fn),
            removeEventListener: (_: string, fn: (e: { matches: boolean }) => void) => listeners.delete(fn),
        }),
    });
    return {
        set(next: boolean) {
            matches = next;
            for (const fn of [...listeners]) fn({ matches: next });
        },
        listeners: () => listeners.size,
    };
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

    it("has no backdrop while forced colours are on, and follows the system turning them on and off while mounted", () => {
        const q = fakeForcedColors(true);
        const r = mount("hb-forced", () => ({ backdrop: teal }));
        expect(r()).toBeUndefined();
        expect(getPaneScope("hb-forced")()).toBeUndefined();
        q.set(false);
        expect(r()?.id).toBe("builtin:teal");
        expect(getPaneScope("hb-forced")()?.["--surface"]).toMatch(/^#[0-9a-f]{6}$/);
        q.set(true);
        expect(r()).toBeUndefined();
        expect(getPaneScope("hb-forced")()).toBeUndefined();
    });

    it("stops listening for forced colours when the pane is gone", () => {
        const q = fakeForcedColors(false);
        const dispose = createRoot((d) => {
            usePaneBackdrop("hb-listen", () => ({ backdrop: teal }));
            return d;
        });
        expect(q.listeners()).toBe(1);
        dispose();
        expect(q.listeners()).toBe(0);
    });

    it("a second frame for the block (owner: false) resolves the backdrop but never registers or releases the live scope", () => {
        mount("hb-owner", () => ({ backdrop: teal }));
        const live = getPaneScope("hb-owner")();
        expect(live?.["--surface"]).toMatch(/^#[0-9a-f]{6}$/);
        const dispose = createRoot((d) => {
            const r = usePaneBackdrop("hb-owner", () => ({ backdrop: { ...teal, id: "crimson" } }), { owner: false });
            expect(r()?.id).toBe("builtin:crimson");
            return d;
        });
        expect(getPaneScope("hb-owner")()).toEqual(live);
        dispose();
        expect(getPaneScope("hb-owner")()).toEqual(live);
    });
});
