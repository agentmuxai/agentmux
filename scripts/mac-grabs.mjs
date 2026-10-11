#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Which macOS keys in the shortcut table something else takes first: the OS,
// AgentMux's own menu bar, or a window manager app (layer L3 of
// docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md, read-only).
// It reads settings and source, and presses no keys.
//
// Author: Masty@starpower. Run on macOS with Node 22.18+ (it imports the
// table's TypeScript directly): `node scripts/mac-grabs.mjs [--json]`.
//
// Limits:
// - macOS stores only the system shortcuts a user has changed, so the rest
//   come from Apple's defaults as listed below. The Shift variants of the
//   Mission Control and Spaces keys (ids 34, 35, 80, 82) aren't shown in
//   System Settings; they're reported as "suspected" until a key press
//   confirms them.
// - Of window manager apps, only Magnet's settings are read. Others that are
//   running are named, not checked.
// - Not covered: input methods, and the fn + arrow keys that type
//   Page Up/Down on a keyboard without them.
// - A chord is checked by its first key.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const repo = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
const { DEFAULT_KEYBINDINGS } = await import(join(repo, "frontend/app/keybindings/defaults.ts"));

// macOS virtual key codes, named as the table names keys.
const KEYCODE = {
    0: "a", 1: "s", 2: "d", 3: "f", 4: "h", 5: "g", 6: "z", 7: "x", 8: "c", 9: "v", 11: "b", 12: "q", 13: "w",
    14: "e", 15: "r", 16: "y", 17: "t", 18: "1", 19: "2", 20: "3", 21: "4", 22: "6", 23: "5", 24: "=", 25: "9",
    26: "7", 27: "-", 28: "8", 29: "0", 30: "]", 31: "o", 32: "u", 33: "[", 34: "i", 35: "p", 36: "Enter", 37: "l",
    38: "j", 39: "'", 40: "k", 41: ";", 42: "\\", 43: ",", 44: "/", 45: "n", 46: "m", 47: ".", 48: "Tab",
    49: "Space", 50: "`", 51: "Backspace", 53: "Escape", 96: "F5", 97: "F6", 98: "F7", 99: "F3", 100: "F8",
    101: "F9", 103: "F11", 109: "F10", 111: "F12", 115: "Home", 116: "PageUp", 117: "Delete", 118: "F4",
    119: "End", 120: "F2", 121: "PageDown", 122: "F1", 123: "ArrowLeft", 124: "ArrowRight", 125: "ArrowDown",
    126: "ArrowUp",
};

const CODE_KEY = { Backquote: "`", Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/", Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]", Backslash: "\\" };

/** "meta+shift+code:Digit1" → "meta+shift+1": sorted modifiers, then the key. */
function norm(key) {
    const parts = key.split("+");
    let k = parts.pop();
    if (k.startsWith("code:")) {
        const code = k.slice(5);
        k = code.startsWith("Digit") ? code.slice(5) : (CODE_KEY[code] ?? code);
    }
    if (k.length === 1) k = k.toLowerCase();
    return [...parts.map((m) => m.toLowerCase()).sort(), k].join("+");
}

function combo(mods, key) {
    return norm([...mods, key].join("+"));
}

const taken = new Map();
function take(key, by, confidence) {
    const c = norm(key);
    if (!taken.has(c)) taken.set(c, []);
    const list = taken.get(c);
    if (!list.some((t) => t.by === by)) list.push({ by, confidence });
}

/** A plist as JSON, or null when it's missing or holds data values. */
function plist(path) {
    try {
        return JSON.parse(execFileSync("plutil", ["-convert", "json", "-o", "-", path], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }));
    } catch {
        return null;
    }
}

// ── System shortcuts ──
// id → [key, enabled by default, confidence]
const SYSTEM_DEFAULTS = {
    7: ["ctrl+F2", true, "confirmed"], 8: ["ctrl+F3", true, "confirmed"], 9: ["ctrl+F4", true, "confirmed"],
    10: ["ctrl+F5", true, "confirmed"], 11: ["ctrl+F6", true, "confirmed"], 12: ["ctrl+F1", true, "confirmed"],
    13: ["ctrl+F7", true, "confirmed"], 57: ["ctrl+F8", true, "confirmed"],
    27: ["meta+`", true, "confirmed"],
    28: ["meta+shift+3", true, "confirmed"], 29: ["ctrl+meta+shift+3", true, "confirmed"],
    30: ["meta+shift+4", true, "confirmed"], 31: ["ctrl+meta+shift+4", true, "confirmed"],
    184: ["meta+shift+5", true, "confirmed"],
    32: ["ctrl+ArrowUp", true, "confirmed"], 33: ["ctrl+ArrowDown", true, "confirmed"],
    34: ["ctrl+shift+ArrowUp", true, "suspected"], 35: ["ctrl+shift+ArrowDown", true, "suspected"],
    79: ["ctrl+ArrowLeft", true, "confirmed"], 81: ["ctrl+ArrowRight", true, "confirmed"],
    80: ["ctrl+shift+ArrowLeft", true, "suspected"], 82: ["ctrl+shift+ArrowRight", true, "suspected"],
    36: ["F11", true, "confirmed"], 52: ["alt+meta+d", true, "confirmed"],
    60: ["ctrl+Space", true, "confirmed"], 61: ["ctrl+alt+Space", true, "confirmed"],
    64: ["meta+Space", true, "confirmed"], 65: ["alt+meta+Space", true, "confirmed"],
    98: ["meta+shift+/", true, "confirmed"], 162: ["alt+meta+F5", true, "confirmed"],
    ...Object.fromEntries([1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [117 + n, [`ctrl+${n}`, false, "confirmed"]])),
};

const stored = plist(join(homedir(), "Library/Preferences/com.apple.symbolichotkeys.plist"))?.AppleSymbolicHotKeys ?? {};
for (const [id, [key, onByDefault, confidence]] of Object.entries(SYSTEM_DEFAULTS)) {
    const s = stored[id];
    const enabled = s ? Boolean(s.enabled) : onByDefault;
    if (!enabled) continue;
    const p = s?.value?.parameters;
    let k = key;
    if (p && KEYCODE[p[1]]) {
        const m = p[2];
        const mods = [m & 0x40000 && "ctrl", m & 0x20000 && "shift", m & 0x80000 && "alt", m & 0x100000 && "meta"].filter(Boolean);
        k = combo(mods, KEYCODE[p[1]]);
    }
    take(k, `macOS system shortcut ${id}`, confidence);
}
// Fixed ones, not in that settings file.
for (const k of ["meta+Tab", "meta+shift+Tab", "ctrl+meta+q", "alt+meta+Escape", "meta+shift+q", "ctrl+meta+Space", "ctrl+meta+f"]) {
    take(k, "macOS (fixed)", "confirmed");
}

// ── AgentMux's menu bar ──
const MASK = { MOD_CMD: "meta", MOD_SHIFT: "shift", MOD_OPT: "alt", MOD_CTRL: "ctrl" };
const menu = readFileSync(join(repo, "crates/cef/src/macos_menu.rs"), "utf8");
for (const m of menu.matchAll(/add_(?:std|cmd)\([^;]*?"([^"]*)",\s*([A-Z_ |0]+)\);/g)) {
    const [, key, mask] = m;
    if (!key) continue;
    const mods = mask.split("|").map((s) => MASK[s.trim()]).filter(Boolean);
    take(combo(mods, key), "AgentMux menu bar", "confirmed");
}

// ── Window manager apps ──
const running = execFileSync("ps", ["-axo", "comm"], { encoding: "utf8" });
const magnetPrefs = join(homedir(), "Library/Preferences/com.crowdcafe.windowmagnet.plist");
if (existsSync(magnetPrefs) && /Magnet\.app/.test(running)) {
    for (const field of ["horizontalCommands", "verticalCommands"]) {
        // Each field is JSON stored as data, which `plutil -convert json` refuses.
        let cmds = [];
        try {
            const raw = execFileSync("plutil", ["-extract", field, "raw", "-o", "-", magnetPrefs], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
            cmds = JSON.parse(Buffer.from(raw.trim(), "base64").toString("utf8"));
        } catch {}
        for (const c of cmds) {
            const sc = c.keyboardShortcut;
            if (!sc?.enabled || !sc.shortcut || !KEYCODE[sc.shortcut.carbonKeyCode]) continue;
            const m = sc.shortcut.carbonModifiers;
            const mods = [m & 4096 && "ctrl", m & 512 && "shift", m & 2048 && "alt", m & 256 && "meta"].filter(Boolean);
            take(combo(mods, KEYCODE[sc.shortcut.carbonKeyCode]), "Magnet", "confirmed");
        }
    }
}
const unread = ["Rectangle", "Raycast", "Alfred", "BetterTouchTool", "Karabiner", "Hammerspoon", "Moom", "AeroSpace", "Amethyst", "skhd", "Keyboard Maestro"].filter((a) => running.includes(a));

// ── The table ──
const fnState = (() => {
    try {
        return execFileSync("defaults", ["read", "-g", "com.apple.keyboard.fnState"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim() === "1";
    } catch {
        return false;
    }
})();

let checked = 0;
const hits = [];
const fRow = [];
for (const row of DEFAULT_KEYBINDINGS) {
    if (!row.mac || row.devOnly) continue;
    for (const key of row.mac) {
        checked++;
        const first = key.split(" ")[0];
        for (const t of taken.get(norm(first)) ?? []) hits.push({ command: row.command, key, pane: row.pane ?? null, ...t });
        if (/(^|\+)F\d+$/.test(first)) fRow.push({ command: row.command, key });
    }
}

const result = { checked, hits, fRow, fnKeysAreStandard: fnState, appsNotChecked: unread };
if (process.argv.includes("--json")) {
    console.log(JSON.stringify(result, null, 2));
} else {
    console.log(`${checked} macOS keys checked; ${hits.length} taken.\n`);
    console.log("| Command | Key | Taken by | Confidence |\n|---|---|---|---|");
    for (const h of hits) console.log(`| ${h.command}${h.pane ? ` (${h.pane})` : ""} | ${h.key} | ${h.by} | ${h.confidence} |`);
    console.log(`\nF-row keys: ${fRow.map((f) => `${f.command} ${f.key}`).join(", ")}.`);
    console.log(fnState ? "They work as typed here: F1, F2, etc. are standard function keys on this Mac." : "On this Mac they need fn: F1, F2, etc. are media keys.");
    if (unread.length) console.log(`Running but not checked: ${unread.join(", ")}.`);
}
