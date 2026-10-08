// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The screenshot tooling's pure parts: size parsing, file names, the widget
// suite built from widgets.json, and the demo widget config. Capturing itself
// needs a running instance and isn't tested here.

import { describe, expect, it } from "vitest";
import { demoPathProblem, demoWidgets } from "./demo-env.mjs";
import { SIZES, parseSizes, shotFilename } from "./sizes.mjs";
import { EXCLUDE, shots, widgetEntries } from "./widget-shots.mjs";

describe("sizes", () => {
    it("defaults to every size and rejects unknown names", () => {
        expect(parseSizes(undefined)).toEqual(["small", "medium", "large"]);
        expect(parseSizes("all")).toEqual(Object.keys(SIZES));
        expect(parseSizes("small, large")).toEqual(["small", "large"]);
        expect(() => parseSizes("huge")).toThrow(/unknown size "huge"/);
    });

    it("names a sized shot by id and size, and an unsized one by its position", () => {
        expect(shotFilename("03", "widget-files", "medium")).toBe("widget-files-medium.png");
        expect(shotFilename("03", "top-tab-bar", null)).toBe("03-top-tab-bar.png");
    });
});

describe("widget suite", () => {
    const config = {
        "defwidget@agent": { label: "Agent", icon: "sparkles", blockdef: { meta: { view: "agent" } } },
        "defwidget@files": { label: "Hangar", icon: "folder-open", blockdef: { meta: { view: "files" } } },
        "defwidget@messengers": { label: "Messengers", icon: "comments" },
        "defwidget@slack": {
            label: "Slack",
            icon: "brands@slack",
            "display:hidden": true,
            blockdef: { meta: { view: "browser" } },
        },
        "defwidget@browser": { label: "Browser", icon: "globe", blockdef: { meta: { view: "browser" } } },
    };

    it("captures every visible widget with a view, minus the excluded ones", () => {
        expect(widgetEntries(config).map((w) => w.name)).toEqual(["agent", "files"]);
        expect(EXCLUDE.browser).toBeTruthy();
        expect(EXCLUDE.messengers).toBeTruthy();
    });

    it("builds one sized shot per widget in the real config, each with a way to open it", () => {
        expect(shots.length).toBeGreaterThan(10);
        for (const s of shots) {
            expect(s.id).toMatch(/^widget-[a-z]+$/);
            expect(s.sizes).toBe(true);
            expect(typeof s.prep).toBe("function");
            expect(typeof s.cleanup).toBe("function");
        }
        expect(shots.find((s) => s.id === "widget-toolchain").containsWorkspaceData).toBe(true);
        expect(shots.find((s) => s.id === "widget-files").containsWorkspaceData).toBe("review");
    });
});

describe("demo widget config", () => {
    const builtin = {
        "defwidget@files": { label: "Hangar", icon: "folder-open", blockdef: { meta: { view: "files" } } },
        "defwidget@terminal": { label: "Terminal", blockdef: { meta: { view: "term", controller: "shell" } } },
        "defwidget@editor": {
            label: "Editor",
            blockdef: { meta: { view: "editor", "editor:tree_expanded": true, "editor:scratch": true } },
        },
    };

    it("starts Hangar, Terminal and Editor in the demo project and keeps the rest of each entry", () => {
        const w = demoWidgets("D:/demo/acme-web", builtin);
        expect(w["defwidget@files"]).toEqual({
            label: "Hangar",
            icon: "folder-open",
            blockdef: { meta: { view: "files", "files:path": "D:/demo/acme-web" } },
        });
        expect(w["defwidget@terminal"].blockdef.meta).toEqual({
            view: "term",
            controller: "shell",
            "cmd:cwd": "D:/demo/acme-web",
        });
        expect(w["defwidget@editor"].blockdef.meta).toEqual({
            view: "editor",
            "editor:tree_expanded": false,
            file: "D:/demo/acme-web/src/App.tsx",
        });
        // The built-ins aren't modified.
        expect(builtin["defwidget@editor"].blockdef.meta["editor:scratch"]).toBe(true);
    });
});

describe("demo project path", () => {
    const machine = { user: "alice", home: "C:/Users/alice" };

    it("accepts a path that says nothing about the machine", () => {
        expect(demoPathProblem("D:/demo/acme-web", machine)).toBeNull();
        expect(demoPathProblem("/opt/demo/acme-web", { user: "alice", home: "/home/alice" })).toBeNull();
    });

    it("refuses a path under the home folder or containing the user name", () => {
        expect(demoPathProblem("C:/Users/alice/demo/acme-web", machine)).toMatch(/home folder/);
        expect(demoPathProblem("c:/users/ALICE", machine)).toMatch(/home folder/);
        expect(demoPathProblem("D:/alice-shots/acme-web", machine)).toMatch(/user name/);
    });
});
