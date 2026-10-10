// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { keyPlatform } from "@/app/keybindings";
import {
    lastResolvedCommand,
    listShortcuts,
    noteResolved,
    paneRefusal,
    planKeyPress,
    runCommand,
    type KeyPressPlan,
    type RunResult,
} from "@/app/keybindings/app-api";
import type { KeyEventLike } from "@/app/keybindings/keys";
import { chordLeaderOf, commandForKey, DOC_TAB_HOSTS, resolveKey, type KeyContext, type ResolvedBinding } from "@/app/keybindings/registry";
import { commandRegistry } from "@/app/store/command-registry";
import { atoms, getApi, getBlockComponentModel, setControlShiftDelayAtom } from "@/app/store/global";
import { getLayoutModelForStaticTab } from "@/layout/index";
import { revealBlockLocally } from "@/app/util/reveal-block";
import { isEditableTarget } from "@/util/focusutil";
import * as keyutil from "@/util/keyutil";
import { CHORD_TIMEOUT } from "@/util/sharedconst";
import { createSignal } from "solid-js";

export type KeyHandler = (event: MuxKeyboardEvent) => boolean;

const [simpleControlShift, setSimpleControlShift] = createSignal(false);
/** What each command in the shortcut table (keybindings/defaults.ts) does
 *  when its key is pressed. Filled by registerGlobalKeys (keymodel.ts); a
 *  command without an entry runs from the command registry. */
export const keyCommands = new Map<string, KeyHandler>();
let globalKeybindingsDisabled = false;

// track current chord state and timeout (for resetting)
let activeChord: string | null = null;
let chordTimeout: NodeJS.Timeout = null;

function resetChord() {
    activeChord = null;
    if (chordTimeout) {
        clearTimeout(chordTimeout);
        chordTimeout = null;
    }
}

function setActiveChord(activeChordArg: string) {
    if (chordTimeout) {
        clearTimeout(chordTimeout);
    }
    activeChord = activeChordArg;
    chordTimeout = setTimeout(() => resetChord(), CHORD_TIMEOUT);
}

/** A chord's first key was pressed and its second is awaited. */
export function isChordActive(): boolean {
    return activeChord != null;
}

export function keyboardMouseDownHandler(e: MouseEvent) {
    if (!e.ctrlKey || !e.shiftKey) {
        unsetControlShift();
    }
}

function setControlShift() {
    setSimpleControlShift(true);
    setTimeout(() => {
        if (simpleControlShift()) {
            setControlShiftDelayAtom(true);
        }
    }, 400);
}

function unsetControlShift() {
    setSimpleControlShift(false);
    setControlShiftDelayAtom(false);
}

export function disableGlobalKeybindings() {
    globalKeybindingsDisabled = true;
}

export function enableGlobalKeybindings() {
    globalKeybindingsDisabled = false;
}

function shouldDispatchToBlock(e: MuxKeyboardEvent): boolean {
    if (atoms.modalOpen()) {
        return false;
    }
    const activeElem = document.activeElement;
    if (activeElem != null && activeElem instanceof HTMLElement) {
        if (activeElem.tagName == "INPUT" || activeElem.tagName == "TEXTAREA" || activeElem.contentEditable == "true") {
            if (activeElem.classList.contains("dummy-focus") || activeElem.classList.contains("dummy")) {
                return true;
            }
            if (keyutil.isInputEvent(e)) {
                return false;
            }
            return true;
        }
    }
    return true;
}

let lastHandledEvent: KeyboardEvent | null = null;

/** Focus is somewhere the user types. Not the terminal: its hidden textarea
 *  is focus plumbing, and the terminal has its own context (terminalFocus). */
export function isTypingFocus(el: Element | null = document.activeElement): boolean {
    if (!(el instanceof HTMLElement)) return false;
    if (el.classList.contains("xterm-helper-textarea")) return false;
    if (el.classList.contains("dummy-focus") || el.classList.contains("dummy")) return false;
    return isEditableTarget(el);
}

/** Focus is in a terminal (its hidden textarea, or anywhere inside xterm). */
export function isTerminalFocus(el: Element | null = document.activeElement): boolean {
    return el instanceof HTMLElement && (el.classList.contains("xterm-helper-textarea") || el.closest(".xterm") != null);
}

function focusedViewType(): string {
    const blockId = getLayoutModelForStaticTab()?.focusedNode?.()?.data?.blockId;
    return (blockId && getBlockComponentModel(blockId)?.viewModel?.viewType) || "";
}

/** Where focus is, for the shortcut table's `when` clauses. */
export function currentKeyContext(): KeyContext {
    const el = document.activeElement;
    const viewType = focusedViewType();
    return {
        textInputFocus: isTypingFocus(el),
        terminalFocus: isTerminalFocus(el),
        viewType,
        docTabsHost: DOC_TAB_HOSTS.includes(viewType),
    };
}

function keyEventLike(muxEvent: MuxKeyboardEvent): KeyEventLike {
    const native = (muxEvent as { nativeEvent?: KeyboardEvent }).nativeEvent;
    return {
        key: muxEvent.key,
        code: muxEvent.code,
        ctrlKey: !!muxEvent.control,
        shiftKey: !!muxEvent.shift,
        altKey: !!muxEvent.alt,
        metaKey: !!muxEvent.meta,
        getModifierState: native?.getModifierState?.bind(native),
    };
}

/** The table binding a key press resolves to in `ctx`, without running it. */
export function resolveKeyEvent(muxEvent: MuxKeyboardEvent, ctx: KeyContext): ResolvedBinding | null {
    return resolveKey(keyEventLike(muxEvent), ctx, keyPlatform());
}

/** Runs a command's key handler; false when it didn't apply. */
export function runKeyCommand(command: string, muxEvent: MuxKeyboardEvent): boolean {
    const handler = keyCommands.get(command);
    if (handler) return handler(muxEvent) === true;
    return commandRegistry.run(command);
}

export function appHandleKeyDown(muxEvent: MuxKeyboardEvent): boolean {
    if (globalKeybindingsDisabled) {
        return false;
    }
    const nativeEvent = (muxEvent as any).nativeEvent;
    // A pane or the code editor already handled this key (it called
    // preventDefault). Solid's delegated onKeyDown and this handler both
    // listen on `document`, so a pane's stopPropagation can't stop us; this
    // check is what keeps one key press from also running a global shortcut.
    if (nativeEvent?.defaultPrevented) {
        return false;
    }
    if (lastHandledEvent != null && nativeEvent != null && lastHandledEvent === nativeEvent) {
        return false;
    }
    lastHandledEvent = nativeEvent;
    const ctx = currentKeyContext();
    const platform = keyPlatform();
    const ev = keyEventLike(muxEvent);
    if (activeChord) {
        // The second key of a chord: run its binding, or consume the key.
        const leader = activeChord;
        resetChord();
        const second = resolveKey(ev, ctx, platform, leader);
        if (second) {
            noteResolved(second.row.command, "global");
            runKeyCommand(second.row.command, muxEvent);
        }
        return true;
    }
    const resolved = resolveKey(ev, ctx, platform);
    if (resolved && !resolved.chordStart) noteResolved(resolved.row.command, "global");
    if (resolved?.chordStart) {
        setActiveChord(chordLeaderOf(ev, platform));
        return true;
    }
    if (resolved && runKeyCommand(resolved.row.command, muxEvent)) {
        return true;
    }
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    const blockId = focusedNode?.data?.blockId;
    if (blockId != null && shouldDispatchToBlock(muxEvent)) {
        const bcm = getBlockComponentModel(blockId);
        const viewModel = bcm?.viewModel;
        if (viewModel?.keyDownHandler) {
            const handledByBlock = viewModel.keyDownHandler(muxEvent);
            if (handledByBlock) {
                return true;
            }
        }
    }
    return false;
}

/** Runs the app shortcuts the host forwards out of a focused browser pane,
 *  whose keys never reach this document (crates/cef `forward_app_shortcut`,
 *  keys from keybindings/host-keys.json). */
export function registerHostShortcuts() {
    void getApi().listen<{ block_id: string; command: string; key?: string }>("app-shortcut", (payload) => {
        // Same rule as a key pressed in the app (the command palette turns
        // global shortcuts off while it's open).
        if (globalKeybindingsDisabled) return;
        // The host knows only the default keys: run what this key does in the
        // effective table, so the user's unbinds and remaps apply here too.
        const command = payload.key ? commandForKey(payload.key, keyPlatform()) : payload.command;
        if (!command) return;
        // The browser pane holds OS keyboard focus; take it back first, or a
        // command that opens something to type in (the palette) shows while
        // the keys still go to the page. DOM focus alone doesn't move it.
        const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
        void getApi()
            .reclaimWindowFocus(label)
            .catch(() => {})
            .finally(() => runKeyCommand(command, { repeat: false } as MuxKeyboardEvent));
    });
}

/** Shows the numbered pane overlay while Ctrl+Shift is held (after a short
 *  delay, so a quick Ctrl+Shift+key chord doesn't flash it). It used to wait
 *  for a `control-shift-state-update` event from the host that nothing ever
 *  sent, so the overlay never appeared. */
export function registerControlShiftTracking() {
    const update = (e: KeyboardEvent) => {
        if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey) {
            if (!simpleControlShift()) setControlShift();
        } else if (simpleControlShift()) {
            unsetControlShift();
        }
    };
    document.addEventListener("keydown", update, true);
    document.addEventListener("keyup", update, true);
    window.addEventListener("blur", () => unsetControlShift());
}

/** A key event for running a command without a key press (RunCommand). */
const NO_KEY: MuxKeyboardEvent = { type: "keydown", key: "", code: "", repeat: false };

/**
 * Focuses a pane for the App API, as a click would: layout focus and the
 * caret. `blockId` may be any pane in this window, in any tab, or one of a
 * pane's tabs: the window switches to that tab, the pane shows that tab, and
 * a minimized or hidden pane is brought back (revealBlockLocally; the kinks
 * plan's D1, A2). It never reaches another window. False if no tab of this
 * window holds `blockId`.
 */
export async function focusPaneForApi(blockId: string): Promise<boolean> {
    return revealBlockLocally(blockId);
}

/** Whether the caret is in `blockId`'s pane: in any element of that block, since a pane tab's content sits in its own block. */
export function caretInBlock(blockId: string): boolean {
    const caret = document.activeElement;
    if (!caret) return false;
    return [...document.querySelectorAll("[data-blockid]")].some((el) => el.getAttribute("data-blockid") === blockId && el.contains(caret));
}

/** Resolves once the caret is in `blockId`, or false after `ms`: focus can land a few frames late (giveBlockFocus retries). */
async function caretArrives(blockId: string, ms = 500): Promise<boolean> {
    for (const until = Date.now() + ms; ; ) {
        if (caretInBlock(blockId)) return true;
        if (Date.now() >= until) return false;
        await new Promise((r) => setTimeout(r, 16));
    }
}

/**
 * Publishes the shortcut table to the host as `window.__agentmux_shortcuts`,
 * for the App API's ListShortcuts, RunCommand and PressKeys
 * (keybindings/app-api.ts). The host reaches it only in the window that
 * holds the calling agent's pane.
 */
export function installShortcutApi() {
    const focusedBlockId = (): string | null => getLayoutModelForStaticTab().focusedNode?.()?.data?.blockId ?? null;
    const deps = () => ({
        platform: keyPlatform(),
        focusedBlockId,
        focusBlock: focusPaneForApi,
        runGlobal: (command: string): boolean => runKeyCommand(command, NO_KEY),
    });
    (window as unknown as { __agentmux_shortcuts: unknown }).__agentmux_shortcuts = {
        list: () => listShortcuts(keyPlatform()),
        run: (command: string, target?: string): RunResult | Promise<RunResult> => runCommand(command, target || undefined, deps()),
        // PressKeys: what to send, after focusing `target`. The host then
        // sends the events and reads `last()`.
        plan: async (keys: string, target?: string): Promise<KeyPressPlan | { reason: string }> => {
            if (target && !(await focusPaneForApi(target))) return { reason: `pane ${target} is not in this window` };
            // A key goes where the caret is: refuse rather than press it into another pane.
            if (target && !(await caretArrives(target))) return { reason: `pane ${target} didn't take keyboard focus` };
            const plan = planKeyPress(keys, keyPlatform());
            // The pane's own guard (the terminal's paste guard) sees the key too.
            const refused = "commands" in plan ? await paneRefusal(target ?? focusedBlockId(), plan.commands) : undefined;
            return refused ? { reason: refused } : plan;
        },
        focus: (blockId: string): Promise<boolean> => focusPaneForApi(blockId),
        focused: focusedBlockId,
        last: () => lastResolvedCommand(),
    };
}
