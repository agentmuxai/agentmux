#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Which Linux ("other") keys in the shortcut table GNOME takes first: the
// accelerators it grabs on this host, read with gsettings (layer L3 of
// docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md, read-only).
// It presses no keys.
//
// Author: Maricon@charlie. Run on a GNOME desktop with Node 22.18+ (it
// imports the table's TypeScript directly):
// `node scripts/gnome-grabs.mjs [checkout] [--json]` (default: this checkout).
//
// Limits:
// - GNOME only: KDE, Xfce and the tiling window managers keep their keys
//   elsewhere.
// - Of extensions, only Dash to Dock and Tiling Assistant are read.
// - Not covered: input method switch keys (IBus, Fcitx) and keys set by
//   xkb options.
// - A chord is checked by its first key.
//
// verify-shortcuts-l3-linux.mjs imports readGnomeGrabs and normTable to skip
// the keys GNOME takes before it presses anything.
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import path from "node:path";

const SCHEMAS = [
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.settings-daemon.plugins.media-keys",
    "org.gnome.shell.extensions.dash-to-dock",
    "org.gnome.shell.extensions.tiling-assistant",
];

const KEY_ALIASES = {
    left: "arrowleft", right: "arrowright", up: "arrowup", down: "arrowdown",
    page_up: "pageup", page_down: "pagedown", kp_prior: "pageup", kp_next: "pagedown",
    above_tab: "code:backquote", grave: "code:backquote", return: "enter", space: " ",
    minus: "-", equal: "=", plus: "=", comma: ",", slash: "/", period: ".",
    bracketleft: "[", bracketright: "]", backslash: "\\", semicolon: ";", apostrophe: "'",
};
const MOD_ALIASES = { control: "ctrl", primary: "ctrl", ctrl: "ctrl", alt: "alt", shift: "shift", super: "meta", meta: "meta" };

// "<Control><Alt>Left" -> "alt+ctrl+arrowleft"
export function normGnome(accel) {
    const mods = new Set();
    const rest = accel.replace(/<([^>]+)>/g, (_, m) => {
        const k = MOD_ALIASES[m.toLowerCase()];
        if (k) mods.add(k);
        return "";
    });
    if (!rest) return null;
    let key = rest.toLowerCase();
    key = KEY_ALIASES[key] ?? key;
    return [...[...mods].sort(), key].join("+");
}

// Table syntax "ctrl+shift+ArrowUp" -> same normal form. A chord's first key is what the OS sees.
export function normTable(keys) {
    const first = keys.split(" ")[0];
    const parts = first.split("+");
    // "ctrl++" style isn't used; a trailing empty part means the key was "+".
    let key = parts.pop().toLowerCase();
    if (key === "space") key = " ";
    const mods = parts.map((m) => MOD_ALIASES[m.toLowerCase()] ?? m.toLowerCase()).sort();
    return [...mods, key].join("+");
}

/** A schema's settings, or null when gsettings can't read it. */
function gsettingsList(schema) {
    try {
        return execFileSync("gsettings", ["list-recursively", schema], { encoding: "utf8" });
    } catch {
        return null;
    }
}

/** The accelerators GNOME grabs on this host: normal form -> the settings that bind it.
 *  `readable` is false when the window manager's own keybindings couldn't be read
 *  (no gsettings, no session bus, not GNOME): an empty map then means "unknown". */
export function readGnomeGrabs() {
    const grabs = new Map();
    const addGrab = (accel, owner) => {
        const n = normGnome(accel);
        if (!n) return;
        if (!grabs.has(n)) grabs.set(n, []);
        grabs.get(n).push(owner);
    };
    let wmRead = false;
    for (const schema of SCHEMAS) {
        const listed = gsettingsList(schema);
        if (schema === "org.gnome.desktop.wm.keybindings") wmRead = Boolean(listed?.trim());
        for (const line of (listed ?? "").split("\n")) {
            const m = line.match(/^(\S+) (\S+) (.*)$/);
            if (!m) continue;
            const accels = [...m[3].matchAll(/'([^']*<[^']*|[^']*)'/g)].map((x) => x[1]).filter((a) => a.includes("<") || /^(F\d+|Print)$/.test(a));
            for (const a of accels) addGrab(a, `${schema.replace(/^org\.gnome\./, "")} ${m[2]}`);
        }
    }
    // Custom shortcuts (Settings > Keyboard > Custom).
    const customs = (gsettingsList("org.gnome.settings-daemon.plugins.media-keys") ?? "").match(/custom-keybindings \[(.*)\]/)?.[1] ?? "";
    for (const p of [...customs.matchAll(/'([^']+)'/g)].map((x) => x[1])) {
        const schema = `org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:${p}`;
        const binding = execFileSync("gsettings", ["get", schema, "binding"], { encoding: "utf8" }).trim().replace(/^'|'$/g, "");
        const name = execFileSync("gsettings", ["get", schema, "name"], { encoding: "utf8" }).trim();
        addGrab(binding, `custom ${name}`);
    }
    grabs.readable = wmRead;
    return grabs;
}

async function main() {
    const repo = process.argv[2] && !process.argv[2].startsWith("--") ? process.argv[2] : execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
    const { DEFAULT_KEYBINDINGS } = await import(pathToFileURL(path.join(repo, "frontend/app/keybindings/defaults.ts")).href);
    const grabs = readGnomeGrabs();
    const rows = [];
    for (const row of DEFAULT_KEYBINDINGS) {
        for (const k of row.other ?? []) {
            const owners = grabs.get(normTable(k));
            rows.push({ command: row.command, label: row.label, key: k, pane: row.pane ?? "", devOnly: !!row.devOnly, owners: owners ?? [] });
        }
    }

    const taken = rows.filter((r) => r.owners.length);
    if (process.argv.includes("--json")) {
        console.log(JSON.stringify({ grabs: Object.fromEntries(grabs), rows }, null, 2));
    } else {
        console.log(`${rows.length} Linux key entries checked, ${grabs.size} GNOME accelerators, ${taken.length} taken:\n`);
        console.log("| Command | Label | Key | Taken by (GNOME setting) |");
        console.log("|---|---|---|---|");
        for (const r of taken) console.log(`| \`${r.command}\`${r.devOnly ? " (dev only)" : ""} | ${r.label} | \`${r.key}\` | ${r.owners.map((o) => `\`${o}\``).join(", ")} |`);
    }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
