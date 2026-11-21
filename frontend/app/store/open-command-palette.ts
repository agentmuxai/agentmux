// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { markStart } from "@/perf/marks";
import { modalsModel, openModal } from "./modalmodel";

// Opens the command palette. The palette reads the command registry and keymodel, which open it in turn; loading it
// here, not at import, keeps those modules out of one import cycle (scripts/RATCHETS.md).
//
// The palette is the one surface where a press must show something at once, so the open path awaits nothing once the
// palette's code is in memory: `preloadCommandPalette()` loads it after startup, and the open that follows is
// synchronous, so the palette is mounted in the same task as the key press and painted in the next frame. A press that
// comes before the preload has finished (a key held through startup) waits for the load, once, and opens then.
//
// Asked for while it is open, it closes: the key, the logo menu's item and the macOS menu bar's all toggle. Asked for
// with a menu or popover open, it closes those first (a dialog stays, and the palette opens above it).

interface PaletteCode {
    CommandPaletteModal: (typeof import("@/app/modals/command-palette"))["CommandPaletteModal"];
    dismissAnchoredPopovers: (typeof import("@/app/element/anchored-popover"))["dismissAnchoredPopovers"];
}

let code: PaletteCode | null = null;
let loading: Promise<PaletteCode> | null = null;

/** Loads the palette's code, once. Called after startup, and by an open that came first. */
export function preloadCommandPalette(): Promise<unknown> {
    loading ??= Promise.all([import("@/app/modals/command-palette"), import("@/app/element/anchored-popover")])
        .then(([{ CommandPaletteModal }, { dismissAnchoredPopovers }]) => (code = { CommandPaletteModal, dismissAnchoredPopovers }))
        .catch((err: unknown) => {
            // One failed load (a chunk that could not be fetched) must not be the answer to every later press: the next one tries again.
            loading = null;
            throw err;
        });
    return loading;
}

/** Loads it when the window is idle (no wait where there is no idle callback to ask for, as in a test), within 5 s. */
export function preloadCommandPaletteWhenIdle(): void {
    if (typeof requestIdleCallback === "function") requestIdleCallback(() => void preloadCommandPalette().catch(() => {}), { timeout: 5000 });
}

function toggle({ CommandPaletteModal, dismissAnchoredPopovers }: PaletteCode): void {
    if (modalsModel.isModalOpen(CommandPaletteModal)) {
        modalsModel.closeModalsOf(CommandPaletteModal);
        return;
    }
    dismissAnchoredPopovers();
    openModal(CommandPaletteModal);
}

/** Returns nothing to wait for when the palette's code is loaded; the load's promise when it is not yet. */
export function openCommandPalette(): void | Promise<void> {
    markStart("palette-open");
    if (code) return toggle(code);
    return preloadCommandPalette().then(() => toggle(code!));
}
