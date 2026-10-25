// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { MENU_ICON, menuIconClass } from "./menu-icons";

const CSS_DIR = join(process.cwd(), "public", "fontawesome", "css");
const css = readdirSync(CSS_DIR)
    .filter((f) => f.endsWith(".css"))
    .map((f) => readFileSync(join(CSS_DIR, f), "utf8"))
    .join("\n");
const exists = (name: string) => new RegExp(`\\.fa-${name}\\b`).test(css);

describe("menu icon table", () => {
    it("names only icons the vendored Font Awesome stylesheets define", () => {
        for (const [action, icon] of Object.entries(MENU_ICON)) {
            const name = icon.replace(/^(solid|regular|brands)@/, "");
            expect(exists(name), `${action}: ${icon}`).toBe(true);
        }
    });

    it("gives every action a class list", () => {
        for (const [action, icon] of Object.entries(MENU_ICON)) {
            expect(menuIconClass(icon), action).not.toBeNull();
        }
    });
});

describe("menuIconClass", () => {
    it("is a fixed-width Font Awesome glyph in the shared slot", () => {
        const cls = menuIconClass("copy")!.split(" ");
        expect(cls).toEqual(expect.arrayContaining(["fa-solid", "fa-copy", "fa-fw", "menu-item-icon"]));
    });

    it("reads the weight prefix", () => {
        expect(menuIconClass("regular@window-maximize")).toContain("fa-regular");
    });

    it("is null for nothing or for a name that isn't one", () => {
        expect(menuIconClass(undefined)).toBeNull();
        expect(menuIconClass("")).toBeNull();
        expect(menuIconClass("Not A Name!")).toBeNull();
    });
});
