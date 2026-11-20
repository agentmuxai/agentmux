// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Asking for the command palette while it is open closes it (the key, the logo menu's item and the macOS menu bar's
// all go through openCommandPalette), and asking for it with a menu or popover open closes that first, as a press
// outside would. SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md R20, R21.

import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ dismissed: 0 }));

vi.mock("@/app/modals/command-palette", () => ({ CommandPaletteModal: () => null }));
vi.mock("@/app/element/anchored-popover", () => ({
    dismissAnchoredPopovers: () => {
        h.dismissed++;
    },
}));

import { modalsModel } from "./modalmodel";
import { openCommandPalette } from "./open-command-palette";

afterEach(() => {
    for (const m of [...modalsModel.modalsAtom()]) modalsModel.closeModal(m.id);
    h.dismissed = 0;
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
