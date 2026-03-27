// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane context menu actions: copy, paste, split, and shared menu builder.
 * Used from both handleHeaderContextMenu (header) and onContextMenu (body) in blockframe.tsx.
 */

import { atoms, createBlockSplitHorizontally, createBlockSplitVertically, replaceBlock } from "@/app/store/global";
import { readText as clipboardReadText, writeText as clipboardWriteText } from "@/util/clipboard";

type SplitDirection = "up" | "down" | "left" | "right";

// ─── Copy / Paste helpers ─────────────────────────────────────────────────────

/**
 * Get the current text selection for a pane.
 * - Terminal: uses xterm.js's own selection (reliable after right-click).
 * - Other panes: falls back to browser window.getSelection().
 */
function getPaneSelection(viewModel?: ViewModel): string {
    // TermViewModel exposes termRef.current.terminal (xterm.js Terminal instance).
    // xterm maintains its own selection model independently of browser focus,
    // so getSelection() is reliable even after a right-click clears browser selection.
    const termSel = (viewModel as any)?.termRef?.current?.terminal?.getSelection?.();
    if (typeof termSel === "string") return termSel;
    return window.getSelection()?.toString() ?? "";
}

/**
 * Returns true if the pane accepts text input (i.e. paste makes sense).
 * Currently only terminal panes accept input via the PTY.
 */
function paneAcceptsInput(blockData: Block): boolean {
    return blockData.meta?.view === "term";
}

// ─── Split ────────────────────────────────────────────────────────────────────

// Agent-specific meta fields that should NOT be inherited when splitting.
// A split agent pane should open the picker, not re-launch the same agent.
const agentInheritBlocklist = new Set([
    "agentId",
    "agentName",
    "agentIcon",
    "agentMode",
    "agentProvider",
    "agentCliPath",
    "agentCliArgs",
    "agentOutputFormat",
    "agentBinDir",
    "cmd",
    "cmd:args",
    "cmd:interactive",
    "cmd:runonstart",
]);

/**
 * Split the pane in the given direction, spawning a new pane of the same type
 * that inherits the source pane's meta (view, controller, cwd, connection, etc.).
 * Agent panes strip agent-specific fields so the new pane shows the agent picker.
 */
export async function handleSplitPane(blockData: Block, direction: SplitDirection): Promise<void> {
    const sourceConn = blockData.meta?.connection;
    const meta: Record<string, unknown> = { ...(blockData.meta ?? {}) };
    // Only inherit connection for non-local connections (SSH/WSL).
    // Local terminals have no connection field — setting it to "local"
    // triggers the connection overlay and shows "Disconnected".
    if (!sourceConn || sourceConn === "local") {
        delete meta["connection"];
    }
    // Agent panes: drop all agent-specific fields so the new pane shows
    // the agent picker instead of re-launching the same agent session.
    if (blockData.meta?.view === "agent") {
        for (const key of agentInheritBlocklist) {
            delete meta[key];
        }
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

/** Non-pane widget views that should be excluded from the Replace submenu. */
const nonPaneViews = new Set(["devtools", "settings"]);

/**
 * Build a "Replace With..." submenu listing all pane-based widgets.
 * Returns an array with the submenu item + a trailing separator, or empty
 * array if no replacement widgets are available.
 */
function buildReplaceSubmenu(blockData: Block): ContextMenuItem[] {
    const fullConfig = atoms.fullConfigAtom();
    const widgets = fullConfig?.widgets ?? {};
    const currentView = blockData?.meta?.view;

    const items: ContextMenuItem[] = Object.values(widgets)
        .filter((w) => {
            const view = w.blockdef?.meta?.view;
            if (!view || nonPaneViews.has(view)) return false;
            if (view === currentView) return false;
            return true;
        })
        .sort((a, b) => {
            const orderA = a["display:order"] ?? 0;
            const orderB = b["display:order"] ?? 0;
            if (orderA !== orderB) return orderA - orderB;
            return (a.label ?? "").localeCompare(b.label ?? "");
        })
        .map((widget) => ({
            label: widget.label ?? "Unnamed",
            click: () => void replaceBlock(blockData.oid, widget.blockdef, true),
        }));

    if (items.length === 0) return [];
    return [
        { label: "Replace With...", type: "submenu" as const, submenu: items },
        { type: "separator" as const },
    ];
}

// ─── Menu builder ─────────────────────────────────────────────────────────────

export interface PaneContextMenuOpts {
    magnified: boolean;
    onMagnifyToggle: () => void;
    onClose: () => void;
}

/**
 * Build the reusable pane context menu items shared between header and body right-click.
 * Pass viewModel to enable terminal-aware copy/paste.
 */
export function buildPaneContextMenu(
    blockData: Block,
    opts: PaneContextMenuOpts,
    viewModel?: ViewModel
): ContextMenuItem[] {
    const selection = getPaneSelection(viewModel);
    const hasSelection = selection.length > 0;
    const canPaste = paneAcceptsInput(blockData);

    return [
        // Copy — always present; disabled when nothing is selected
        {
            label: "Copy",
            enabled: hasSelection,
            click: () => {
                if (selection) {
                    clipboardWriteText(selection).catch(console.error);
                }
            },
        },
        // Paste — only shown for input-accepting panes (terminals)
        ...(canPaste
            ? [
                  {
                      label: "Paste",
                      click: () => {
                          void (async () => {
                              try {
                                  const text = await clipboardReadText();
                                  if (!text) return;
                                  const terminal = (viewModel as any)?.termRef?.current?.terminal;
                                  if (terminal) {
                                      terminal.paste(text);
                                  }
                              } catch (e) {
                                  console.error("[pane-actions] paste failed:", e);
                              }
                          })();
                      },
                  } as ContextMenuItem,
              ]
            : []),
        { type: "separator" },
        { label: "Split Up",    click: () => void handleSplitPane(blockData, "up") },
        { label: "Split Down",  click: () => void handleSplitPane(blockData, "down") },
        { label: "Split Left",  click: () => void handleSplitPane(blockData, "left") },
        { label: "Split Right", click: () => void handleSplitPane(blockData, "right") },
        { type: "separator" },
        ...buildReplaceSubmenu(blockData),
        {
            label: opts.magnified ? "Un-Magnify Block" : "Magnify Block",
            click: opts.onMagnifyToggle,
        },
        { label: "Close Block", click: opts.onClose },
    ];
}
