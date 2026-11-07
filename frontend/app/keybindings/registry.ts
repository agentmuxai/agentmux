// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Resolves a key press to a command from the default table (defaults.ts),
// given where focus is. Pure: the dispatcher supplies the context.

import { createSignal } from "solid-js";
import { DEFAULT_KEYBINDINGS, rowKeys, type KeyBindingRow, type KeyCategory, type KeyPane } from "./defaults";
import { formatKey, keyId, matchKey, parseKey, sameKey, type KeyEventLike, type KeyPlatform, type KeySpec } from "./keys";

/** Where focus is when a key is pressed. */
export interface KeyContext {
    /** A text field, the agent composer or the code editor has focus. */
    textInputFocus: boolean;
    /** A terminal has focus. */
    terminalFocus: boolean;
    /** The focused pane's view type ("term", "agent", "editor", …). */
    viewType: string;
    /** The focused pane has document tabs (editor, media). */
    docTabsHost: boolean;
}

/** View types whose panes have document tabs. */
export const DOC_TAB_HOSTS = ["editor"];

/** The `when` a pane row implicitly has: its keys only reach it there. */
const PANE_WHEN: Record<KeyPane, string> = {
    browser: "viewType == browser",
    doctabs: "docTabsHost",
    editor: "viewType == editor",
    files: "viewType == files",
};

/** Where a row's keys reach: its own `when` plus the pane it belongs to. */
export function contextOf(row: Pick<KeyBindingRow, "when" | "pane">): string {
    return [row.when, row.pane && PANE_WHEN[row.pane]].filter(Boolean).join(" && ");
}

export interface ResolvedBinding {
    row: KeyBindingRow;
    /** The binding is a chord and this key was its first step. */
    chordStart: boolean;
}

export interface CompiledRow {
    row: KeyBindingRow;
    steps: KeySpec[];
    source: string;
    /** From the person's `keybindings` setting, not from the default table. */
    user: boolean;
}

const compiled = new Map<KeyPlatform, CompiledRow[]>();

/** One entry of the `keybindings` setting (report §6.6). */
export interface UserKeybinding {
    key?: string;
    /** A command id; `-command` unbinds it (just `key`, or every key when no key is given). */
    command: string;
    when?: string;
    /** Limit to one platform; both when omitted. */
    platform?: KeyPlatform;
}

let userBindings: UserKeybinding[] = [];

/** An entry of the setting that was skipped, with where it is, so the Settings view can show it under its row. */
export interface KeybindingIssue {
    /** Position in the setting's array. */
    index: number;
    /** The command the entry names, without a leading `-`; "" when it names none. */
    command: string;
    message: string;
}

let issues: KeybindingIssue[] = [];

/** Bumped whenever the user's keybindings change, so views that read the
 *  table (help pane, CodeMirror keymaps) can follow. */
const [keybindingsVersion, setKeybindingsVersion] = createSignal(0);
export { keybindingsVersion };

const TABLE_COMMANDS = new Set(DEFAULT_KEYBINDINGS.map((r) => r.command));

/** Why `e` can't be used as an entry of the `keybindings` setting, or null when it can. */
function entryProblem(e: UserKeybinding, isKnownCommand: (id: string) => boolean): string | null {
    if (!e || typeof e.command !== "string" || !e.command.replace(/^-/, "")) return 'needs a "command"';
    const unbind = e.command.startsWith("-");
    if (e.key != null && (typeof e.key !== "string" || !e.key.trim())) return '"key" is empty';
    if (!unbind && e.key == null) return `needs a "key" for ${e.command}`;
    if (e.key && e.key.trim().split(/\s+/).length > 2) return `"${e.key}": a chord is at most two keys`;
    const id = e.command.replace(/^-/, "");
    // A widget's command runs only on the user's click (palette, status bar); bound to a key it would join the
    // table agents' RunCommand reads (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.8).
    if (id.startsWith("ext:")) return `"${id}" is a widget's command, which can't be bound to a key`;
    // A typo on a default key would otherwise win and run nothing.
    if (!TABLE_COMMANDS.has(id) && !isKnownCommand(id)) return `unknown command "${id}"`;
    if (e.platform != null && e.platform !== "mac" && e.platform !== "other") return 'platform must be "mac" or "other"';
    try {
        if (e.key) for (const p of ["mac", "other"] as const) e.key.trim().split(/\s+/).forEach((k) => parseKey(k, p));
        validateWhen(e.when);
    } catch (err) {
        return (err as Error).message;
    }
    return null;
}

/** Whether a command outside the table exists (the palette's ids), as the app last told `setUserKeybindings`. */
let knownCommand: (id: string) => boolean = () => false;

/** Why `entry` would be skipped as an entry of the `keybindings` setting, or null when it would be applied: the
 *  same check as `setUserKeybindings`, for a list that is not saved yet (an import). */
export function entryIssue(entry: unknown): string | null {
    return entryProblem(entry as UserKeybinding, knownCommand);
}

/**
 * The entries of `entries` the registry would apply: the same check as `setUserKeybindings`, so a list that is
 * not saved yet (Settings, an agent's request) is read exactly as the saved one will be.
 */
export function validEntries(entries: unknown[]): UserKeybinding[] {
    return entries.filter((e): e is UserKeybinding => entryProblem(e as UserKeybinding, knownCommand) == null);
}

/**
 * Applies the `keybindings` setting. User keys come before the defaults, so
 * they win. Returns a message for every entry that was skipped; a bad entry
 * never breaks the defaults.
 */
export function setUserKeybindings(entries: unknown, isKnownCommand: (id: string) => boolean = () => false): string[] {
    const warnings: string[] = [];
    const found: KeybindingIssue[] = [];
    const ok: UserKeybinding[] = [];
    for (const [i, raw] of (Array.isArray(entries) ? entries : []).entries()) {
        const e = raw as UserKeybinding;
        const problem = entryProblem(e, isKnownCommand);
        if (problem == null) {
            ok.push(e);
            continue;
        }
        warnings.push(`keybindings[${i}]: ${problem}`);
        found.push({ index: i, command: typeof e?.command === "string" ? e.command.replace(/^-/, "") : "", message: problem });
    }
    userBindings = ok;
    issues = found;
    knownCommand = isKnownCommand;
    compiled.clear();
    setKeybindingsVersion((v) => v + 1);
    return warnings;
}

/** The entries of the setting that were skipped, in the order of the array. Reactive. */
export function keybindingIssues(): KeybindingIssue[] {
    keybindingsVersion();
    return issues;
}

/**
 * The rows `entries` (the person's valid `keybindings`) and the default table give on `platform`, user keys
 * first. Pure: Settings reads it for a list of entries that is not yet saved.
 */
export function compileRows(platform: KeyPlatform, entries: UserKeybinding[]): CompiledRow[] {
    const rows: CompiledRow[] = [];
    // Your "other" entries apply on Linux too (it's Windows and Linux).
    const mine = entries.filter((u) => u.platform == null || u.platform === platform || (u.platform === "other" && platform === "linux"));
    // By parsed key, not text: "Ctrl+Tab" and "mod+t" unbind ctrl+Tab and meta+t.
    const stepsOf = (key: string) => key.trim().split(/\s+/).map((s) => parseKey(s, platform));
    const sameSteps = (a: KeySpec[], b: KeySpec[]) => a.length === b.length && a.every((k, i) => sameKey(k, b[i]));
    const unbinds = mine
        .filter((u) => u.command.startsWith("-"))
        .map((u) => ({ command: u.command.slice(1), steps: u.key ? stepsOf(u.key) : null }));
    const unbound = (command: string, steps: KeySpec[]) =>
        unbinds.some((u) => u.command === command && (u.steps == null || sameSteps(u.steps, steps)));
    // A command's first row is its main one (later rows add chords or scoped keys).
    const byCommand = new Map<string, KeyBindingRow>();
    for (const r of DEFAULT_KEYBINDINGS) if (!byCommand.has(r.command)) byCommand.set(r.command, r);
    // In list order: an unbind removes the defaults and your own entries
    // above it, so "unbind, then rebind elsewhere" works.
    const userRows: CompiledRow[] = [];
    for (const u of mine) {
        if (u.command.startsWith("-")) {
            const command = u.command.slice(1);
            const steps = u.key ? stepsOf(u.key) : null;
            for (let i = userRows.length - 1; i >= 0; i--) {
                const r = userRows[i];
                if (r.row.command === command && (steps == null || sameSteps(steps, r.steps))) userRows.splice(i, 1);
            }
            continue;
        }
        if (!u.key) continue;
        const base = byCommand.get(u.command);
        const row: KeyBindingRow = {
            command: u.command,
            label: base?.label ?? u.command,
            category: base?.category ?? "General",
            when: u.when,
            // In a terminal: as its command's own rows do; always for a
            // command the table doesn't have (you bound it on purpose) or
            // when your `when` asks for the terminal.
            skipShell: base ? base.skipShell || /(^|&&)\s*terminalFocus\s*($|&&)/.test(u.when ?? "") : true,
            pane: base?.pane,
            helpGroup: base?.helpGroup,
        };
        // One space between chord steps, whatever was typed: labels split on " ".
        userRows.push({ row, source: u.key.trim().split(/\s+/).join(" "), steps: stepsOf(u.key), user: true });
    }
    rows.push(...userRows);
    for (const row of DEFAULT_KEYBINDINGS) {
        for (const source of rowKeys(row, platform)) {
            const steps = source.split(" ").map((s) => parseKey(s, platform));
            if (unbound(row.command, steps)) continue;
            rows.push({ row, source, steps, user: false });
        }
    }
    return rows;
}

function rowsFor(platform: KeyPlatform): CompiledRow[] {
    let rows = compiled.get(platform);
    if (!rows) {
        rows = compileRows(platform, userBindings);
        compiled.set(platform, rows);
    }
    return rows;
}

/** Every row as resolved for `platform`, user keys first (for the help pane). */
export function effectiveRows(platform: KeyPlatform): { row: KeyBindingRow; source: string }[] {
    return rowsFor(platform).map(({ row, source }) => ({ row, source }));
}

const WHEN_FLAGS = new Set(["textInputFocus", "terminalFocus", "docTabsHost"]);

/** Throws on any term that isn't a known flag or a viewType comparison.
 *  Checks every term: evalWhen stops at the first false one, so a bad flag
 *  later in the clause would otherwise slip through and throw on a key press. */
function validateWhen(when: string | undefined): void {
    if (!when) return;
    for (const raw of when.split("&&")) {
        const t = raw.trim();
        if (/^viewType\s*(==|!=)\s*\S+$/.test(t)) continue;
        if (!WHEN_FLAGS.has(t.replace(/^!/, ""))) throw new Error(`keybinding when: unknown context key "${t}"`);
    }
}

/** `when` is `flag`, `!flag`, `viewType == x` or `viewType != x`, joined with `&&`. */
function evalWhen(when: string | undefined, ctx: KeyContext): boolean {
    if (!when) return true;
    return when.split("&&").every((raw) => {
        const t = raw.trim();
        const cmp = t.match(/^viewType\s*(==|!=)\s*(\S+)$/);
        if (cmp) return cmp[1] === "==" ? ctx.viewType === cmp[2] : ctx.viewType !== cmp[2];
        const neg = t.startsWith("!");
        const flag = (neg ? t.slice(1) : t) as keyof KeyContext;
        if (!(flag in ctx) || typeof ctx[flag] !== "boolean") {
            throw new Error(`keybinding when: unknown context key "${t}"`);
        }
        return neg ? !ctx[flag] : (ctx[flag] as boolean);
    });
}

/**
 * The binding a key press runs, or null. `chordLeader` is the first step of
 * a chord already pressed, if any. In a terminal only `skipShell` bindings
 * are considered; every other key belongs to the shell.
 */
export function resolveKey(
    e: KeyEventLike,
    ctx: KeyContext,
    platform: KeyPlatform,
    chordLeader?: string
): ResolvedBinding | null {
    for (const c of rowsFor(platform)) {
        // A pane row is matched by its pane (matchPaneKey), never globally.
        if (c.row.pane) continue;
        if (ctx.terminalFocus && !c.row.skipShell) continue;
        if (!evalWhen(c.row.when, ctx)) continue;
        if (chordLeader != null) {
            if (c.steps.length === 2 && c.source.split(" ")[0] === chordLeader && matchKey(e, c.steps[1])) {
                return { row: c.row, chordStart: false };
            }
            continue;
        }
        if (matchKey(e, c.steps[0])) {
            return { row: c.row, chordStart: c.steps.length === 2 };
        }
    }
    return null;
}

/**
 * The command a pane's own key handler should run for this key, from that
 * pane's rows (`pane: "files"` …), or null. The pane is its own context, so
 * `when` isn't consulted.
 */
export function matchPaneKey(e: KeyEventLike, pane: KeyPane, platform: KeyPlatform): string | null {
    for (const c of rowsFor(platform)) {
        if (c.row.pane === pane && c.steps.length === 1 && matchKey(e, c.steps[0])) return c.row.command;
    }
    return null;
}

/** The first step of the chord a resolved binding starts. */
export function chordLeaderOf(e: KeyEventLike, platform: KeyPlatform): string | null {
    for (const c of rowsFor(platform)) {
        if (c.steps.length === 2 && matchKey(e, c.steps[0])) return c.source.split(" ")[0];
    }
    return null;
}

/** Where focus is when the host forwards a key from a browser pane. */
const BROWSER_PANE_CONTEXT: KeyContext = { textInputFocus: false, terminalFocus: false, viewType: "browser", docTabsHost: false };

/**
 * The global command a single key forwarded from a browser pane runs in the
 * effective table (user keys first), or null.
 */
export function commandForKey(source: string, platform: KeyPlatform): string | null {
    const steps = source.trim().split(/\s+/).map((s) => parseKey(s, platform));
    if (steps.length !== 1) return null;
    for (const c of rowsFor(platform)) {
        if (c.row.pane || c.steps.length !== 1 || !sameKey(c.steps[0], steps[0])) continue;
        if (!evalWhen(c.row.when, BROWSER_PANE_CONTEXT)) continue;
        return c.row.command;
    }
    return null;
}

/** Every key bound to `command` on `platform`, as written in the table. */
export function keysFor(command: string, platform: KeyPlatform): string[] {
    return rowsFor(platform)
        .filter((c) => c.row.command === command)
        .map((c) => c.source);
}

/** The label of `command`'s first key, or "" when it has none on `platform`. */
export function formatCommand(command: string, platform: KeyPlatform): string {
    const k = keysFor(command, platform)[0];
    return k ? formatKey(k, platform) : "";
}

/** Rows whose keys can collide: same first key and `when` not provably disjoint. */
export function findConflicts(platform: KeyPlatform): string[] {
    const rows = rowsFor(platform);
    const out: string[] = [];
    for (let i = 0; i < rows.length; i++) {
        for (let j = i + 1; j < rows.length; j++) {
            const a = rows[i];
            const b = rows[j];
            if (a.row.command === b.row.command) continue;
            // A pane row over a global row is precedence by design (see
            // `KeyBindingRow.pane`); two pane rows or two global rows must not clash.
            if (!a.row.pane !== !b.row.pane) continue;
            if (whenDisjoint(contextOf(a.row), contextOf(b.row))) continue;
            if (a.steps.length !== b.steps.length) {
                // A chord leader must not also be a single-key binding.
                const [single, chord] = a.steps.length === 1 ? [a, b] : [b, a];
                if (sameKey(single.steps[0], chord.steps[0])) out.push(`${single.source} (${single.row.command}) shadows chord ${chord.source}`);
                continue;
            }
            if (a.steps.every((s, k) => sameKey(s, b.steps[k]))) {
                out.push(`${a.source}: ${a.row.command} vs ${b.row.command}`);
            }
        }
    }
    return out;
}

type WhenTerm = { flag: string; neg: boolean } | { view: string; eq: boolean };

function parseTerms(when: string): WhenTerm[] {
    return when
        .split("&&")
        .map((s) => s.trim())
        .filter(Boolean)
        .map((t) => {
            const cmp = t.match(/^viewType\s*(==|!=)\s*(\S+)$/);
            if (cmp) return { view: cmp[2], eq: cmp[1] === "==" };
            return t.startsWith("!") ? { flag: t.slice(1), neg: true } : { flag: t, neg: false };
        });
}

/** True when no context can satisfy both `when` clauses. */
export function whenDisjoint(a: string, b: string): boolean {
    const ta = parseTerms(a);
    const tb = parseTerms(b);
    const contradicts = (x: WhenTerm, y: WhenTerm): boolean => {
        if ("flag" in x && "flag" in y) return x.flag === y.flag && x.neg !== y.neg;
        if ("view" in x && "view" in y) {
            if (x.eq && y.eq) return x.view !== y.view;
            return x.view === y.view && x.eq !== y.eq;
        }
        // docTabsHost holds exactly when viewType is a doc-tab host.
        const [f, v] = "flag" in x ? [x, y as { view: string; eq: boolean }] : [y as { flag: string; neg: boolean }, x];
        if (f.flag !== "docTabsHost" || !v.eq) return false;
        return DOC_TAB_HOSTS.includes(v.view) ? f.neg : !f.neg;
    };
    return ta.some((x) => tb.some((y) => contradicts(x, y)));
}

/** One key of a command, with the context it works in. */
export interface BindingKey {
    key: string;
    when?: string;
    /** Written by the person, not in the default table. */
    user: boolean;
}

/** What Settings, Help, the palette and `ListShortcuts` show for one command. */
export interface CommandBindings {
    command: string;
    label: string;
    category: KeyCategory;
    helpGroup?: string;
    pane?: KeyPane;
    devOnly: boolean;
    /** The keys the table gives it on this platform. */
    defaults: string[];
    /** The keys it has now (the person's, or the defaults they left alone). */
    keys: string[];
    keyRows: BindingKey[];
    /** Differs from the defaults: a key added or removed, or a key with a context of their own. */
    changed: boolean;
    /** No key at all ("Not set"). */
    unbound: boolean;
    /** Messages for the setting's entries that name this command and were skipped. */
    issues: string[];
}

/** The keys the default table gives `command` on `platform`, each with the `when` of its row. */
export function defaultKeysOf(command: string, platform: KeyPlatform): { key: string; when?: string }[] {
    return DEFAULT_KEYBINDINGS.filter((r) => r.command === command).flatMap((r) => rowKeys(r, platform).map((key) => ({ key, when: r.when })));
}

/**
 * Every command with its keys now and its defaults, for `platform`: the default table's order, then any
 * command the person bound that the table does not have. The one place that joins the two; views read it and
 * never walk the rows themselves.
 */
export function bindingsView(platform: KeyPlatform): CommandBindings[] {
    keybindingsVersion();
    const rows = rowsFor(platform);
    const base = new Map<string, KeyBindingRow>();
    for (const r of DEFAULT_KEYBINDINGS) if (!base.has(r.command)) base.set(r.command, r);
    const order = [...base.keys()];
    for (const c of rows) if (!base.has(c.row.command) && !order.includes(c.row.command)) order.push(c.row.command);
    return order.map((command) => {
        const main = base.get(command);
        const own = rows.filter((c) => c.row.command === command);
        const defaults = defaultKeysOf(command, platform);
        const keyRows = own.map((c) => ({ key: c.source, when: c.row.when, user: c.user }));
        const ids = (keys: string[]) => new Set(keys.map((k) => keyId(k, platform)));
        const have = ids(keyRows.map((k) => k.key));
        const was = ids(defaults.map((k) => k.key));
        const changed = have.size !== was.size || [...have].some((k) => !was.has(k)) || keyRows.some((k) => k.user && k.when);
        return {
            command,
            label: main?.label ?? own[0]?.row.label ?? command,
            category: main?.category ?? own[0]?.row.category ?? "General",
            helpGroup: main?.helpGroup,
            pane: main?.pane,
            devOnly: !!main?.devOnly,
            defaults: [...new Set(defaults.map((k) => k.key))],
            keys: [...new Set(keyRows.map((k) => k.key))],
            keyRows,
            changed,
            unbound: keyRows.length === 0,
            issues: issues.filter((i) => i.command === command).map((i) => i.message),
        };
    });
}

export const _rowsForTests = rowsFor;
