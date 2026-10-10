#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Checks every shortcut the Help pane lists in a running AgentMux instance,
// at layers L1 and L2 of docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md:
//   L1  the command runs (RunCommand's code path: `__agentmux_shortcuts.run`)
//   L2  each of its keys, sent as real key events with modifiers through the
//       DevTools protocol, resolves to that command (PressKeys' code path)
// L3, whether the OS lets the key reach the app at all, is the grab reports
// (scripts/gnome-grabs.mjs, scripts/mac-grabs.mjs) and a person at the keys.
//
// Usage:
//   node scripts/verify-shortcuts.mjs [--port N] [--layer 1|2|both] [--only REGEX]
//       [--files ID] [--editor ID] [--media ID] [--term ID] [--json]
//
// Run it against a dev build or a throwaway instance (the port is
// AGENTMUX_CDP_PORT, 9223 by default in dev): it really runs each command,
// so it opens tabs, panes and a window, and changes focus. It never runs a
// command that closes or deletes, or one that opens an agent; those are
// listed as "manual". A pane's own commands (files:*, editor:*, doctab:*,
// term:copy/paste/clear) run only when you pass a pane of that kind, by
// block id (WhoAmI or Layout shows them); doctab:* runs in --editor, or in
// --media if no editor is given.
//
// Exit code: 0 when everything checked passes, 1 when anything fails, 2 on a
// setup error (no CDP target, or a build without the shortcut API).

const args = (() => {
    const a = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9223, layer: "both", only: null, panes: {}, json: false };
    const argv = process.argv.slice(2);
    for (let i = 0; i < argv.length; i++) {
        const k = argv[i];
        const v = () => argv[++i];
        if (k === "--port") a.port = Number(v());
        else if (k === "--layer") a.layer = v();
        else if (k === "--only") a.only = new RegExp(v());
        else if (k === "--json") a.json = true;
        else if (["--files", "--editor", "--media", "--term"].includes(k)) a.panes[k.slice(2)] = v();
        else {
            console.error(`unknown argument: ${k}`);
            process.exit(2);
        }
    }
    return a;
})();

/** Commands this script leaves to a person, and why. */
const MANUAL = {
    "pane:close": "closes a pane (refused to agents: ClosePane has the undo)",
    "files:deletePermanently": "deletes for good (refused to agents)",
    "tab:close": "closes a tab",
    "doctab:close": "closes a document",
    "files:closeTab": "closes a tab",
    "open:agent": "opens an agent",
};

/** The pane a pane-local command runs in, by the flags above. */
function paneFor(s) {
    if (s.pane === "files") return args.panes.files;
    if (s.pane === "editor") return args.panes.editor;
    if (s.pane === "doctabs") return args.panes.editor ?? args.panes.media;
    if (/^term:(copy|paste|clear)$/.test(s.command)) return args.panes.term;
    return undefined;
}

function needsPane(s) {
    return Boolean(s.pane) || /^term:(copy|paste|clear)$/.test(s.command);
}

async function connect(port) {
    let targets;
    try {
        targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    } catch (e) {
        throw new Error(`no CDP server on port ${port} (${e.message}): set --port or AGENTMUX_CDP_PORT`);
    }
    const page = targets.find((t) => t.type === "page" && /AgentMux/.test(t.title)) || targets.find((t) => t.type === "page");
    if (!page) throw new Error(`no page target on CDP port ${port}`);
    const ws = new WebSocket(page.webSocketDebuggerUrl);
    let seq = 0;
    const pending = new Map();
    ws.onmessage = (ev) => {
        const m = JSON.parse(ev.data);
        if (m.id && pending.has(m.id)) {
            pending.get(m.id)(m);
            pending.delete(m.id);
        }
    };
    await new Promise((r, j) => {
        ws.onopen = r;
        ws.onerror = () => j(new Error(`CDP websocket on port ${port} failed`));
    });
    const send = (method, params = {}) =>
        new Promise((r) => {
            const id = ++seq;
            pending.set(id, r);
            ws.send(JSON.stringify({ id, method, params }));
        });
    const evaluate = async (expression) => {
        const m = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
        if (m.result?.exceptionDetails) throw new Error(m.result.exceptionDetails.exception?.description ?? m.result.exceptionDetails.text);
        return m.result?.result?.value;
    };
    return { ws, send, evaluate };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const js = (v) => (v === undefined ? "undefined" : JSON.stringify(v));

/** Sends one planned key the way the host's press_keys does. */
async function sendKey(cdp, ev) {
    for (const type of ["rawKeyDown", "keyUp"]) {
        await cdp.send("Input.dispatchKeyEvent", { type, key: ev.key, code: ev.code, windowsVirtualKeyCode: ev.keyCode, modifiers: ev.modifiers });
    }
}

/** Escape, to close whatever a command opened (a dialog, the palette). */
async function settle(cdp) {
    await sleep(150);
    await sendKey(cdp, { key: "Escape", code: "Escape", keyCode: 27, modifiers: 0 });
    await sleep(100);
}

async function main() {
    let cdp;
    try {
        cdp = await connect(args.port);
    } catch (e) {
        console.error(e.message);
        process.exit(2);
    }
    const S = "window.__agentmux_shortcuts";
    if (!(await cdp.evaluate(`typeof ${S} === "object"`))) {
        console.error("this instance has no shortcut API (window.__agentmux_shortcuts): build it from a branch that has it");
        process.exit(2);
    }
    const list = (await cdp.evaluate(`${S}.list()`)).filter((s) => !args.only || args.only.test(s.command));
    const rows = [];
    for (const s of list) {
        const row = { command: s.command, label: s.label, keys: s.raw, l1: null, l2: [], note: "" };
        rows.push(row);
        if (MANUAL[s.command]) {
            row.note = `manual: ${MANUAL[s.command]}`;
            continue;
        }
        const target = paneFor(s);
        if (needsPane(s) && !target) {
            row.note = `skipped: needs a ${s.pane ?? "term"} pane (--${s.pane === "doctabs" ? "editor or --media" : (s.pane ?? "term")})`;
            continue;
        }
        if (args.layer !== "2") {
            const r = await cdp.evaluate(`${S}.run(${js(s.command)}, ${js(target)})`);
            row.l1 = r.ran ? { ok: true } : { ok: false, why: r.reason };
            await settle(cdp);
        }
        if (args.layer !== "1") {
            for (const keys of s.raw) {
                const plan = await cdp.evaluate(`${S}.plan(${js(keys)}, ${js(target)})`);
                if (plan.reason) {
                    row.l2.push({ keys, ok: false, why: plan.reason });
                    continue;
                }
                const before = await cdp.evaluate("Date.now()");
                for (const ev of plan.events) await sendKey(cdp, ev);
                await sleep(80);
                const last = await cdp.evaluate(`${S}.last()`);
                const got = last && last.at >= before ? last.command : null;
                row.l2.push({ keys, ok: got === s.command, got, modifiers: plan.modifiers });
                await settle(cdp);
            }
        }
    }
    cdp.ws.close();

    const failed = rows.filter((r) => (r.l1 && !r.l1.ok) || r.l2.some((k) => !k.ok));
    if (args.json) {
        console.log(JSON.stringify({ port: args.port, rows }, null, 2));
    } else {
        console.log(`${rows.length} shortcuts, ${failed.length} failing.\n`);
        console.log("| Command | L1 RunCommand | L2 keys | Note |\n|---|---|---|---|");
        for (const r of rows) {
            const l1 = r.l1 ? (r.l1.ok ? "pass" : `FAIL: ${r.l1.why}`) : "";
            const l2 = r.l2
                .map((k) => `\`${k.keys}\` ${k.ok ? "pass" : k.why ? `FAIL: ${k.why}` : `FAIL: resolved ${k.got ?? "nothing"}`}`)
                .join("<br>");
            console.log(`| \`${r.command}\` | ${l1} | ${l2} | ${r.note} |`);
        }
    }
    process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
    console.error(e.stack ?? e);
    process.exit(2);
});
