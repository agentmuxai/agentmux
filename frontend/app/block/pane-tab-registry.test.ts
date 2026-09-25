// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import {
    getPaneTab,
    isKeepAliveView,
    legacyAdapter,
    paneTabIconFor,
    paneTabLabelFor,
    registerPaneTab,
    resolvePaneTabView,
} from "./pane-tab-registry";

class FakeViewModel {
    viewType = "fake";
    viewComponent = null;
    constructor(public blockId: string) {}
}

const unregisters: (() => void)[] = [];
function register(...args: Parameters<typeof legacyAdapter>) {
    const unregister = registerPaneTab(legacyAdapter(...args));
    unregisters.push(unregister);
    return unregister;
}

afterEach(() => {
    while (unregisters.length) unregisters.pop()!();
});

describe("pane tab registry", () => {
    it("looks a view up by its name", () => {
        register("fake", FakeViewModel as any, { label: "Fake", icon: "flask" });
        const m = getPaneTab("fake");
        expect(m?.view).toBe("fake");
        expect(m?.apiVersion).toBe(1);
        expect(m?.viewModelClass).toBe(FakeViewModel);
    });

    it("resolves an alias to the canonical view, for lookups and for the view name", () => {
        register("fake", FakeViewModel as any, { label: "Fake", icon: "flask", aliases: ["oldfake"] });
        expect(resolvePaneTabView("oldfake")).toBe("fake");
        expect(getPaneTab("oldfake")?.view).toBe("fake");
        expect(resolvePaneTabView("fake")).toBe("fake");
        expect(resolvePaneTabView("unknown")).toBe("unknown");
    });

    it("knows nothing about an unregistered view", () => {
        expect(getPaneTab("nope")).toBeUndefined();
        expect(getPaneTab("")).toBeUndefined();
    });

    it("unregistering removes the view and its aliases", () => {
        const unregister = register("fake", FakeViewModel as any, { label: "Fake", icon: "flask", aliases: ["oldfake"] });
        unregister();
        expect(getPaneTab("fake")).toBeUndefined();
        expect(resolvePaneTabView("oldfake")).toBe("oldfake");
    });

    it("refuses a second registration of the same view or alias", () => {
        register("fake", FakeViewModel as any, { label: "Fake", icon: "flask", aliases: ["oldfake"] });
        expect(() => registerPaneTab(legacyAdapter("fake", FakeViewModel as any, { label: "X", icon: "x" }))).toThrow();
        expect(() =>
            registerPaneTab(legacyAdapter("other", FakeViewModel as any, { label: "X", icon: "x", aliases: ["oldfake"] }))
        ).toThrow();
        expect(() => registerPaneTab(legacyAdapter("oldfake", FakeViewModel as any, { label: "X", icon: "x" }))).toThrow();
    });

    it("keep-alive is a per-view capability, remount by default", () => {
        register("kept", FakeViewModel as any, { label: "Kept", icon: "k", lifecycle: "keepAlive" });
        register("plain", FakeViewModel as any, { label: "Plain", icon: "p" });
        expect(isKeepAliveView("kept")).toBe(true);
        expect(isKeepAliveView("plain")).toBe(false);
        expect(isKeepAliveView("unknown")).toBe(false);
    });

    it("label and icon come from the manifest, with the old fallbacks for unknown views", () => {
        register("fake", FakeViewModel as any, { label: "Fake", icon: "flask" });
        expect(paneTabLabelFor("fake")).toBe("Fake");
        expect(paneTabIconFor("fake")).toBe("flask");
        expect(paneTabLabelFor("unknown")).toBe("unknown");
        expect(paneTabIconFor("unknown")).toBe("square");
        expect(paneTabLabelFor("")).toBe("(No View)");
        expect(paneTabLabelFor(undefined)).toBe("(No View)");
    });

    it("the legacy adapter defaults the label to the view name and the icon to a square", () => {
        register("bare", FakeViewModel as any);
        expect(paneTabLabelFor("bare")).toBe("bare");
        expect(paneTabIconFor("bare")).toBe("square");
    });

    it("carries a pane-tab descriptor for pill labels and icons", () => {
        const tab = { label: () => "From descriptor" };
        register("fake", FakeViewModel as any, { label: "Fake", icon: "flask", tab });
        expect(getPaneTab("fake")?.tab).toBe(tab);
    });
});
