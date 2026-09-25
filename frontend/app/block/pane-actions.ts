// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane context menu actions: copy, paste, split, and shared menu builder.
 * Used from both handleHeaderContextMenu (header) and onContextMenu (body) in blockframe.tsx.
 */

import { paneTabCapability } from "@/app/block/pane-tab-registry";
import { atoms, createBlockSplitHorizontally, createBlockSplitVertically, getApi, replaceBlock } from "@/app/store/global";
import { buildPaneWidgetMenuItems } from "@/app/window/action-widgets-config";
import { readText as clipboardReadText, writeText as clipboardWriteText } from "@/util/clipboard";

type SplitDirection = "up" | "down" | "left" | "right";

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
 * Split the pane in the given direction, spawning a new pane of the same type
 * that inherits the source pane's meta (view, controller, cwd, connection, etc.).
 * Agent panes strip agent-specific fields so the new pane shows the agent picker.
 */
async function handleSplitPane(blockData: Block, direction: SplitDirection): Promise<void> {
    const sourceConn = blockData.meta?.connection;
    const meta: Record<string, unknown> = { ...(blockData.meta ?? {}) };
    // Only inherit connection for non-local connections (SSH/WSL).
    // Local terminals have no connection field — setting it to "local"
    // triggers the connection overlay and shows "Disconnected".
    if (!sourceConn || sourceConn === "local") {
        delete meta["connection"];
    }
    // A view can declare meta a split must not copy (`splitDropsMeta`, Pane
    // Tab contract Phase 5) — the agent pane drops its agent-specific fields
    // so the new pane shows the picker instead of re-launching the same agent.
    for (const key of paneTabCapability(blockData.meta?.view, "splitDropsMeta") ?? []) {
        delete meta[key];
    }
    const blockDef: BlockDef = { meta };

    try {
        switch (direction) {
            case "up":
                await createBlockSplitVertically(blockDef, blockData.oid, "before");
                break;
            case "down":
                await createBlockSplitVertically(blockDef, blockData.oid, "after");
                break;
            case "left":
                await createBlockSplitHorizontally(blockDef, blockData.oid, "before");
                break;
            case "right":
                await createBlockSplitHorizontally(blockDef, blockData.oid, "after");
                break;
        }
    } catch (e) {
        console.error("[pane-actions] split failed:", e);
    }
}

// ─── Replace With submenu ─────────────────────────────────────────────────────

/**
 * Build the "Replace With..." submenu entry listing all pane-based widgets
 * (grouped widgets — e.g. the Messengers group's Discord/Slack/etc. — nest
 * under their parent's own label rather than each showing up individually;
 * see buildPaneWidgetMenuItems). Returns a one-item array, or an empty array
 * if no replacement widgets are available. Separators between sections are
 * added by buildPaneContextMenu, not here.
 */
function buildReplaceSubmenu(blockData: Block): ContextMenuItem[] {
    const fullConfig = atoms.fullConfigAtom();
    const wmap = fullConfig?.widgets ?? {};
    const settings = fullConfig?.settings ?? {};
    const items = buildPaneWidgetMenuItems(
        wmap,
        settings,
        (blockdef) => void replaceBlock(blockData.oid, blockdef, true),
        { excludeView: blockData?.meta?.view }
    );

    if (items.length === 0) return [];
    return [{ label: "Replace With...", type: "submenu" as const, submenu: items }];
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
    | "split" // Split Up / Down / Left / Right
    | "replace" // Replace With...
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
 * them omitted, or nothing applies — e.g. no replacement widgets) are skipped,
 * so no combination can leave a leading, trailing, or doubled separator:
 *
 *   [clipboard] ─ [split] ─ [replace] ─ [magnify, close] ─ [inspect]
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
                click: () => {
                    void (async () => {
                        try {
                            const text = await clipboardReadText();
                            if (!text) return;
                            viewModel?.paste?.(text);
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
              { label: "Split Up", click: () => void handleSplitPane(blockData, "up") },
              { label: "Split Down", click: () => void handleSplitPane(blockData, "down") },
              { label: "Split Left", click: () => void handleSplitPane(blockData, "left") },
              { label: "Split Right", click: () => void handleSplitPane(blockData, "right") },
          ]
        : [];

    const replace: ContextMenuItem[] = has("replace") ? buildReplaceSubmenu(blockData) : [];

    const paneActions: ContextMenuItem[] = [
        ...(has("magnify")
            ? [{ label: opts.magnified ? "Un-Magnify Pane" : "Magnify Pane", click: opts.onMagnifyToggle }]
            : []),
        ...(has("close") ? [{ label: "Close Pane", click: opts.onClose }] : []),
    ];

    // Inspect Element — opens CEF DevTools focused on whatever was
    // under the right-click. Only appears when `inspectAt` was supplied
    // (call sites that don't capture click coords don't get the entry).
    // Available in any build; the DevTools window itself decides whether
    // to launch based on the app's debug flags.
    const inspect: ContextMenuItem[] =
        has("inspect") && opts.inspectAt
            ? [
                  {
                      label: "Inspect Element",
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

    return joinMenuGroups([clipboard, split, replace, paneActions, inspect]);
}
