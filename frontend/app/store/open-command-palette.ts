// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openModal } from "./modalmodel";

// Opens the command palette. The palette reads the command registry and keymodel, which open it in turn; loading it
// here, when it is asked for, keeps those modules out of one import cycle (scripts/RATCHETS.md).
export async function openCommandPalette(): Promise<void> {
    const { CommandPaletteModal } = await import("@/app/modals/command-palette");
    openModal(CommandPaletteModal);
}
