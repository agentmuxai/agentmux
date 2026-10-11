// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, openLink } from "./global";
import {
    readAttachments as clipboardReadAttachments,
    readText as clipboardReadText,
    writeText as clipboardWriteText,
} from "@/util/clipboard";
import * as util from "@/util/util";

class ContextMenuModelType {
    handlers: Map<string, () => void> = new Map(); // id -> handler

    constructor() {}

    // Must be called from app-init.ts:initApp() after setupCefApi() has installed window.api.
    // Calling getApi() here (module level) would crash before window.api exists.
    init() {
        getApi().onContextMenuClick(this.handleContextMenuClick.bind(this));
    }

    handleContextMenuClick(id: string): void {
        const handler = this.handlers.get(id);
        if (handler) {
            handler();
        }
    }

    _convertAndRegisterMenu(menu: ContextMenuItem[]): NativeContextMenuItem[] {
        const nativeMenuItems: NativeContextMenuItem[] = [];
        for (const item of menu) {
            const nativeItem: NativeContextMenuItem = {
                role: item.role,
                type: item.type,
                label: item.label,
                sublabel: item.sublabel,
                id: crypto.randomUUID(),
                checked: item.checked,
                swatchColor: item.swatchColor,
            };
            if (item.visible === false) {
                nativeItem.visible = false;
            }
            if (item.enabled === false) {
                nativeItem.enabled = false;
            }
            if (item.click) {
                this.handlers.set(nativeItem.id, item.click);
            }
            if (item.submenu) {
                nativeItem.submenu = this._convertAndRegisterMenu(item.submenu);
            }
            nativeMenuItems.push(nativeItem);
        }
        return nativeMenuItems;
    }

    showContextMenu(menu: ContextMenuItem[], ev: MouseEvent | { stopPropagation(): void }): void {
        ev.stopPropagation();
        this.handlers.clear();
        const nativeMenuItems = this._convertAndRegisterMenu(menu);
        const position = { x: Math.round((ev as MouseEvent).clientX), y: Math.round((ev as MouseEvent).clientY) };
        getApi().showContextMenu(atoms.workspace()?.oid, nativeMenuItems, position);
    }
}

const ContextMenuModel = new ContextMenuModelType();

// ── Shared Cut/Copy/Paste text-input menu ────────────────────────────────
//
// Any pane body (block/blockframe.tsx's onBodyContextMenu) intercepts
// right-click before it can reach the app-root fallback in app.tsx, and
// replaces it with a generic Copy-on-selection-only menu that never offers
// Paste (except for `view: "term"` panes). That leaves every <input>/
// <textarea> living directly in a pane body (not inside a portalled Modal,
// which escapes the pane body entirely) with no way to paste via right-click.
// This is the fix: attach `onContextMenu={showTextInputContextMenu}` directly
// on the element. `stopPropagation` keeps the event from ever reaching
// blockframe's handler, so this always wins for the element it's attached
// to, matching what already works correctly for modal inputs.
function isContentEditableBeingEdited(): boolean {
    const activeElement = document.activeElement;
    return (
        activeElement != null &&
        activeElement.getAttribute("contenteditable") !== null &&
        activeElement.getAttribute("contenteditable") !== "false"
    );
}

function canEnablePaste(): boolean {
    const activeElement = document.activeElement;
    return activeElement?.tagName === "INPUT" || activeElement?.tagName === "TEXTAREA" || isContentEditableBeingEdited();
}

// window.getSelection() doesn't see a selection inside an <input> or
// <textarea> in Chromium, so read the focused input's own selection.
function canEnableCopy(): boolean {
    return !util.isBlank(selectedTextIn(document.activeElement));
}

function canEnableCut(): boolean {
    if (document.activeElement?.classList.contains("xterm-helper-textarea")) {
        return false;
    }
    return !util.isBlank(selectedTextIn(document.activeElement)) && canEnablePaste();
}

/** The text an input's (or the page's) selection covers. */
function selectedTextIn(el: Element | null): string {
    if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
        const start = el.selectionStart ?? 0;
        const end = el.selectionEnd ?? start;
        return el.value.slice(start, end);
    }
    return window.getSelection()?.toString() ?? "";
}

/**
 * Replace `el`'s selection with `text` the way a real paste would.
 * `execCommand("insertText")` is deprecated, but it is the one API that
 * records the edit in Chromium's native undo history, and AgentMux only runs
 * on Chromium; `setRangeText` + an `input` event is the fallback.
 */
function insertIntoInput(el: HTMLElement | null, text: string): void {
    if (!el) return;
    el.focus();
    if (typeof document.execCommand === "function" && document.execCommand("insertText", false, text)) return;
    if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
        const start = el.selectionStart ?? el.value.length;
        const end = el.selectionEnd ?? start;
        el.setRangeText(text, start, end, "end");
        el.dispatchEvent(new Event("input", { bubbles: true }));
    }
}

async function getClipboardURL(): Promise<URL | null> {
    try {
        const clipboardText = await clipboardReadText();
        if (clipboardText == null) {
            return null;
        }
        const url = new URL(clipboardText);
        if (!url.protocol.startsWith("http")) {
            return null;
        }
        return url;
    } catch (e) {
        return null;
    }
}

/**
 * @param leadingItems caller-supplied items rendered ABOVE the standard
 * Cut/Copy/Paste block (separated from it) — e.g. the agent composer's
 * "Undo" (AgentFooter.tsx). When provided, the menu shows even if none of
 * the standard items are enabled (an empty composer still offers Undo).
 */
export interface TextInputMenuOptions {
    /**
     * Also take files and image data from the clipboard on Paste (the agent
     * composer's image attachments). Receives their paths; the text part is
     * still inserted as usual.
     */
    onPasteAttachments?: (paths: string[]) => void;
}

async function showTextInputContextMenu(
    e: MouseEvent,
    leadingItems?: ContextMenuItem[],
    opts?: TextInputMenuOptions,
): Promise<void> {
    e.preventDefault();
    e.stopPropagation();
    const canPaste = canEnablePaste();
    const canCopy = canEnableCopy();
    const canCut = canEnableCut();
    const clipboardURL = await getClipboardURL();
    if (!canPaste && !canCopy && !canCut && !clipboardURL && !leadingItems?.length) {
        return;
    }
    let menu: ContextMenuItem[] = [];
    if (leadingItems?.length) {
        menu.push(...leadingItems);
        if (canCut || canCopy || canPaste || clipboardURL) {
            menu.push({ type: "separator" });
        }
    }
    // The JS context menu never executes an item's `role` (only `click`), so
    // Cut/Copy/Paste do the work themselves, against the element that had
    // focus when the menu opened: clicking a menu row can move focus away.
    const target = document.activeElement as HTMLElement | null;
    if (canCut) {
        menu.push({
            label: "Cut",
            role: "cut",
            click: () => {
                const text = selectedTextIn(target);
                void clipboardWriteText(text).then(() => {
                    target?.focus();
                    if (typeof document.execCommand === "function" && document.execCommand("delete")) return;
                    insertIntoInput(target, "");
                });
            },
        });
    }
    if (canCopy) {
        menu.push({
            label: "Copy",
            role: "copy",
            click: () => void clipboardWriteText(selectedTextIn(target)),
        });
    }
    if (canPaste) {
        menu.push({
            label: "Paste",
            role: "paste",
            click: () => {
                const onPasteAttachments = opts?.onPasteAttachments;
                if (onPasteAttachments) {
                    void clipboardReadAttachments()
                        .then(({ text, paths }) => {
                            if (text) insertIntoInput(target, text);
                            if (paths.length > 0) onPasteAttachments(paths);
                        })
                        .catch(() => {});
                    return;
                }
                void clipboardReadText()
                    .then((text) => {
                        if (text) insertIntoInput(target, text);
                    })
                    .catch(() => {});
            },
        });
    }
    if (clipboardURL) {
        menu.push({ type: "separator" });
        menu.push({
            label: "Open Clipboard URL (" + clipboardURL.hostname + ")",
            click: () => {
                openLink(clipboardURL.toString());
            },
        });
    }
    ContextMenuModel.showContextMenu(menu, e);
}

/** One "Copy <thing>" entry for {@link showCopyContextMenu}. */
export interface CopyMenuEntry {
    /** Menu label, e.g. "Copy command". */
    label: string;
    /** Text written to the clipboard. An empty/nullish value drops the entry. */
    value: string | null | undefined;
}

/**
 * Right-click menu of "Copy <thing>" entries for a row/node whose text cannot be
 * selected (`user-select: none` — the Swarm rows and Drone nodes), so the pane
 * body's generic Copy-on-selection menu can never fire for them.
 *
 * Empty values are dropped. If nothing is left the event is NOT consumed, so it
 * still reaches the pane's own menu instead of swallowing the right-click for
 * an empty menu. Otherwise the event is claimed (preventDefault +
 * stopPropagation) so the pane menu does not also appear.
 *
 * Returns whether a menu was shown.
 */
function showCopyContextMenu(entries: CopyMenuEntry[], e: MouseEvent): boolean {
    const menu: ContextMenuItem[] = [];
    for (const { label, value } of entries) {
        if (!value) continue;
        menu.push({ label, click: () => void clipboardWriteText(value) });
    }
    if (menu.length === 0) return false;
    e.preventDefault();
    ContextMenuModel.showContextMenu(menu, e);
    return true;
}

export { ContextMenuModel, showCopyContextMenu, showTextInputContextMenu };
