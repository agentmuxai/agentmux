// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Backdrop menu: laid out like Pane Color, a pick applies at once and keeps the menu open, and an old record is kept. */

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setBlockMeta: vi.fn(), light: false }));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: h.setBlockMeta }));
vi.mock("@/app/store/global", () => ({ MOS: { getMuxObjectAtom: () => () => undefined, makeORef: () => "" } }));
vi.mock("@/app/element/ui/open-menu-at", () => ({ openMenuAt: vi.fn() }));
vi.mock("../block/pane-identity", () => ({ isLightThemeActive: () => h.light }));

import { backdropMenuItems, backdropSubmenuItem } from "./backdrop-menu";
import { BUILTINS, deriveBuiltin } from "./builtins";

const HUES = ["Rose", "Red", "Orange", "Amber", "Yellow", "Green", "Jade", "Teal", "Sky", "Blue", "Violet", "Magenta"];
const block = (backdrop?: unknown) => ({ oid: "block-1", meta: { view: "agent", backdrop } }) as unknown as Block;
const rows = (backdrop?: unknown) => backdropMenuItems(block(backdrop));
const colours = (items: ContextMenuItem[]) => items.filter((i) => i.type !== "separator" && i.label !== "None");

beforeEach(() => {
    h.setBlockMeta.mockClear();
    h.light = false;
});

describe("the Backdrop menu", () => {
    it("is None, the 12 hues, then Black and White in a group of their own: the Pane Color layout", () => {
        expect(rows().map((i) => (i.type === "separator" ? "-" : i.label))).toEqual(["None", "-", ...HUES, "-", "Black", "White"]);
    });

    it("draws each backdrop as the surface it makes under the app theme", () => {
        const dark = colours(rows());
        expect(dark.map((i) => i.swatchColor)).toEqual(BUILTINS.map((b) => deriveBuiltin(b.id, "dark")!.surfaces.surface));
        h.light = true;
        expect(colours(rows())[3].swatchColor).toBe(deriveBuiltin(BUILTINS[3].id, "light")!.surfaces.surface);
    });

    it("is the Backdrop row of the header menu, with those rows inside", () => {
        const item = backdropSubmenuItem(block());
        expect([item.label, item.type, item.submenu?.length]).toEqual(["Backdrop", "submenu", 17]);
    });

    it("keeps the menu open on every pick, None included", () => {
        for (const item of rows().filter((i) => i.type !== "separator")) expect(item.keepOpen, item.label).toBe(true);
    });

    it("checks None with no backdrop, and the chosen colour otherwise; a custom colour checks nothing", () => {
        const checked = (backdrop?: unknown) => rows(backdrop).filter((i) => i.checked).map((i) => i.label);
        expect(checked()).toEqual(["None"]);
        expect(checked({ kind: "builtin", id: "teal", polarity: "follow", version: 1 })).toEqual(["Teal"]);
        expect(checked({ kind: "color", hex: "#336699", polarity: "follow", version: 1 })).toEqual([]);
    });

    it("follows the pane while it is open: the check moves with a pick", () => {
        const now = block();
        const items = backdropMenuItems(now, () => now);
        const live = () => items.filter((i) => i.live?.().checked).map((i) => i.label);
        expect(live()).toEqual(["None"]);
        (now.meta as Record<string, unknown>).backdrop = { kind: "builtin", id: "blue", polarity: "follow", version: 1 };
        expect(live()).toEqual(["Blue"]);
    });

    it("writes the pick at once, keeping the polarity the pane already has", () => {
        const teal = rows({ kind: "builtin", id: "rose", polarity: "dark", version: 1 }).find((i) => i.label === "Teal")!;
        teal.click!();
        expect(h.setBlockMeta).toHaveBeenCalledWith("block-1", { backdrop: { kind: "builtin", id: "teal", polarity: "dark", version: 1 } });
        h.setBlockMeta.mockClear();
        rows().find((i) => i.label === "Red")!.click!();
        expect(h.setBlockMeta).toHaveBeenCalledWith("block-1", { backdrop: { kind: "builtin", id: "red", polarity: "follow", version: 1 } });
    });

    it("None clears the record", () => {
        rows({ kind: "color", hex: "#336699", polarity: "follow", version: 1 }).find((i) => i.label === "None")!.click!();
        expect(h.setBlockMeta).toHaveBeenCalledWith("block-1", { backdrop: null });
    });
});
