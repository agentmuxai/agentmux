// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import {
    getPaneTab,
    isKeepAliveView,
    paneTabIconFor,
    paneTabLabelFor,
    registerPaneTab,
    resolvePaneTabView,
} from "./pane-tab-registry";
import { stubPaneTab } from "./pane-tab-test-utils";

const unregisters: (() => void)[] = [];
function register(...args: Parameters<typeof stubPaneTab>) {
    const unregister = registerPaneTab(stubPaneTab(...args));
    unregisters.push(unregister);
    return unregister;
}

afterEach(() => {
    while (unregisters.length) unregisters.pop()!();
});

describe("pane tab registry", () => {
    it("looks a view up by its name", () => {
        register("fake", { label: "Fake", icon: "flask" });
        const m = getPaneTab("fake");
        expect(m?.view).toBe("fake");
        expect(m?.apiVersion).toBe(1);
        expect(m?.create).toBeTypeOf("function");
    });

    it("resolves an alias to the canonical view, for lookups and for the view name", () => {
        register("fake", { label: "Fake", icon: "flask", aliases: ["oldfake"] });
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
        const unregister = register("fake", { label: "Fake", icon: "flask", aliases: ["oldfake"] });
        unregister();
        expect(getPaneTab("fake")).toBeUndefined();
        expect(resolvePaneTabView("oldfake")).toBe("oldfake");
    });

    it("refuses a second registration of the same view or alias", () => {
        register("fake", { label: "Fake", icon: "flask", aliases: ["oldfake"] });
        expect(() => registerPaneTab(stubPaneTab("fake", { label: "X", icon: "x" }))).toThrow();
        expect(() =>
            registerPaneTab(stubPaneTab("other", { label: "X", icon: "x", aliases: ["oldfake"] }))
        ).toThrow();
        expect(() => registerPaneTab(stubPaneTab("oldfake", { label: "X", icon: "x" }))).toThrow();
    });

    it("keep-alive is a per-view capability, remount by default", () => {
        register("kept", { label: "Kept", icon: "k", lifecycle: "keepAlive" });
        register("plain", { label: "Plain", icon: "p" });
        expect(isKeepAliveView("kept")).toBe(true);
        expect(isKeepAliveView("plain")).toBe(false);
        expect(isKeepAliveView("unknown")).toBe(false);
    });

    it("label and icon come from the manifest, with the old fallbacks for unknown views", () => {
        register("fake", { label: "Fake", icon: "flask" });
        expect(paneTabLabelFor("fake")).toBe("Fake");
        expect(paneTabIconFor("fake")).toBe("flask");
        expect(paneTabLabelFor("unknown")).toBe("unknown");
        expect(paneTabIconFor("unknown")).toBe("square");
        expect(paneTabLabelFor("")).toBe("(No View)");
        expect(paneTabLabelFor(undefined)).toBe("(No View)");
    });

    it("the test stub defaults the label to the view name and the icon to a square", () => {
        register("bare");
        expect(paneTabLabelFor("bare")).toBe("bare");
        expect(paneTabIconFor("bare")).toBe("square");
    });

    it("carries a pane-tab descriptor for pill labels and icons", () => {
        const tab = { label: () => "From descriptor" };
        register("fake", { label: "Fake", icon: "flask", tab });
        expect(getPaneTab("fake")?.tab).toBe(tab);
    });
});

describe("create(ctx)", () => {
    const create = () => ({ component: () => null as any });

    it("registers a manifest with its instance factory", () => {
        unregisters.push(registerPaneTab({ apiVersion: 1, view: "native", label: "Native", icon: "n", create }));
        expect(getPaneTab("native")?.create).toBe(create);
    });

    // A widget's manifest is plain JS, so the type alone doesn't guarantee it.
    it("refuses a manifest without create", () => {
        expect(() => registerPaneTab({ apiVersion: 1, view: "neither", label: "N", icon: "n" } as any)).toThrow(
            /needs create/
        );
        expect(getPaneTab("neither")).toBeUndefined();
    });
});
