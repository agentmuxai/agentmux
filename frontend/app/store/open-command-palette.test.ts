// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Asking for the command palette while it is open closes it (the key, the logo menu's item and the macOS menu bar's
// all go through openCommandPalette), and asking for it with a menu or popover open closes that first, as a press
// outside would. SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md R20, R21.

import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ dismissed: 0, failLoad: false }));

vi.mock("@/app/modals/command-palette", () => ({ CommandPaletteModal: () => null }));
vi.mock("@/app/element/anchored-popover", () => ({
    // Read when the load finishes: a load that fails once (a chunk that could not be fetched) throws here, once.
    get dismissAnchoredPopovers() {
        if (h.failLoad) {
            h.failLoad = false;
            throw new Error("chunk load failed");
        }
        return () => {
            h.dismissed++;
        };
    },
}));

import { modalsModel } from "./modalmodel";
import { openCommandPalette, preloadCommandPalette } from "./open-command-palette";

afterEach(() => {
    for (const m of [...modalsModel.modalsAtom()]) modalsModel.closeModal(m.id);
    h.dismissed = 0;
});

// The palette is the one surface where a press must show something at once: once its code is loaded, the open path
// makes no call and awaits nothing, so the palette is mounted in the same task as the key press.
describe("openCommandPalette: the fast path", () => {
    it("before the code is loaded, waits for the load once and then opens; a load that failed once is tried again by the next press", async () => {
        h.failLoad = true;
        await expect(preloadCommandPalette()).rejects.toThrow("chunk load failed");
        const pending = openCommandPalette();
        expect(pending).toBeInstanceOf(Promise);
        expect(modalsModel.modalsAtom()).toHaveLength(0);
        await pending;
        expect(modalsModel.modalsAtom()).toHaveLength(1);
    });

    it("with the code loaded, opens before the call returns, and returns nothing to wait for", async () => {
        await preloadCommandPalette();
        const result = openCommandPalette();
        expect(result).toBeUndefined();
        expect(modalsModel.modalsAtom()).toHaveLength(1);
        expect(h.dismissed).toBe(1);
    });

    it("with the code loaded, closes it again in the same task", async () => {
        await preloadCommandPalette();
        openCommandPalette();
        expect(openCommandPalette()).toBeUndefined();
        expect(modalsModel.modalsAtom()).toHaveLength(0);
    });

    it("loads the code once however often it is asked to", async () => {
        const a = preloadCommandPalette();
        expect(preloadCommandPalette()).toBe(a);
        await a;
    });
});

describe("openCommandPalette", () => {
    it("opens the palette", async () => {
        await openCommandPalette();
        expect(modalsModel.modalsAtom()).toHaveLength(1);
    });

    it("closes the palette when it is asked for while it is open", async () => {
        await openCommandPalette();
        await openCommandPalette();
        expect(modalsModel.modalsAtom()).toHaveLength(0);
    });

    it("opens it again after it was closed, whichever way it closed", async () => {
        await openCommandPalette();
        modalsModel.closeTopModal();
        await openCommandPalette();
        expect(modalsModel.modalsAtom()).toHaveLength(1);
    });

    it("closes the open menus and popovers before it opens, and not when it only closes", async () => {
        await openCommandPalette();
        expect(h.dismissed).toBe(1);
        await openCommandPalette();
        expect(h.dismissed).toBe(1);
    });

    it("leaves another dialog open and opens above it", async () => {
        const Other = () => null;
        modalsModel.openModal(Other);
        await openCommandPalette();
        expect(modalsModel.modalsAtom()).toHaveLength(2);
        await openCommandPalette();
        expect(modalsModel.modalsAtom().map((m) => m.Component)).toEqual([Other]);
    });
});
