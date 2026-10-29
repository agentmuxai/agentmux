// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Sections as part of the pane contract (pane-sections.ts), and the SDK's list of them. */

import { readFileSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { SETTINGS_SECTIONS } from "@/app/view/settings/settings-model";
import { CONNECTORS_SECTIONS, MEMORY_SECTIONS } from "@/app/view/section-pane/panes";
import { WARDEN_SECTIONS } from "@/app/view/warden/warden-model";
import { VAULT_SECTIONS } from "@/app/view/vault/vault-kinds";
import { SWARM_SECTIONS } from "@/app/view/swarm/swarm-section";
import {
    isSectionMeta,
    movedMeta,
    movedSection,
    retiredSection,
    sectionKeyOf,
    sectionKeysIn,
    sectionLabelOf,
    sectionMetaOf,
    sectionOf,
    sectionProblem,
    type PaneSections,
} from "./pane-sections";
import { registerPaneTab } from "./pane-tab-registry";

const BOARD: PaneSections<"todo" | "done"> = {
    default: "todo",
    items: [
        { id: "todo", label: "To do", icon: "list" },
        {
            id: "done",
            label: "Done",
            icon: "check",
            sections: { default: "week", items: [{ id: "week", label: "This week", icon: "x" }, { id: "all", label: "All", icon: "x" }] },
        },
    ],
};

let unregister: () => void = () => {};
const unregisterReal: (() => void)[] = [];
beforeAll(() => {
    // Warden, the Vault and the Swarm, as their manifests declare their sections.
    for (const [view, sections] of [["warden", WARDEN_SECTIONS], ["vault", VAULT_SECTIONS], ["swarm", SWARM_SECTIONS]] as const) {
        unregisterReal.push(registerPaneTab({ apiVersion: 1, view, label: view, icon: "x", sections, create: () => ({ component: () => null }) }));
    }
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
afterAll(() => {
    unregister();
    unregisterReal.forEach((u) => u());
});

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

    it("reaches a section's own sections with <id>/<sub id> and <id>:section", () => {
        expect(sectionProblem("test-board", "done/all")).toBeNull();
        expect(sectionProblem("test-board", "done/nope")).toBe(`the test-board pane's "done" section has no "nope" (it has week, all)`);
        expect(sectionProblem("test-board", "todo/all")).toBe(`the test-board pane's "todo" section has no sections of its own`);
        expect(sectionMetaOf("test-board", "done/all")).toEqual({ "test-board:section": "done", "done:section": "all" });
        expect(sectionMetaOf("test-board", "todo")).toEqual({ "test-board:section": "todo" });
        expect(isSectionMeta("test-board", "test-board:section", "done")).toBe(true);
        expect(isSectionMeta("test-board", "done:section", "all")).toBe(true);
        expect(isSectionMeta("test-board", "done:section", "nope")).toBe(false);
        expect(isSectionMeta("test-board", "todo:section", "all")).toBe(false);
        expect(isSectionMeta("term", "term:section", "x")).toBe(false);
        expect(sectionKeysIn({ view: "warden", "warden:section": "vault", "vault:section": "cards", url: "x" })).toEqual({
            "warden:section": "vault",
            "vault:section": "cards",
        });
        expect(sectionKeysIn({ view: "warden" })).toBeUndefined();
    });

    it("sends a section that moved panes to where it lives now, and leaves everything else alone", () => {
        // The Kanban left Warden for the Swarm, then the Swarm for a pane of its own: both open that pane.
        expect(movedSection("warden", "kanban")).toEqual({ view: "kanban" });
        expect(movedSection("swarm", "kanban")).toEqual({ view: "kanban" });
        // Warden's Vault, History, Audit and Host are panes of their own.
        expect(movedSection("warden", "vault")).toEqual({ view: "vault" });
        expect(movedSection("warden", "history")).toEqual({ view: "history" });
        expect(movedSection("warden", "audit")).toEqual({ view: "audit" });
        expect(movedSection("warden", "host")).toEqual({ view: "swarm" });
        expect(movedSection("warden", "jobs")).toBeNull();
        expect(movedSection("swarm", "broadcast")).toBeNull();
        // The Vault's own section carries over under the key the Vault pane reads.
        expect(movedMeta({ view: "warden", "warden:section": "vault", "vault:section": "cards" })).toEqual({
            view: "vault",
            "vault:section": "cards",
        });
        expect(movedMeta({ view: "warden", "warden:section": "kanban", "kanban:open": "t1" })).toEqual({
            view: "kanban",
            "kanban:open": "t1",
        });
        expect(movedMeta({ view: "swarm", "swarm:section": "kanban", "kanban:lanes": true })).toEqual({
            view: "kanban",
            "kanban:lanes": true,
        });
        const same = { view: "warden", "warden:section": "jobs" };
        expect(movedMeta(same)).toBe(same);
        expect(movedMeta(undefined)).toBeUndefined();
        const broadcast = { view: "swarm", "swarm:section": "broadcast" };
        expect(movedMeta(broadcast)).toBe(broadcast);
    });

    it("does not flag a moved pair in Settings > Widgets", () => {
        // Warden declares no Kanban section now, but an entry naming it still opens (MOVED_SECTIONS).
        expect(sectionProblem("warden", "kanban")).toBeNull();
        expect(sectionProblem("swarm", "kanban")).toBeNull();
        expect(sectionProblem("warden", "vault/cards")).toBeNull();
        expect(sectionProblem("warden", "history")).toBeNull();
        expect(sectionProblem("warden", "nope")).toMatch(/no "nope" section/);
    });

    it("says where a retired section went, and opens the pane with no section", () => {
        expect(retiredSection("warden", "lan")).toMatch(/network pill/);
        expect(retiredSection("warden", "ambient")).toMatch(/token popover/);
        expect(retiredSection("warden", "internet")).toMatch(/MuxBus Cloud/);
        expect(retiredSection("warden", "jobs")).toBeNull();
        expect(sectionProblem("warden", "lan")).toMatch(/"lan" section is gone: LAN peers are in the status bar/);
        expect(movedMeta({ view: "warden", "warden:section": "lan", "term:zoom": 1.2 })).toEqual({ view: "warden", "term:zoom": 1.2 });
    });

    it("lists the same sections in the widget SDK as the panes declare", () => {
        const dts = readFileSync("sdk/widget-sdk/v1.d.ts", "utf-8");
        const block = dts.slice(dts.indexOf("export interface PaneSections {"));
        const body = block.slice(0, block.indexOf("}"));
        const sdk: Record<string, string[]> = {};
        for (const m of body.matchAll(/^\s+([a-z]+): (.+);$/gm)) sdk[m[1]] = [...m[2].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
        const ids = (s: PaneSections) => s.items.map((i) => i.id);
        expect(sdk).toEqual({
            swarm: ids(SWARM_SECTIONS),
            warden: ids(WARDEN_SECTIONS),
            vault: ids(VAULT_SECTIONS),
            connectors: ids(CONNECTORS_SECTIONS),
            memory: ids(MEMORY_SECTIONS),
            settings: ids(SETTINGS_SECTIONS),
        });
    });
});
