// Copyright 2026, Command Line Inc.
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane context menu actions: copy, paste, split, and shared menu builder.
 * Used from both handleHeaderContextMenu (header) and onContextMenu (body) in blockframe.tsx.
 */

import { paneTabCapability } from "@/app/block/pane-tab-registry";
import { shortcutsSplitBlockDef } from "./split-block-def";
import { createBlockSplitHorizontally, createBlockSplitVertically, getApi } from "@/app/store/global";
import { readText as clipboardReadText, writeText as clipboardWriteText } from "@/util/clipboard";
import { shortcutFor } from "@/app/keybindings";
import { devModeOn } from "@/app/store/dev-mode";
import { MENU_ICON } from "@/app/util/menu-icons";

/** A menu item's shortcut, from the shortcut table; undefined when it has none. */
const hint = (command: string): string | undefined => shortcutFor(command) || undefined;

type SplitDirection = "down" | "right";

// ─── Copy / Paste helpers ─────────────────────────────────────────────────────

/**
 * Get the current text selection for a pane: the tab's own
 * (`ViewModel.getSelection` — a terminal's xterm keeps its selection apart
 * from the page's, so it survives the right-click that clears the page's),
 * else the window's.
 */
function getPaneSelection(viewModel?: ViewModel): string {
    const own = viewModel?.getSelection?.();
    if (typeof own === "string") return own;
    return window.getSelection()?.toString() ?? "";
}

/**
 * Returns true if the pane accepts text input (i.e. paste makes sense) — its
 * view's `acceptsInput` capability (today: the terminal, via its PTY).
 */
function paneAcceptsInput(blockData: Block): boolean {
    return paneTabCapability(blockData.meta?.view, "acceptsInput") === true;
}

// ─── Split ────────────────────────────────────────────────────────────────────


/**
 * The block a split of `blockData` creates: a Shortcuts pane, whatever the pane is. Its tiles
 * start where the pane was (SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md); the agent picker and
 * the copy of the pane it replaces are the tiles' Agent and Terminal.
 */
function splitBlockDef(blockData: Block): BlockDef {
    return shortcutsSplitBlockDef(blockData, () => ({ meta: { view: "shortcuts" } }));
}

/** Split the pane in the given direction. */
async function handleSplitPane(blockData: Block, direction: SplitDirection): Promise<void> {
    const blockDef = splitBlockDef(blockData);

    try {
        switch (direction) {
            case "down":
                await createBlockSplitVertically(blockDef, blockData.oid, "after");
                break;
            case "right":
                await createBlockSplitHorizontally(blockDef, blockData.oid, "after");
                break;
        }
    } catch (e) {
        console.error("[pane-actions] split failed:", e);
    }
}

// ─── Menu builder ─────────────────────────────────────────────────────────────

/**
 * Join item groups with one separator between each pair of NON-EMPTY groups.
 * Empty groups contribute nothing (not even a separator), so callers can pass
 * conditionally-empty groups without ever producing a leading, trailing, or
 * doubled separator.
 */
export function joinMenuGroups(groups: ContextMenuItem[][]): ContextMenuItem[] {
    const menu: ContextMenuItem[] = [];
    for (const group of groups) {
        if (group.length === 0) continue;
        if (menu.length > 0) menu.push({ type: "separator" });
        menu.push(...group);
    }
    return menu;
}

/**
 * Named sections of the pane menu. A region (see context-menu-region.ts) drops
 * sections by name instead of filtering by label. `viewItems` is composed by
 * blockframe.tsx (ViewModel.getBodyContextMenuItems) ahead of these, so it is
 * honoured there rather than in buildPaneContextMenu.
 */
export type PaneMenuSection =
    | "viewItems" // viewModel.getBodyContextMenuItems()
    | "clipboard" // Copy / Paste
    | "split" // Split Down / Right
    | "magnify" // Magnify / Un-Magnify Pane
    | "close" // Close Pane
    | "inspect"; // Inspect Element

export interface PaneContextMenuOpts {
    magnified: boolean;
    onMagnifyToggle: () => void;
    onClose: () => void;
    /**
     * Right-click coordinates in window-relative pixels. When supplied, the
     * menu adds an "Inspect Element" entry at the bottom that opens DevTools
     * focused on the element at those coords (CEF's
     * `show_dev_tools(..., inspect_element_at)`). Omit to suppress the entry.
     */
    inspectAt?: { x: number; y: number };
    /** Sections to leave out entirely. Separators are generated between the
     *  surviving groups, so any combination is safe. */
    omit?: ReadonlySet<PaneMenuSection>;
}

/**
 * Build the reusable pane context menu items shared between header and body right-click.
 * Pass viewModel to enable terminal-aware copy/paste.
 *
 * Layout is a list of groups joined by separators; empty groups (everything in
 * them omitted, or nothing applies) are skipped, so no combination can leave a
 * leading, trailing, or doubled separator:
 *
 *   [clipboard] ─ [split] ─ [magnify, close] ─ [inspect]
 */
export function buildPaneContextMenu(
    blockData: Block,
    opts: PaneContextMenuOpts,
    viewModel?: ViewModel
): ContextMenuItem[] {
    const has = (section: PaneMenuSection) => !opts.omit?.has(section);

    const clipboard: ContextMenuItem[] = [];
    if (has("clipboard")) {
        const selection = getPaneSelection(viewModel);
        // Copy — always present; disabled when nothing is selected
        clipboard.push({
            label: "Copy",
            icon: MENU_ICON.copy,
            enabled: selection.length > 0,
            click: () => {
                if (selection) {
                    clipboardWriteText(selection).catch(console.error);
                }
            },
        });
        // Paste — only shown for input-accepting panes (terminals)
        if (paneAcceptsInput(blockData)) {
            clipboard.push({
                label: "Paste",
                icon: MENU_ICON.paste,
                click: () => {
                    void (async () => {
                        try {
                            const text = await clipboardReadText();
                            if (!text) return;
                            viewModel?.paste?.(text);
                            // The next keystroke (usually Enter) belongs to
                            // the pane just pasted into.
                            viewModel?.giveFocus?.();
                        } catch (e) {
                            console.error("[pane-actions] paste failed:", e);
                        }
                    })();
                },
            });
        }
    }

    const split: ContextMenuItem[] = has("split")
        ? [
              { label: "Split Down", icon: MENU_ICON.splitDown, sublabel: hint("split:down"), click: () => void handleSplitPane(blockData, "down") },
              { label: "Split Right", icon: MENU_ICON.splitRight, sublabel: hint("split:right"), click: () => void handleSplitPane(blockData, "right") },
          ]
        : [];

    const paneActions: ContextMenuItem[] = [
        ...(has("magnify")
            ? [{ label: opts.magnified ? "Un-Magnify Pane" : "Magnify Pane", icon: opts.magnified ? MENU_ICON.unmagnify : MENU_ICON.magnify, sublabel: hint("pane:magnify"), click: opts.onMagnifyToggle }]
            : []),
        ...(has("close") ? [{ label: "Close Pane", icon: MENU_ICON.close, sublabel: hint("pane:close"), click: opts.onClose }] : []),
    ];

    // Inspect Element — opens CEF DevTools focused on whatever was
    // under the right-click. Only appears when `inspectAt` was supplied
    // (call sites that don't capture click coords don't get the entry).
    // Shown only in dev mode (app:devmode).
    const inspect: ContextMenuItem[] =
        has("inspect") && opts.inspectAt && devModeOn()
            ? [
                  {
                      label: "Inspect Element",
                      icon: MENU_ICON.inspect,
                      click: () => {
                          try {
                              getApi().inspectElementAt(opts.inspectAt!.x, opts.inspectAt!.y);
                          } catch (e) {
                              console.error("[pane-actions] inspect failed:", e);
                          }
                      },
                  },
              ]
            : [];

    return joinMenuGroups([clipboard, split, paneActions, inspect]);
}
