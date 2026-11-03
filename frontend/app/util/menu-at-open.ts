// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a right-click menu drawn by the kit (`openMenuAt`) is open. Code that must not act while a
 * context menu is open (the mouse side buttons) asks here, as it asks `isJsContextMenuOpen` for the JS menu.
 */

let close: (() => void) | null = null;

/** `openMenuAt` calls this with the close function of the menu it just opened, or null when it closed. */
export function setOpenMenuAt(next: (() => void) | null): void {
    close = next;
}

export const isMenuAtOpen = (): boolean => close !== null;

/** Close the open `openMenuAt` menu, if any. */
export const closeMenuAt = (): void => close?.();
