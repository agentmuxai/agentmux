// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Resolves a key press to a command from the default table (defaults.ts),
// given where focus is. Pure: the dispatcher supplies the context.

import { DEFAULT_KEYBINDINGS, type KeyBindingRow } from "./defaults";
import { formatKey, matchKey, parseKey, sameKey, type KeyEventLike, type KeyPlatform, type KeySpec } from "./keys";

/** Where focus is when a key is pressed. */
export interface KeyContext {
    /** A text field, the agent composer or the code editor has focus. */
    textInputFocus: boolean;
    /** A terminal has focus. */
    terminalFocus: boolean;
    /** The focused pane's view type ("term", "agent", "editor", …). */
    viewType: string;
}

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

function rowsFor(platform: KeyPlatform): CompiledRow[] {
    let rows = compiled.get(platform);
    if (!rows) {
        rows = [];
        for (const row of DEFAULT_KEYBINDINGS) {
            for (const source of (platform === "mac" ? row.mac : row.other) ?? []) {
                rows.push({ row, source, steps: source.split(" ").map((s) => parseKey(s, platform)) });
            }
        }
        compiled.set(platform, rows);
    }
    return rows;
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
    const disjoint = (a?: string, b?: string) => {
        const ta = new Set((a ?? "").split("&&").map((s) => s.trim()).filter(Boolean));
        for (const t of (b ?? "").split("&&").map((s) => s.trim()).filter(Boolean)) {
            if (ta.has(t.startsWith("!") ? t.slice(1) : `!${t}`)) return true;
        }
        return false;
    };
    for (let i = 0; i < rows.length; i++) {
        for (let j = i + 1; j < rows.length; j++) {
            const a = rows[i];
            const b = rows[j];
            if (a.row.command === b.row.command) continue;
            if (a.steps.length !== b.steps.length) {
                // A chord leader must not also be a single-key binding.
                const [single, chord] = a.steps.length === 1 ? [a, b] : [b, a];
                if (sameKey(single.steps[0], chord.steps[0])) out.push(`${single.source} (${single.row.command}) shadows chord ${chord.source}`);
                continue;
            }
            if (a.steps.every((s, k) => sameKey(s, b.steps[k])) && !disjoint(a.row.when, b.row.when)) {
                out.push(`${a.source}: ${a.row.command} vs ${b.row.command}`);
            }
        }
    }
    return out;
}

export const _rowsForTests = rowsFor;
