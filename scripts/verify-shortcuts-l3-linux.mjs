#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Layer L3 on Linux: does each shortcut's key reach AgentMux at all, or does
// the desktop take it first? It presses every Linux key in the shortcut table
// as a real keyboard event through the kernel (ydotool, /dev/uinput), so the
// compositor's own shortcuts apply, and records whether the key arrived in
// the page. docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md §3
// and §6; docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, L3.
//
// Usage:
//   node scripts/verify-shortcuts-l3-linux.mjs [--port N] [--only REGEX]
//       [--wait-focus SECONDS] [--json]
//
// The keys go to whatever window has keyboard focus on the real desktop, so:
// - Run it against a dev build or a throwaway instance (AGENTMUX_CDP_PORT,
//   9223 by default in dev), with nobody using the desktop.
// - It refuses to start, and stops, unless that instance's window has focus.
//   Wayland lets no program give a window focus: click the window. With
//   --wait-focus it waits that long for the click instead of refusing.
// - Keys GNOME binds (scripts/gnome-grabs.mjs) are never pressed: what GNOME
//   does with one (switch or move workspaces) can't be confirmed or reliably
//   undone from here on Wayland. They're reported as taken; a person confirms.
//   It runs only in a GNOME session whose keybindings it can read, so a missing
//   gsettings can't make every key look free.
// - Keys a "manual" row of the shortcut table uses globally (closing a tab or
//   pane, opening an agent, the microphone) are never pressed.
// Each key is pressed with the Help pane focused, so the pane-local keys act
// on nothing; the global ones run their commands, as at L2, and whatever they
// open (a dialog, a window) is closed again.
//
// Needs ydotool and its daemon (ydotoold) with access to /dev/uinput.
//
// Exit code: 0 when every key pressed reached the app, 1 when one didn't, 2
// on a setup error (no CDP target, no ydotool, the window not focused).

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { normTable, readGnomeGrabs } from "./gnome-grabs.mjs";
import { appWindows, close, closeNewWindows, connect, js, MANUAL, S, settle, sleep } from "./verify-shortcuts-cdp.mjs";

function parseArgs() {
    const a = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9223, only: null, waitFocus: 0, json: false };
    const argv = process.argv.slice(2);
    for (let i = 0; i < argv.length; i++) {
        const k = argv[i];
        if (k === "--port") a.port = Number(argv[++i]);
        else if (k === "--only") a.only = new RegExp(argv[++i]);
        else if (k === "--wait-focus") a.waitFocus = Number(argv[++i]);
        else if (k === "--json") a.json = true;
        else {
            console.error(`unknown argument: ${k}`);
            process.exit(2);
        }
    }
    return a;
}

let args;

/** Linux input event codes (linux/input-event-codes.h), by DOM `code`. */
const EVDEV = {
    Escape: 1, Digit1: 2, Digit2: 3, Digit3: 4, Digit4: 5, Digit5: 6, Digit6: 7, Digit7: 8, Digit8: 9, Digit9: 10, Digit0: 11,
    Minus: 12, Equal: 13, Backspace: 14, Tab: 15, KeyQ: 16, KeyW: 17, KeyE: 18, KeyR: 19, KeyT: 20, KeyY: 21, KeyU: 22,
    KeyI: 23, KeyO: 24, KeyP: 25, BracketLeft: 26, BracketRight: 27, Enter: 28, KeyA: 30, KeyS: 31, KeyD: 32, KeyF: 33,
    KeyG: 34, KeyH: 35, KeyJ: 36, KeyK: 37, KeyL: 38, Semicolon: 39, Quote: 40, Backquote: 41, Backslash: 43, KeyZ: 44,
    KeyX: 45, KeyC: 46, KeyV: 47, KeyB: 48, KeyN: 49, KeyM: 50, Comma: 51, Period: 52, Slash: 53, Space: 57,
    F1: 59, F2: 60, F3: 61, F4: 62, F5: 63, F6: 64, F7: 65, F8: 66, F9: 67, F10: 68, NumpadSubtract: 74, NumpadAdd: 78,
    Numpad0: 82, F11: 87, F12: 88, Home: 102, ArrowUp: 103, PageUp: 104, ArrowLeft: 105, ArrowRight: 106, End: 107,
    ArrowDown: 108, PageDown: 109, Insert: 110, Delete: 111, NumpadMultiply: 55, Numpad7: 71, Numpad8: 72, Numpad9: 73,
    Numpad4: 75, Numpad5: 76, Numpad6: 77, Numpad1: 79, Numpad2: 80, Numpad3: 81, NumpadDecimal: 83, NumpadEnter: 96,
    NumpadDivide: 98, IntlBackslash: 86,
};
/** CDP modifier bits (Alt 1, Ctrl 2, Meta 4, Shift 8) and their keys. */
const MODIFIERS = [
    [2, 29], // Ctrl: KEY_LEFTCTRL
    [8, 42], // Shift: KEY_LEFTSHIFT
    [1, 56], // Alt: KEY_LEFTALT
    [4, 125], // Meta: KEY_LEFTMETA
];

/** ydotool's `key` arguments for one planned key: modifiers down, the key, modifiers up. */
export function keySequence(ev) {
    const code = EVDEV[ev.code];
    if (code == null) throw new Error(`no Linux key code for ${ev.code}`);
    const mods = MODIFIERS.filter(([bit]) => ev.modifiers & bit).map(([, c]) => c);
    return [...mods.map((c) => `${c}:1`), `${code}:1`, `${code}:0`, ...mods.reverse().map((c) => `${c}:0`)];
}

/** Presses one key with its modifiers through ydotool. */
function press(ev) {
    execFileSync("ydotool", ["key", ...keySequence(ev)], { timeout: 5000, stdio: "ignore" });
}

const ARRIVALS = `(() => {
    if (!window.__l3keys) {
        const log = [];
        addEventListener("keydown", (e) => {
            log.push({ at: Date.now(), code: e.code, mods: (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0), trusted: e.isTrusted });
            if (log.length > 200) log.shift();
        }, true);
        window.__l3keys = log;
    }
    return true;
})()`;

function ydotoolReady() {
    try {
        execFileSync("ydotool", ["--help"], { stdio: "ignore", timeout: 3000 });
    } catch (e) {
        if (e.code === "ENOENT") return "ydotool isn't installed";
    }
    const socket = process.env.YDOTOOL_SOCKET ?? path.join(process.env.XDG_RUNTIME_DIR ?? "/tmp", ".ydotool_socket");
    return fs.existsSync(socket) ? null : `no ydotoold socket at ${socket}: start ydotoold (systemctl --user start ydotool)`;
}

/** Puts keyboard focus in the Help pane (help:shortcuts opens it, or focuses
 *  the one already open); its block id, or null. */
async function focusHelp(cdp, helpId) {
    const inHelp = (id) =>
        cdp.evaluate(`(() => {
            const a = document.activeElement;
            return !!a && !a.closest(".xterm, [contenteditable=true]") && !!a.closest(${js(`[data-blockid="${id}"]`)});
        })()`);
    if (helpId && (await cdp.evaluate(`${S}.focus(${js(helpId)})`))) {
        await sleep(150);
        if (await inHelp(helpId)) return helpId;
    }
    for (const first of [null, "tab:goto:1"]) {
        if (first) await cdp.evaluate(`${S}.run(${js(first)})`);
        await cdp.evaluate(`${S}.run("help:shortcuts")`);
        await sleep(500);
        const id = await cdp.evaluate(`${S}.focused()`);
        if (id && (await inHelp(id))) return id;
    }
    return null;
}

async function main() {
    args = parseArgs();
    if (process.platform !== "linux") {
        console.error("this is the Linux L3 pass; macOS has verify-shortcuts-l3-macos.mjs");
        process.exit(2);
    }
    // Fail closed: with no grab list, every key would look free.
    const grabs = /GNOME/i.test(process.env.XDG_CURRENT_DESKTOP ?? "") ? readGnomeGrabs() : null;
    if (!grabs?.readable) {
        console.error(grabs ? "couldn't read GNOME's keybindings (gsettings): not pressing keys it might take" : "not a GNOME session (XDG_CURRENT_DESKTOP): this pass only knows which keys GNOME takes");
        process.exit(2);
    }
    const notReady = ydotoolReady();
    if (notReady) {
        console.error(notReady);
        process.exit(2);
    }
    let cdp;
    try {
        cdp = await connect(args.port);
    } catch (e) {
        console.error(e.message);
        process.exit(2);
    }
    const focused = () => cdp.evaluate("document.hasFocus()");
    /** Waits up to `ms` for the window to have keyboard focus. */
    const regainFocus = async (ms) => {
        for (let waited = 0; waited < ms && !(await focused()); waited += 250) await sleep(250);
        return focused();
    };
    if (args.waitFocus > 0 && !(await focused())) console.error(`waiting up to ${args.waitFocus} s for a click on the AgentMux window…`);
    if (!(await regainFocus(args.waitFocus * 1000))) {
        console.error("the AgentMux window on this port doesn't have keyboard focus: click it, then run this again (or pass --wait-focus)");
        process.exit(2);
    }
    // Give the person who clicked a moment to let go of the mouse and keys.
    if (args.waitFocus > 0) await sleep(3000);
    await cdp.evaluate(ARRIVALS);
    const list = await cdp.evaluate(`${S}.list()`);

    // One entry per distinct key, with every row that uses it.
    const keys = new Map();
    for (const s of list) {
        for (const raw of s.raw) {
            // Every step: chords that share a first key are different keys.
            const id = raw.split(" ").map(normTable).join(" ");
            if (!keys.has(id)) keys.set(id, { raw, rows: [], manual: [] });
            const k = keys.get(id);
            k.rows.push(s.command);
            // A manual row that isn't pane-local would run wherever focus is.
            if (MANUAL[s.command] && !s.pane && !/^term:/.test(s.command)) k.manual.push(s.command);
        }
    }
    const results = [];
    let helpId = null;
    for (const [id, k] of keys) {
        // The desktop sees a chord's first key.
        const norm = id.split(" ")[0];
        if (args.only && !k.rows.some((c) => args.only.test(c))) continue;
        const r = { key: k.raw, rows: k.rows, result: "", resolved: null };
        results.push(r);
        const owners = grabs.get(norm) ?? [];
        if (k.manual.length) {
            r.result = `skipped: also ${k.manual.join(", ")} (manual)`;
            continue;
        }
        if (owners.length) {
            r.result = `skipped: taken by GNOME (${owners.join(", ")})`;
            continue;
        }
        helpId = await focusHelp(cdp, helpId);
        if (!helpId) {
            r.result = "skipped: couldn't put focus in the Help pane";
            continue;
        }
        if (!(await focused())) {
            r.result = "stopped: the window lost keyboard focus before this key";
            break;
        }
        const plan = await cdp.evaluate(`${S}.plan(${js(k.raw)})`);
        if (plan.reason) {
            r.result = `skipped: ${plan.reason}`;
            continue;
        }
        const unknown = plan.events.find((ev) => EVDEV[ev.code] == null);
        if (unknown) {
            r.result = `skipped: no Linux key code for ${unknown.code}`;
            continue;
        }
        const windowsBefore = new Set((await appWindows(args.port)).map((t) => t.id));
        await cdp.evaluate(ARRIVALS);
        const before = await cdp.evaluate("Date.now()");
        let reached = true;
        for (const ev of plan.events) {
            press(ev);
            let got = false;
            for (let waited = 0; waited < 500 && !got; waited += 50) {
                await sleep(50);
                got = await cdp.evaluate(`window.__l3keys.some((k) => k.at >= ${before} && k.code === ${js(ev.code)} && k.mods === ${ev.modifiers})`);
            }
            reached &&= got;
        }
        const last = await cdp.evaluate(`${S}.last()`);
        r.resolved = last && last.at >= before ? last.command : null;
        r.result = reached ? "reached" : "didn't reach the app";
        await settle(cdp).catch(() => {});
        await closeNewWindows(args.port, windowsBefore).catch(() => {});
        // A window a key opened took focus; closing it hands focus back.
        if (!(await regainFocus(1500))) {
            r.result += "; then the window lost keyboard focus, so the run stopped";
            break;
        }
    }
    await close(cdp);
    report(results);
}

function report(results) {
    const failed = results.filter((r) => /didn't reach|stopped|lost/.test(r.result));
    if (args.json) {
        console.log(JSON.stringify({ port: args.port, results }, null, 2));
    } else {
        const pressed = results.filter((r) => !r.result.startsWith("skipped")).length;
        console.log(`${results.length} keys, ${pressed} pressed, ${failed.length} not reaching the app.\n`);
        console.log("| Key | Rows | L3 | Resolved by the app |\n|---|---|---|---|");
        for (const r of results) console.log(`| \`${r.key}\` | ${r.rows.map((c) => `\`${c}\``).join(" ")} | ${r.result} | ${r.resolved ? `\`${r.resolved}\`` : ""} |`);
    }
    process.exit(failed.length ? 1 : 0);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    main().catch((e) => {
        console.error(e.stack ?? e);
        process.exit(2);
    });
}
