// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Right-click clipboard items for the agent pane's Shell drawer.
 *
 * The generic pane menu's Copy/Paste read `viewModel.termRef`, which the agent
 * ViewModel does not have (the drawer's xterm belongs to a headless sub-block),
 * so the drawer supplies its own via a context-menu region — see
 * docs/specs/SPEC_AGENT_SHELL_DRAWER_CONTEXT_MENU_PASTE_AND_REGIONS_2026_09_25.md §3.
 */

/** Largest clipboard the drawer's Paste will send. Above this the paste is
 *  refused with an explanation rather than streamed: a shell is the wrong
 *  place for megabytes of text (the line editor re-renders it, scrollback
 *  churns, and the chunked send takes long enough to block typing), and a
 *  file the shell can `cat`/reference is the better tool. */
export const SHELL_PASTE_MAX_BYTES = 1024 * 1024;

export interface ShellDrawerMenuDeps {
    /** The live xterm, or undefined while loading / after a failed start. */
    getTerminal: () => { getSelection(): string; paste(text: string): void } | undefined;
    /** True while an agent holds the shell — human input is dropped then. */
    isAgentLocked: () => boolean;
    readClipboard: () => Promise<string | null | undefined>;
    writeClipboard: (text: string) => Promise<void>;
    notify: (n: NotificationType) => void;
}

/** "1 MB" / "3.2 MB" / "512 KB" — whole numbers when exact, else one decimal. */
export function formatSize(bytes: number): string {
    const fmt = (v: number, unit: string) => `${Number.isInteger(v) ? v : v.toFixed(1)} ${unit}`;
    if (bytes >= 1024 * 1024) return fmt(bytes / (1024 * 1024), "MB");
    return fmt(Math.max(1, Math.round((bytes / 1024) * 10) / 10), "KB");
}

export function buildShellDrawerClipboardItems(deps: ShellDrawerMenuDeps): ContextMenuItem[] {
    const terminal = deps.getTerminal();
    // Captured now, at right-click time — the selection can't change between
    // the menu opening and the click.
    const selection = terminal?.getSelection() ?? "";
    const locked = deps.isAgentLocked();

    // The reason/limit rides in the LABEL, not `sublabel`: the JS-rendered
    // context menu (cef-api.ts showJsContextMenu) does not draw sublabels at
    // all, so a sublabel here would be invisible. A greyed-out item then still
    // says why.
    const pasteLabel = locked
        ? "Paste (agent is using this shell)"
        : `Paste (up to ${formatSize(SHELL_PASTE_MAX_BYTES)})`;

    return [
        {
            label: "Copy",
            enabled: selection.length > 0,
            click: () => {
                if (selection) deps.writeClipboard(selection).catch(console.error);
            },
        },
        {
            label: pasteLabel,
            // Disabled while the agent is driving the shell: sendDataHandler
            // would silently drop the input, which would look like a broken menu.
            enabled: terminal != null && !locked,
            click: () => {
                void (async () => {
                    try {
                        const text = await deps.readClipboard();
                        if (!text) return;
                        const bytes = new TextEncoder().encode(text).length;
                        if (bytes > SHELL_PASTE_MAX_BYTES) {
                            // Say what happened, the limit, and what to do
                            // instead — a silent no-op reads as a bug.
                            deps.notify({
                                icon: "clipboard",
                                title: "Paste too large for the shell",
                                message:
                                    `The clipboard is ${formatSize(bytes)}; the shell accepts up to ` +
                                    `${formatSize(SHELL_PASTE_MAX_BYTES)}. Save it to a file and reference ` +
                                    `it from the shell instead.`,
                                timestamp: new Date().toISOString(),
                                type: "warning",
                            });
                            return;
                        }
                        // terminal.paste (not a raw write): applies bracketed-paste
                        // wrapping when the shell enabled it, then flows out through
                        // the drawer's sendDataHandler like typed input.
                        deps.getTerminal()?.paste(text);
                    } catch (e) {
                        console.error("[shell-drawer] paste failed:", e);
                    }
                })();
            },
        },
    ];
}
