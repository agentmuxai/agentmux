// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Sections as part of the pane contract (pane-sections.ts), and the SDK's list of them. */

import { readFileSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { SETTINGS_SECTIONS } from "@/app/view/settings/settings-model";
import { CONNECTORS_SECTIONS, MEMORY_SECTIONS } from "@/app/view/section-pane/panes";
import { VAULT_SECTIONS } from "@/app/view/vault/vault-kinds";
import { WARDEN_SECTIONS } from "@/app/view/warden/warden-model";
import { sectionKeyOf, sectionLabelOf, sectionOf, sectionProblem, type PaneSections } from "./pane-sections";
import { registerPaneTab } from "./pane-tab-registry";

const BOARD: PaneSections<"todo" | "done"> = {
    default: "todo",
    items: [
        { id: "todo", label: "To do", icon: "list" },
        { id: "done", label: "Done", icon: "check" },
    ],
};

let unregister: () => void = () => {};
beforeAll(() => {
    unregister = registerPaneTab({
        apiVersion: 1,
        view: "test-board",
        aliases: ["old-board"],
        label: "Board",
        icon: "x",
        sections: BOARD,
        create: () => ({ component: () => null }),
    });
});
afterAll(() => unregister());

describe("pane sections", () => {
    it("keeps the selected section under <view>:section, falling back to the default", () => {
        expect(sectionKeyOf("warden")).toBe("warden:section");
        expect(sectionOf(BOARD, "test-board", { "test-board:section": "done" })).toBe("done");
        expect(sectionOf(BOARD, "test-board", { "test-board:section": "nope" })).toBe("todo");
        expect(sectionOf(BOARD, "test-board", { "other:section": "done" })).toBe("todo");
        expect(sectionOf(BOARD, "test-board", null)).toBe("todo");
    });

    it("says why a hand-written section can't open, naming the ones that can", () => {
        expect(sectionProblem("test-board", "done")).toBeNull();
        expect(sectionProblem("old-board", "done")).toBe(`"old-board" is now the test-board pane: use "view": "test-board"`);
        expect(sectionProblem("test-board", "nope")).toBe('the test-board pane has no "nope" section (it has todo, done)');
        expect(sectionProblem("term", "x")).toBe("the term pane has no sections");
        expect(sectionProblem(undefined, "x")).toBe('"section": "x" needs a "view" in its blockdef');
        expect(sectionLabelOf("test-board", "done")).toBe("Done");
    });

    it("lists the same sections in the widget SDK as the panes declare", () => {
        const dts = readFileSync("sdk/widget-sdk/v1.d.ts", "utf-8");
        const block = dts.slice(dts.indexOf("export interface PaneSections {"));
        const body = block.slice(0, block.indexOf("}"));
        const sdk: Record<string, string[]> = {};
        for (const m of body.matchAll(/^\s+([a-z]+): (.+);$/gm)) sdk[m[1]] = [...m[2].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
        const ids = (s: PaneSections) => s.items.map((i) => i.id);
        expect(sdk).toEqual({
            warden: ids(WARDEN_SECTIONS),
            vault: ids(VAULT_SECTIONS),
            connectors: ids(CONNECTORS_SECTIONS),
            memory: ids(MEMORY_SECTIONS),
            settings: ids(SETTINGS_SECTIONS),
        });
    });
});
