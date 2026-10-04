// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Resolves a key press to a command from the default table (defaults.ts),
// given where focus is. Pure: the dispatcher supplies the context.

import { DEFAULT_KEYBINDINGS, type KeyBindingRow, type KeyPane } from "./defaults";
import { formatKey, matchKey, parseKey, sameKey, type KeyEventLike, type KeyPlatform, type KeySpec } from "./keys";

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
export const DOC_TAB_HOSTS = ["editor", "media"];

/** The `when` a pane row implicitly has: its keys only reach it there. */
const PANE_WHEN: Record<KeyPane, string> = {
    doctabs: "docTabsHost",
    editor: "viewType == editor",
    files: "viewType == files",
};

export interface ResolvedBinding {
    row: KeyBindingRow;
    /** The binding is a chord and this key was its first step. */
    chordStart: boolean;
}

interface CompiledRow {
    row: KeyBindingRow;
    steps: KeySpec[];
    source: string;
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

/**
 * Applies the `keybindings` setting. User keys come before the defaults, so
 * they win. Returns a message for every entry that was skipped; a bad entry
 * never breaks the defaults.
 */
export function setUserKeybindings(entries: unknown): string[] {
    const warnings: string[] = [];
    const ok: UserKeybinding[] = [];
    const ctx: KeyContext = { textInputFocus: false, terminalFocus: false, viewType: "", docTabsHost: false };
    for (const [i, raw] of (Array.isArray(entries) ? entries : []).entries()) {
        const e = raw as UserKeybinding;
        const where = `keybindings[${i}]`;
        if (!e || typeof e.command !== "string" || !e.command.replace(/^-/, "")) {
            warnings.push(`${where}: needs a "command"`);
            continue;
        }
        if (!e.command.startsWith("-") && typeof e.key !== "string") {
            warnings.push(`${where}: needs a "key" for ${e.command}`);
            continue;
        }
        if (e.platform != null && e.platform !== "mac" && e.platform !== "other") {
            warnings.push(`${where}: platform must be "mac" or "other"`);
            continue;
        }
        try {
            if (e.key) for (const p of ["mac", "other"] as const) e.key.split(" ").forEach((k) => parseKey(k, p));
            evalWhen(e.when, ctx);
        } catch (err) {
            warnings.push(`${where}: ${(err as Error).message}`);
            continue;
        }
        ok.push(e);
    }
    userBindings = ok;
    compiled.clear();
    return warnings;
}

function rowsFor(platform: KeyPlatform): CompiledRow[] {
    let rows = compiled.get(platform);
    if (!rows) {
        rows = [];
        const mine = userBindings.filter((u) => u.platform == null || u.platform === platform);
        const unbound = (command: string, source: string) =>
            mine.some((u) => u.command === `-${command}` && (u.key == null || u.key === source));
        const byCommand = new Map(DEFAULT_KEYBINDINGS.map((r) => [r.command, r] as const));
        for (const u of mine) {
            if (u.command.startsWith("-") || !u.key) continue;
            const base = byCommand.get(u.command);
            const row: KeyBindingRow = {
                command: u.command,
                label: base?.label ?? u.command,
                category: base?.category ?? "General",
                when: u.when,
                skipShell: base?.skipShell,
                pane: base?.pane,
            };
            rows.push({ row, source: u.key, steps: u.key.split(" ").map((s) => parseKey(s, platform)) });
        }
        for (const row of DEFAULT_KEYBINDINGS) {
            for (const source of (platform === "mac" ? row.mac : row.other) ?? []) {
                if (unbound(row.command, source)) continue;
                rows.push({ row, source, steps: source.split(" ").map((s) => parseKey(s, platform)) });
            }
        }
        compiled.set(platform, rows);
    }
    return rows;
}

/** Every row as resolved for `platform`, user keys first (for the help pane). */
export function effectiveRows(platform: KeyPlatform): { row: KeyBindingRow; source: string }[] {
    return rowsFor(platform).map(({ row, source }) => ({ row, source }));
}

/** `when` is `flag`, `!flag`, `viewType == x` or `viewType != x`, joined with `&&`. */
export function evalWhen(when: string | undefined, ctx: KeyContext): boolean {
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
    const whenOf = (r: KeyBindingRow) => [r.when, r.pane && PANE_WHEN[r.pane]].filter(Boolean).join(" && ");
    for (let i = 0; i < rows.length; i++) {
        for (let j = i + 1; j < rows.length; j++) {
            const a = rows[i];
            const b = rows[j];
            if (a.row.command === b.row.command) continue;
            // A pane row over a global row is precedence by design (see
            // `KeyBindingRow.pane`); two pane rows or two global rows must not clash.
            if (!a.row.pane !== !b.row.pane) continue;
            if (whenDisjoint(whenOf(a.row), whenOf(b.row))) continue;
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

export const _rowsForTests = rowsFor;
