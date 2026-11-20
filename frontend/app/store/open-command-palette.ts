// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { modalsModel, openModal } from "./modalmodel";

// Opens the command palette. The palette reads the command registry and keymodel, which open it in turn; loading it
// here, when it is asked for, keeps those modules out of one import cycle (scripts/RATCHETS.md).
//
// Asked for while it is open, it closes: the key, the logo menu's item and the macOS menu bar's all toggle. Asked for
// with a menu or popover open, it closes those first (a dialog stays, and the palette opens above it).
export async function openCommandPalette(): Promise<void> {
    const [{ CommandPaletteModal }, { dismissAnchoredPopovers }] = await Promise.all([
        import("@/app/modals/command-palette"),
        import("@/app/element/anchored-popover"),
    ]);
    if (modalsModel.isModalOpen(CommandPaletteModal)) {
        modalsModel.closeModalsOf(CommandPaletteModal);
        return;
    }
    dismissAnchoredPopovers();
    openModal(CommandPaletteModal);
}
