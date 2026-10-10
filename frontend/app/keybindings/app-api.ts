// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The agent App API's view of the shortcut table: ListShortcuts, RunCommand and
// PressKeys (docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md §4).
// The host calls these through `window.__agentmux_shortcuts`, in the window that
// holds the calling agent's own pane; srv decides which window that is.

import type { KeyBindingRow, KeyCategory } from "./defaults";
import { formatKey, parseKey, sameKey, type KeyPlatform, type KeySpec } from "./keys";
import { effectiveRows } from "./registry";

/** A shortcut as the Help pane shows it on this platform. */
export interface ShortcutInfo {
    command: string;
    label: string;
    category: KeyCategory;
    /** Keys as the Help pane shows them ("Ctrl+Shift+D", "⌘D"). */
    keys: string[];
    /** The same keys in the table's syntax, for PressKeys ("ctrl+shift+d"). */
    raw: string[];
    when?: string;
    /** The pane whose own handler runs this command, if any. */
    pane?: string;
}

export interface RunResult {
    ran: boolean;
    /** Why a command did not run, in words a person can act on. */
    reason?: string;
}

/** What the dispatcher last resolved a key press to, for PressKeys. */
export interface ResolvedNote {
    command: string;
    /** "global" (the dispatcher) or the pane that matched its own row. */
    by: string;
    at: number;
}

/**
 * What the page refuses to run for the App API, with the reason it gives.
 * Agents are first-class owners, so this is only what has no protection: a
 * permanent delete can't be undone. `pane:close` is handled by srv, through
 * ClosePane's undo (srv `ui_shortcuts.rs`), so it never reaches the page from
 * RunCommand; here it stops PressKeys pressing a key that closes at once.
 * Everything else that closes or destroys runs with the confirmation or undo
 * a user gets (`tab:close`, `files:trash`, replacing a pane), and a pane can
 * add its own guard (`registerPaneCommandRunner`'s `refuse`).
 */
export const API_REFUSED: ReadonlyMap<string, string> = new Map([
    ["files:deletePermanently", "it can't be undone"],
    ["pane:close", "a key press closes the focused pane at once: RunCommand pane:close with a target closes it with the user's 15-second undo"],
]);

/** Commands refused on macOS only, for something macOS can't undo yet. None
 *  today: `files:trash` can be put back there too. */
const API_REFUSED_MAC: ReadonlyMap<string, string> = new Map();

/** Why the App API refuses `command` on `platform`, or undefined if it doesn't. */
export function refusalFor(command: string, platform: KeyPlatform): string | undefined {
    return API_REFUSED.get(command) ?? (platform === "mac" ? API_REFUSED_MAC.get(command) : undefined);
}

/**
 * The terminal's own commands. Their rows have no `pane` (a `when` clause
 * scopes them), but the terminal's key handler runs them, not the dispatcher.
 */
const TERMINAL_OWN = new Set(["term:copy", "term:paste", "term:clear"]);

let lastResolved: ResolvedNote | null = null;

/** A pane's own command runner: true/false, or a result when it needs to say why (or to wait). */
export type PaneRun = (command: string) => boolean | RunResult | Promise<RunResult>;
/** Why a pane won't let the App API run `command` right now, or undefined. */
export type PaneRefuse = (command: string) => Promise<string | undefined>;

const paneRunners = new Map<string, { run: PaneRun; refuse?: PaneRefuse }>();

/** Called by the dispatcher and the panes whenever a key resolves to a command. */
export function noteResolved(command: string, by: string): void {
    lastResolved = { command, by, at: Date.now() };
}

export function lastResolvedCommand(): ResolvedNote | null {
    return lastResolved;
}

/**
 * A pane registers the function that runs its own commands (`files:*`,
 * `editor:*`, `doctab:*`, `term:*`), so RunCommand reaches the same code its
 * keys do, and optionally a guard that PressKeys and RunCommand ask first
 * (the terminal's paste guard). Returns the unregister function, for the
 * pane's cleanup.
 */
export function registerPaneCommandRunner(blockId: string, run: PaneRun, refuse?: PaneRefuse): () => void {
    const entry = { run, refuse };
    paneRunners.set(blockId, entry);
    return () => {
        if (paneRunners.get(blockId) === entry) paneRunners.delete(blockId);
    };
}

/** The first refusal a pane's guard gives for any of `commands`, or undefined. */
export async function paneRefusal(blockId: string | null, commands: string[]): Promise<string | undefined> {
    const refuse = blockId ? paneRunners.get(blockId)?.refuse : undefined;
    if (!refuse) return undefined;
    for (const command of commands) {
        const why = await refuse(command);
        if (why) return `${command}: ${why}`;
    }
    return undefined;
}

export function listShortcuts(platform: KeyPlatform): ShortcutInfo[] {
    const byRow = new Map<KeyBindingRow, ShortcutInfo>();
    for (const { row, source } of effectiveRows(platform)) {
        if (row.devOnly) continue;
        let info = byRow.get(row);
        if (!info) {
            info = { command: row.command, label: row.label, category: row.category, keys: [], raw: [] };
            if (row.when) info.when = row.when;
            if (row.pane) info.pane = row.pane;
            byRow.set(row, info);
        }
        info.keys.push(formatKey(source, platform));
        info.raw.push(source);
    }
    return [...byRow.values()];
}

/** Everything `runCommand` needs from the app, so tests can stand in for it. */
export interface RunDeps {
    platform: KeyPlatform;
    /** The pane a key press would reach now (the focused pane), if any. */
    focusedBlockId: () => string | null;
    /** Focuses `blockId` in this window, switching tabs if needed; false if it isn't in this window. */
    focusBlock: (blockId: string) => boolean | Promise<boolean>;
    /** The dispatcher's own handler: true when the command applied. */
    runGlobal: (command: string) => boolean;
}

/**
 * Runs a table command as its key would: a global command through the
 * dispatcher's handler, a pane command through the target pane's own runner.
 * `target` is a pane in this window, in any tab (default: the focused pane).
 */
export function runCommand(command: string, target: string | undefined, deps: RunDeps): RunResult | Promise<RunResult> {
    const refused = refusalFor(command, deps.platform);
    if (refused) return { ran: false, reason: `${command} is not available to agents: ${refused}` };
    const info = listShortcuts(deps.platform).find((s) => s.command === command);
    if (!info) {
        return { ran: false, reason: `unknown command ${command}: ListShortcuts lists the commands` };
    }
    if (target) {
        const notHere: RunResult = { ran: false, reason: `pane ${target} is not in this window` };
        const focused = deps.focusBlock(target);
        if (typeof focused !== "boolean") return focused.then((ok) => (ok ? runFocused(command, target, info, deps) : notHere));
        if (!focused) return notHere;
    }
    return runFocused(command, target, info, deps);
}

function runFocused(command: string, target: string | undefined, info: ShortcutInfo, deps: RunDeps): RunResult | Promise<RunResult> {
    if (info.pane || TERMINAL_OWN.has(command)) {
        const blockId = target ?? deps.focusedBlockId();
        if (!blockId) return { ran: false, reason: `${command} needs a pane: none is focused` };
        const runner = paneRunners.get(blockId);
        if (!runner) return { ran: false, reason: `pane ${blockId} doesn't handle ${command}` };
        const out = runner.run(command);
        if (typeof out !== "boolean") return out;
        return out ? { ran: true } : { ran: false, reason: `${command} didn't apply in pane ${blockId} now` };
    }
    return deps.runGlobal(command) ? { ran: true } : { ran: false, reason: `${command} didn't apply in the current context` };
}

/**
 * One key event as the host sends it through the DevTools protocol
 * (`Input.dispatchKeyEvent`): `modifiers` is CDP's bit mask (Alt 1, Ctrl 2,
 * Meta/⌘ 4, Shift 8).
 */
export interface KeyEventPlan {
    key: string;
    code: string;
    keyCode: number;
    modifiers: number;
}

export interface KeyPressPlan {
    /** One event per key: two for a chord. */
    events: KeyEventPlan[];
    /** The physical modifiers of each key, by name ("ctrl", "meta"…). */
    modifiers: string[][];
    /** Every command the table binds these keys to, in some context. */
    commands: string[];
}

const CDP_ALT = 1;
const CDP_CTRL = 2;
const CDP_META = 4;
const CDP_SHIFT = 8;

/** key, code and Windows virtual-key code for each physical key the table uses. */
const CODE_EVENT: Record<string, [string, number]> = {
    Backquote: ["`", 192],
    Minus: ["-", 189],
    Equal: ["=", 187],
    BracketLeft: ["[", 219],
    BracketRight: ["]", 221],
    Backslash: ["\\", 220],
    Semicolon: [";", 186],
    Quote: ["'", 222],
    Comma: [",", 188],
    Period: [".", 190],
    Slash: ["/", 191],
    ...Object.fromEntries(Array.from({ length: 10 }, (_, n) => [`Numpad${n}`, [String(n), 96 + n]])),
    NumpadAdd: ["+", 107],
    NumpadSubtract: ["-", 109],
};

const NAMED_EVENT: Record<string, number> = {
    Enter: 13,
    Tab: 9,
    Escape: 27,
    Backspace: 8,
    Delete: 46,
    Insert: 45,
    Home: 36,
    End: 35,
    PageUp: 33,
    PageDown: 34,
    ArrowLeft: 37,
    ArrowUp: 38,
    ArrowRight: 39,
    ArrowDown: 40,
    ...Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`F${i + 1}`, 112 + i])),
};

function eventFor(k: KeySpec): KeyEventPlan | null {
    const modifiers = (k.alt ? CDP_ALT : 0) | (k.ctrl ? CDP_CTRL : 0) | (k.meta ? CDP_META : 0) | (k.shift ? CDP_SHIFT : 0);
    if (k.letter) {
        const upper = k.letter.toUpperCase();
        return { key: k.shift ? upper : k.letter, code: `Key${upper}`, keyCode: upper.charCodeAt(0), modifiers };
    }
    if (k.code?.startsWith("Digit")) {
        const digit = k.code.slice(5);
        return { key: digit, code: k.code, keyCode: digit.charCodeAt(0), modifiers };
    }
    if (k.code) {
        const ev = CODE_EVENT[k.code];
        return ev ? { key: ev[0], code: k.code, keyCode: ev[1], modifiers } : null;
    }
    if (k.named && NAMED_EVENT[k.named] != null) {
        return { key: k.named, code: k.named, keyCode: NAMED_EVENT[k.named], modifiers };
    }
    return null;
}

function modifierNames(k: KeySpec): string[] {
    return (["ctrl", "alt", "shift", "meta"] as const).filter((m) => k[m]);
}

/**
 * What PressKeys sends for `keys` (the table's syntax; a chord is two keys
 * separated by a space), or why it won't. Only keys the table binds are
 * accepted, so PressKeys can't type arbitrary shortcuts; and a key bound to
 * a refused command anywhere in the table is refused, whatever has focus.
 */
export function planKeyPress(keys: string, platform: KeyPlatform): KeyPressPlan | { reason: string } {
    const parts = keys.trim().split(/\s+/).filter(Boolean);
    if (parts.length === 0 || parts.length > 2) return { reason: `"${keys}": give one key, or a chord of two keys` };
    let specs: KeySpec[];
    try {
        specs = parts.map((p) => parseKey(p, platform));
    } catch (e) {
        return { reason: String((e as Error).message ?? e) };
    }
    const commands = new Set<string>();
    for (const { row, source } of effectiveRows(platform)) {
        if (row.devOnly) continue;
        const bound = source.split(" ").map((p) => parseKey(p, platform));
        if (bound.length === specs.length && bound.every((b, i) => sameKey(b, specs[i]))) commands.add(row.command);
    }
    if (commands.size === 0) return { reason: `"${keys}" isn't a key in the shortcut table: ListShortcuts lists them` };
    for (const c of commands) {
        const refused = refusalFor(c, platform);
        if (refused) return { reason: `"${keys}" runs ${c}, which is not available to agents: ${refused}` };
    }
    const events: KeyEventPlan[] = [];
    for (const [i, spec] of specs.entries()) {
        const ev = eventFor(spec);
        if (!ev) return { reason: `"${parts[i]}": PressKeys can't send that key yet` };
        events.push(ev);
    }
    return { events, modifiers: specs.map(modifierNames), commands: [...commands] };
}
