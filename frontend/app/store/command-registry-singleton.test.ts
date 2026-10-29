// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The palette's open entries follow the same singleton rule as the widget bar: one that is a
 * singleton brings back the pane already open, a "New ..." entry or a Shift on the entry opens another.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    openOrFocusPaneByView: vi.fn(() => Promise.resolve()),
    createBlock: vi.fn(() => Promise.resolve("new-block")),
    singleton: true,
}));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: h.openOrFocusPaneByView }));
vi.mock("@/app/store/widget-singleton", () => ({ isSingletonView: () => h.singleton }));
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<Record<string, unknown>>()),
    createBlock: h.createBlock,
}));

import { commandRegistry, registerDefaultCommands } from "./command-registry";

registerDefaultCommands();

afterEach(() => {
    h.openOrFocusPaneByView.mockClear();
    h.createBlock.mockClear();
    h.singleton = true;
});

describe("the palette's open entries", () => {
    it.each([
        ["open:tower", "tower"],
        ["open:help", "help"],
        ["open:swarm", "swarm"],
        ["open:kanban", "kanban"],
        ["open:vault", "vault"],
        ["open:history", "history"],
        ["open:audit", "audit"],
    ])("%s brings back the open %s pane", async (id, view) => {
        await commandRegistry.get(id)!.execute();
        expect(h.openOrFocusPaneByView).toHaveBeenCalledWith(view, { meta: { view } });
        expect(h.createBlock).not.toHaveBeenCalled();
    });

    it("opens a new pane on Shift", async () => {
        await commandRegistry.get("open:help")!.execute({ forceNew: true });
        expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "help" } });
        expect(h.openOrFocusPaneByView).not.toHaveBeenCalled();
    });

    it.each(["new:tower", "new:help", "new:swarm", "new:kanban"])("%s always opens a new pane", async (id) => {
        await commandRegistry.get(id)!.execute();
        expect(h.createBlock).toHaveBeenCalledTimes(1);
        expect(h.openOrFocusPaneByView).not.toHaveBeenCalled();
    });

    it("opens a new pane for an entry whose view is not a singleton", async () => {
        h.singleton = false;
        await commandRegistry.get("open:swarm")!.execute();
        expect(h.createBlock).toHaveBeenCalledTimes(1);
        expect(h.openOrFocusPaneByView).not.toHaveBeenCalled();
    });
});
