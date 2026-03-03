// Copyright 2026, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane context menu actions: copy, paste, split, open in VSCode, and shared menu builder.
 * Used from both handleHeaderContextMenu (header) and onContextMenu (body) in blockframe.tsx.
 */

import { createBlockSplitHorizontally, createBlockSplitVertically, getApi } from "@/app/store/global";

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

/**
 * Split the pane in the given direction, spawning a new terminal
 * that inherits the source pane's cwd and connection.
 */
export async function handleSplitPane(blockData: Block, direction: SplitDirection): Promise<void> {
    const sourceConn = blockData.meta?.connection;
    const blockDef: BlockDef = {
        meta: {
            view: "term",
            controller: "shell",
            // Only inherit connection for non-local connections (SSH/WSL).
            // Local terminals have no connection field — setting it to "local"
            // triggers the connection overlay and shows "Disconnected".
            ...(sourceConn && sourceConn !== "local" ? { connection: sourceConn } : {}),
            "cmd:cwd": blockData.meta?.["cmd:cwd"] ?? "",
        },
    };

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

/**
 * Open the pane's working directory in VS Code using the vscode:// URI scheme.
 * Falls back to opening VS Code's welcome screen if no cwd is available.
 */
export function handleOpenInVSCode(blockData: Block): void {
    const cwd = blockData.meta?.["cmd:cwd"];
    if (cwd) {
        // Normalize backslashes to forward slashes for the vscode:// URI
        const normalized = cwd.replace(/\\/g, "/");
        getApi().openExternal(`vscode://file/${encodeURI(normalized)}`);
    } else {
        getApi().openExternal("vscode://");
    }
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
                    navigator.clipboard.writeText(selection).catch(console.error);
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
                                  const text = await navigator.clipboard.readText();
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
        { label: "Open in VSCode", click: () => handleOpenInVSCode(blockData) },
        { type: "separator" },
        {
            label: opts.magnified ? "Un-Magnify Block" : "Magnify Block",
            click: opts.onMagnifyToggle,
        },
        { label: "Close Block", click: opts.onClose },
    ];
}
