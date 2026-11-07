// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Turns "this command should now have these keys" into the smallest change to the `keybindings` setting, and
// finds what a key would collide with. Pure: no store, no UI, no clock; Settings and the agents' route share it
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §3, §5).

import { DEFAULT_KEYBINDINGS } from "./defaults";
import { keyId, type KeyPlatform } from "./keys";
import { compileRows, contextOf, defaultKeysOf, validEntries, whenDisjoint, type UserKeybinding } from "./registry";
import { reservedFor, type Reserved } from "./reserved";

/** A key and the context it works in (`when`, as in the setting). A default key is the same key only with the
 *  same `when`: carry each key's `when` along from `choicesOf` or `bindingsView`'s `keyRows`. */
export interface KeyChoice {
    key: string;
    when?: string;
}

/**
 * What a key would collide with.
 * - `same`: the same key, where both can fire.
 * - `shadows`: a single key that is the first step of the other's chord.
 * - `shadowed`: a chord whose first step is the other's single key.
 * - `pane`: the other is a pane's own key and the candidate is global (or the other way round); the pane's
 *   handler runs first in its pane, so it never blocks saving.
 */
export interface Conflict {
    command: string;
    label: string;
    /** The other command's key, as written. */
    key: string;
    when?: string;
    kind: "same" | "shadows" | "shadowed" | "pane";
    /** Saving is not possible while it stands. */
    blocking: boolean;
}

export interface PlannedChange {
    /** The whole new setting. Entries of other commands, and entries that are not valid, are kept as they are. */
    entries: unknown[];
    /** What the keys that were added collide with, after the change. */
    conflicts: Conflict[];
    /** Keys that were added and are reserved (blocked ones cannot be saved). */
    reserved: { key: string; reserved: Reserved }[];
}

export interface PlanOptions {
    /** Unbind, on the other commands, the keys that block the new ones. */
    replace?: boolean;
}

/** Where the command works today (its row's `when`): a key given to it works there unless said otherwise, since
 *  an entry with no `when` fires everywhere. */
export const ownWhen = (command: string): string | undefined => DEFAULT_KEYBINDINGS.find((r) => r.command === command)?.when;

/** The platform an entry the view writes is pinned to: Windows and Linux share "other". */
function pinned(platform: KeyPlatform): "mac" | "other" {
    return platform === "mac" ? "mac" : "other";
}

const tidy = (key: string) => key.trim().split(/\s+/).join(" ");

/** Only the entries the registry applies: one it skips never fires, so it neither conflicts nor holds a key. */
const asEntries = validEntries;

function commandOf(entry: unknown): string | null {
    const c = (entry as UserKeybinding | null)?.command;
    return typeof c === "string" ? c.replace(/^-/, "") : null;
}

/** The keys `command` has on `platform` once `entries` are applied. */
export function choicesOf(entries: unknown[], command: string, platform: KeyPlatform): KeyChoice[] {
    return compileRows(platform, asEntries(entries))
        .filter((c) => c.row.command === command)
        .map((c) => ({ key: c.source, when: c.row.when }));
}

/**
 * What `candidate` would collide with on `platform`, given the setting's `entries`. The candidate's own
 * command is never a conflict, so a key it already has is free. A pane's key is matched by its pane, so a pane
 * row and a global row only get a non-blocking `pane` note.
 */
export function conflictsFor(candidate: { command: string; key: string; when?: string }, entries: unknown[], platform: KeyPlatform): Conflict[] {
    const base = DEFAULT_KEYBINDINGS.find((r) => r.command === candidate.command);
    const mine = { when: candidate.when, pane: base?.pane };
    const steps = candidate.key.trim().split(/\s+/);
    const ids = steps.map((s) => keyId(s, platform));
    const out: Conflict[] = [];
    for (const c of compileRows(platform, asEntries(entries))) {
        if (c.row.command === candidate.command) continue;
        const theirs = c.source.split(" ").map((s) => keyId(s, platform));
        let kind: Conflict["kind"];
        if (ids.length === theirs.length && ids.every((id, i) => id === theirs[i])) kind = "same";
        else if (ids.length === 1 && theirs.length === 2 && ids[0] === theirs[0]) kind = "shadows";
        else if (ids.length === 2 && theirs.length === 1 && ids[0] === theirs[0]) kind = "shadowed";
        else continue;
        if (whenDisjoint(contextOf(mine), contextOf(c.row))) continue;
        const mixed = !mine.pane !== !c.row.pane;
        out.push({ command: c.row.command, label: c.row.label, key: c.source, when: c.row.when, kind: mixed ? "pane" : kind, blocking: !mixed });
    }
    return out;
}

/**
 * The setting after `command` is given exactly the keys `wanted` on `platform`.
 *
 * The change is a diff from the defaults, so a default added later still reaches the person: a default key
 * that is dropped becomes `-command key`, a key that is not a default becomes a plain entry, and dropping every
 * key writes one bare `-command`. Every entry written carries `platform`, so a settings file shared between a Mac
 * and a PC never moves one's keys onto the other. The command's entries for this platform are replaced as a
 * group (unbinds first, then adds, the order the registry applies them in); an entry without `platform` is kept
 * for the other platform, so that platform does not change.
 */
export function planChange(entries: unknown[], command: string, wanted: KeyChoice[], platform: KeyPlatform, options: PlanOptions = {}): PlannedChange {
    const here = pinned(platform);
    const there = here === "mac" ? "other" : "mac";

    const kept: unknown[] = [];
    let at = -1;
    for (const e of entries) {
        if (commandOf(e) !== command) {
            kept.push(e);
            continue;
        }
        if (at < 0) at = kept.length;
        const platformOf = (e as UserKeybinding).platform;
        if (platformOf === there) kept.push(e);
        else if (platformOf == null) kept.push({ ...(e as UserKeybinding), platform: there });
        // Pinned to this platform: replaced below.
    }

    const defaults = defaultKeysOf(command, platform);
    const same = (a: KeyChoice, b: KeyChoice) => keyId(a.key, platform) === keyId(b.key, platform) && (a.when ?? "") === (b.when ?? "");
    const group: UserKeybinding[] = [];
    if (wanted.length === 0) {
        if (defaults.length > 0) group.push({ command: `-${command}`, platform: here });
    } else {
        for (const d of defaults) if (!wanted.some((w) => same(w, d))) group.push({ command: `-${command}`, key: d.key, platform: here });
    }
    const added = wanted.filter((w, i) => !defaults.some((d) => same(d, w)) && wanted.findIndex((x) => same(x, w)) === i);
    for (const w of added) group.push({ command, key: tidy(w.key), ...(w.when ? { when: w.when } : {}), platform: here });

    const cut = at < 0 ? kept.length : at;
    let next = [...kept.slice(0, cut), ...group, ...kept.slice(cut)];

    if (options.replace) {
        const bumped = new Map<string, Set<string>>();
        for (const w of added) {
            for (const c of conflictsFor({ command, key: w.key, when: w.when }, kept, platform)) {
                if (!c.blocking) continue;
                (bumped.get(c.command) ?? bumped.set(c.command, new Set()).get(c.command)!).add(keyId(c.key, platform));
            }
        }
        for (const [other, gone] of bumped) {
            const left = choicesOf(next, other, platform).filter((k) => !gone.has(keyId(k.key, platform)));
            next = planChange(next, other, left, platform).entries;
        }
    }

    return {
        entries: next,
        conflicts: added.flatMap((w) => conflictsFor({ command, key: w.key, when: w.when }, next, platform)),
        reserved: added.flatMap((w) => {
            const r = reservedFor(w.key, platform);
            return r ? [{ key: tidy(w.key), reserved: r }] : [];
        }),
    };
}
