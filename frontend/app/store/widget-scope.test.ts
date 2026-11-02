// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Which panes a click looks for in every window (`app`) and which only in its own (`window`, the default). */

import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    widgets: {} as Record<string, unknown>,
    tabs: {} as Record<string, { scope?: string }>,
}));
vi.mock("@/app/store/global", () => ({ atoms: { fullConfigAtom: () => ({ widgets: h.widgets }) } }));
vi.mock("@/app/block/pane-tab-registry", () => ({ getPaneTab: (view: string) => h.tabs[view] }));

import { APP_SCOPE_VIEWS, BUILTIN_SINGLETON_VIEWS, paneScope } from "./widget-singleton";

afterEach(() => {
    h.widgets = {};
    h.tabs = {};
});

describe("paneScope", () => {
    it("is per window for every management pane and for the work panes that hold nothing exclusive", () => {
        for (const view of [...BUILTIN_SINGLETON_VIEWS, "terminal", "files", "sysinfo", "browser", "webview", "unknown-view"]) {
            expect(paneScope(view), view).toBe("window");
        }
        expect(paneScope(undefined)).toBe("window");
    });

    it("is app wide for the panes that hold something exclusive: an agent session and an Editor document", () => {
        expect([...APP_SCOPE_VIEWS].sort()).toEqual(["agent", "editor"]);
        expect(paneScope("agent")).toBe("app");
        expect(paneScope("editor")).toBe("app");
    });

    it("is app wide for a browser tab on a named profile, not for Personal or Incognito", () => {
        expect(paneScope("browser", { "browser:identity": "profile:work" })).toBe("app");
        expect(paneScope("browser", { "browser:identity": "incognito:0123456789abcdef" })).toBe("window");
        expect(paneScope("browser", {})).toBe("window");
        expect(paneScope("browser", { "browser:identity": "profile:Not Valid!" })).toBe("window");
    });

    it("follows the widget entry that opens the view, when it says", () => {
        h.widgets = {
            "defwidget@settings": { blockdef: { meta: { view: "settings" } }, scope: "app" },
            "defwidget@agent": { blockdef: { meta: { view: "agent" } }, scope: "window" },
            "defwidget@help": { blockdef: { meta: { view: "help" } }, scope: "nonsense" },
        };
        expect(paneScope("settings")).toBe("app");
        expect(paneScope("agent")).toBe("window");
        expect(paneScope("help"), "a value that is not a scope says nothing").toBe("window");
    });

    it("follows an extension pane's own scope, below the widget entry", () => {
        h.tabs["ext:acme.board/main"] = { scope: "app" };
        expect(paneScope("ext:acme.board/main")).toBe("app");
        h.widgets = { w: { blockdef: { meta: { view: "ext:acme.board/main" } }, scope: "window" } };
        expect(paneScope("ext:acme.board/main")).toBe("window");
    });
});
