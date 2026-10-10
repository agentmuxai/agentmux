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
//       [--files ID [--files-mutate]] [--editor ID] [--editor-text ID] [--media ID]
//       [--term ID] [--json]
//
// Run it against a dev build or a throwaway instance (the port is
// AGENTMUX_CDP_PORT, 9223 by default in dev): it really runs each command,
// so it opens tabs, panes and a window, and changes focus. It never runs a
// command that closes or deletes, or one that opens an agent; those are
// listed as "manual".
//
// A pane's own commands (files:*, editor:*, doctab:*, term:copy/paste/clear)
// run only when you pass a pane of that kind, by block id (WhoAmI or Layout
// shows them), all in one tab; doctab:* runs in --editor, or in --media if no
// editor is given. They run before the global rows, which switch tabs.
//
// The Files rows that change files on disk (new folder, rename, trash, cut,
// paste, undo) run only with --files-mutate. The script then makes its own
// tree under the OS temp folder (scripts/verify-shortcuts-temp-tree.mjs),
// moves the Files pane into it by typing the path (Ctrl+L), and checks before
// each of those rows, and each of their keys, that the pane still shows a
// folder inside it; a row that would act anywhere else is refused. At the end
// it moves the pane back and removes the tree (trashed items stay in the OS
// Trash). The pane must be on this computer, not an SSH host.
//
// files:copy, files:cut and term:copy put text on the system clipboard,
// replacing what was there.
//
// Editor rows: a Markdown file opens in preview, where save, Save As and find
// don't apply, and editor:togglePreview needs Markdown. Pass --editor on a
// Markdown file (preview, document tabs) and --editor-text on a plain-text
// file (save, Save As, find); without --editor-text those run in --editor.
//
// Exit code: 0 when everything checked passes, 1 when anything fails, 2 on a
// setup error (no CDP target, or a build without the shortcut API).

import { insideTree, isWithinPath, makeTempTree, removeTempTree } from "./verify-shortcuts-temp-tree.mjs";

const args = (() => {
    const a = { port: Number(process.env.AGENTMUX_CDP_PORT) || 9223, layer: "both", only: null, panes: {}, filesMutate: false, json: false };
    const argv = process.argv.slice(2);
    for (let i = 0; i < argv.length; i++) {
        const k = argv[i];
        const v = () => argv[++i];
        if (k === "--port") a.port = Number(v());
        else if (k === "--layer") a.layer = v();
        else if (k === "--only") a.only = new RegExp(v());
        else if (k === "--json") a.json = true;
        else if (k === "--files-mutate") a.filesMutate = true;
        else if (["--files", "--editor", "--editor-text", "--media", "--term"].includes(k)) a.panes[k.slice(2)] = v();
        else {
            console.error(`unknown argument: ${k}`);
            process.exit(2);
        }
    }
    if (a.filesMutate && !a.panes.files) {
        console.error("--files-mutate needs --files ID: the Files pane to move into the temp tree");
        process.exit(2);
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
    "pane:voice": "asks for the microphone",
    "files:mention": "puts paths in an agent's message box",
    "files:openInNewTab": "opens the selected files in their apps",
    // A line break in the clipboard (files:copy puts one between paths) would
    // run each line in the shell.
    "term:paste": "pastes the clipboard into a shell",
};

/** Files rows that change files on disk: only with --files-mutate. */
const FILES_MUTATE = new Set(["files:newFolder", "files:rename", "files:trash", "files:cut", "files:paste", "files:undo"]);
/** Files rows that leave the folder or tab the pane was given: run last
 *  (files:newTabHere makes a new tab the pane's visible one). */
const FILES_NAVIGATE = new Set(["files:back", "files:forward", "files:up", "files:newTabHere"]);

/** A command to run first, so the row has something to act on. Escape is
 *  tried on the replace-pane confirmation (it never confirms: no Enter is
 *  sent, and the next Escape closes it anyway). */
const SETUP = {
    "app:escape": "pane:replaceWithLauncher",
    // Cycling and moving need a second document.
    "doctab:next": "doctab:new",
    "doctab:prev": "doctab:new",
    "doctab:moveRight": "doctab:new",
    "doctab:moveLeft": "doctab:new",
};

/** Rows a dialog handles itself, so its key never reaches the dispatcher:
 *  the key passes when it closes the dialog. */
const CLOSES_DIALOG = new Set(["app:escape"]);
const DIALOGS = `document.querySelectorAll("[role=dialog]").length`;

/** Editor rows that apply to a document's source, not a Markdown preview. */
const EDITOR_TEXT = new Set(["editor:save", "editor:saveAs", "editor:find"]);

/** The pane a pane-local command runs in, by the flags above. */
function paneFor(s) {
    if (s.pane === "files") return args.panes.files;
    if (s.pane === "editor") return (EDITOR_TEXT.has(s.command) && args.panes["editor-text"]) || args.panes.editor;
    if (s.pane === "doctabs") return args.panes.editor ?? args.panes.media;
    if (/^term:(copy|paste|clear)$/.test(s.command)) return args.panes.term;
    return undefined;
}

function needsPane(s) {
    return Boolean(s.pane) || /^term:(copy|paste|clear)$/.test(s.command);
}

/** Run order: the editor's own rows (before the document-tab rows leave an
 *  untitled document active in --editor), the other pane rows (Files
 *  navigation last), then global rows. */
function rank(s) {
    if (!needsPane(s)) return 3;
    if (FILES_NAVIGATE.has(s.command)) return 2;
    if (FILES_MUTATE.has(s.command)) return 1;
    if (s.pane === "editor") return -1;
    return 0;
}

/** Where the temp tree is done with: files:newTabHere makes a new tab the
 *  Files pane's visible one (moving it back would then need to reach a
 *  pane tab, K1), and the global rows switch tabs. */
const treeDone = (s) => rank(s) === 3 || s.command === "files:newTabHere";

async function attach(target) {
    const ws = new WebSocket(target.webSocketDebuggerUrl);
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
        ws.onerror = () => j(new Error(`CDP websocket for ${target.url} failed`));
    });
    // Every call gives up after CALL_TIMEOUT_MS: a page that goes away (a
    // window closing, a dev reload) never answers, and the run would hang.
    const send = (method, params = {}) =>
        new Promise((r, j) => {
            const id = ++seq;
            const timer = setTimeout(() => {
                pending.delete(id);
                j(new Error(`${method} got no answer in ${CALL_TIMEOUT_MS / 1000} s`));
            }, CALL_TIMEOUT_MS);
            pending.set(id, (m) => {
                clearTimeout(timer);
                r(m);
            });
            ws.send(JSON.stringify({ id, method, params }));
        });
    /** Sends a call without waiting for its answer (one that may never come). */
    const fire = (method, params = {}) => ws.send(JSON.stringify({ id: ++seq, method, params }));
    const evaluate = async (expression) => {
        const m = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
        if (m.result?.exceptionDetails) throw new Error(m.result.exceptionDetails.exception?.description ?? m.result.exceptionDetails.text);
        return m.result?.result?.value;
    };
    return { ws, send, fire, evaluate };
}

const CALL_TIMEOUT_MS = 15_000;

async function connect(port) {
    // A dev build's pages reload whenever a file in the checkout changes, and
    // have no shortcut API until the app has started again: keep trying for
    // about 30 s.
    for (let attempt = 1; ; attempt++) {
        try {
            return await connectOnce(port);
        } catch (e) {
            if (attempt >= 10 || e.message.startsWith("no CDP server")) throw e;
            await sleep(3000);
        }
    }
}

/**
 * The window to test: one with the shortcut API that is shown (macOS keeps
 * hidden pool windows with zero width) and, when panes were given, holds the
 * first of them.
 */
async function connectOnce(port) {
    let targets;
    try {
        targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    } catch (e) {
        throw new Error(`no CDP server on port ${port} (${e.message}): set --port or AGENTMUX_CDP_PORT`);
    }
    const pane = Object.values(args.panes)[0];
    const probe = `typeof window.__agentmux_shortcuts === "object" && window.innerWidth > 0${
        pane ? ` && !!document.querySelector(${JSON.stringify(`[data-blockid="${pane}"]`)})` : ""
    }`;
    let sawApi = false;
    for (const t of targets.filter((t) => t.type === "page")) {
        let cdp;
        try {
            cdp = await attach(t);
            if (await cdp.evaluate(probe)) return cdp;
            if (await cdp.evaluate(`typeof window.__agentmux_shortcuts === "object"`)) sawApi = true;
        } catch {
            // Not a page we can drive; try the next.
        }
        await close(cdp);
    }
    if (!sawApi) throw new Error("no window here has the shortcut API (window.__agentmux_shortcuts): build from a branch that has it");
    throw new Error(pane ? `no shown window holds pane ${pane}` : "no shown window has the shortcut API");
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Closes a session and waits for it, so the process doesn't exit mid-close. */
async function close(cdp) {
    if (!cdp || cdp.ws.readyState === WebSocket.CLOSED) return;
    const closed = new Promise((r) => cdp.ws.addEventListener("close", r, { once: true }));
    cdp.ws.close();
    await Promise.race([closed, sleep(1000)]);
}

/** The extra app windows open now (window:new may claim a pre-warmed one,
 *  which then drops its `pool=1`), by DevTools target id. */
async function appWindows(port) {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    return targets.filter((t) => t.type === "page" && /[?&]windowLabel=window-/.test(t.url) && !/[?&]pool=1/.test(t.url));
}

/** Closes the app windows a row opened (window:new), so runs don't pile them up. */
async function closeNewWindows(port, before) {
    await sleep(300);
    for (const t of await appWindows(port)) {
        if (before.has(t.id)) continue;
        const w = await attach(t).catch(() => null);
        if (!w) continue;
        // The window closes before it can answer, so don't wait for a reply.
        w.fire("Runtime.evaluate", { expression: "window.api?.closeWindow?.()" });
        await close(w);
    }
}

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

const S = "window.__agentmux_shortcuts";

/** The folder a Files pane shows, and its SSH host ("" on this computer),
 *  from its visible view's data attributes (files-view.tsx); null if none. */
async function filesShown(cdp, blockId) {
    return cdp.evaluate(`(() => {
        const pane = document.querySelector(${js(`[data-blockid="${blockId}"]`)});
        const view = pane && [...pane.querySelectorAll(".files-view")].find((v) => v.offsetParent !== null);
        return view ? { path: view.getAttribute("data-path") ?? "", connection: view.getAttribute("data-connection") ?? "" } : null;
    })()`);
}

const samePath = (a, b) => isWithinPath(a, b) && isWithinPath(b, a);

/** Moves a Files pane to `dest` the way a person would: Ctrl+L's path box,
 *  the path, Enter. Null when it got there, else why not. */
async function moveFilesPane(cdp, blockId, dest) {
    if (!(await cdp.evaluate(`${S}.focus(${js(blockId)})`))) return "the Files pane isn't in the active tab of this window";
    const r = await cdp.evaluate(`${S}.run("files:editPath", ${js(blockId)})`);
    if (!r.ran) return `files:editPath didn't run (${r.reason})`;
    const inPathBox = `document.activeElement?.classList.contains("files-path-input") ?? false`;
    for (let waited = 0; waited < 1000 && !(await cdp.evaluate(inPathBox)); waited += 50) await sleep(50);
    if (!(await cdp.evaluate(inPathBox))) return "the path box didn't take focus";
    // The box opens with its text selected, so this replaces it.
    await cdp.send("Input.insertText", { text: dest });
    await sendKey(cdp, { key: "Enter", code: "Enter", keyCode: 13, modifiers: 0 });
    for (let waited = 0; waited < 3000; waited += 100) {
        const shown = await filesShown(cdp, blockId);
        if (shown && samePath(shown.path, dest)) return null;
        await sleep(100);
    }
    return `it didn't arrive at ${dest}`;
}

/**
 * --files-mutate: makes the temp tree and moves the Files pane into it.
 * Returns the tree and the folder the pane showed before, or `{ refused }`
 * saying why the rows that change files can't run.
 */
async function setUpTree(cdp) {
    const blockId = args.panes.files;
    const shown = await filesShown(cdp, blockId);
    if (!shown) return { refused: `pane ${blockId} shows no Files view` };
    if (shown.connection) return { refused: `the Files pane is on ${shown.connection}, and the temp tree is on this computer` };
    const tree = makeTempTree();
    const why = await moveFilesPane(cdp, blockId, tree.start);
    if (why) {
        removeTempTree(tree.root);
        return { refused: `couldn't move the Files pane into the temp tree: ${why}` };
    }
    return { ...tree, original: shown.path };
}

/** Moves the Files pane back where it was and removes the temp tree. */
async function tearDownTree(cdp, tree) {
    if (!tree?.root) return;
    const why = tree.original ? await moveFilesPane(cdp, args.panes.files, tree.original).catch((e) => e.message) : null;
    if (why) console.error(`couldn't move the Files pane back to ${tree.original}: ${why}`);
    removeTempTree(tree.root);
    console.error(`removed the temp tree ${tree.root}; anything files:trash moved is in the OS Trash`);
}

/** Null when the Files pane shows a folder inside the temp tree, else why the row is refused. */
async function outsideTree(cdp, tree) {
    const shown = await filesShown(cdp, args.panes.files);
    if (shown && !shown.connection && insideTree(shown.path, tree.root)) return null;
    return `FAIL: refused, the Files pane shows ${shown?.path || "nothing"}, outside the temp tree ${tree.root}`;
}

async function main() {
    let cdp;
    try {
        cdp = await connect(args.port);
    } catch (e) {
        console.error(e.message);
        process.exit(2);
    }
    const list = (await cdp.evaluate(`${S}.list()`))
        .filter((s) => !args.only || args.only.test(s.command))
        .map((s, i) => ({ s, i }))
        .sort((a, b) => rank(a.s) - rank(b.s) || a.i - b.i)
        .map(({ s }) => s);
    const rows = [];
    // The Files rows run in the temp tree, which goes away before the first
    // row that would leave the pane out of reach (treeDone).
    let tree = args.filesMutate && list.some((s) => s.pane === "files") ? await setUpTree(cdp) : null;
    if (tree?.refused) console.error(`not running the rows that change files: ${tree.refused}`);
    try {
        for (const s of list) {
            if (tree && treeDone(s)) {
                await tearDownTree(cdp, tree);
                tree = null;
            }
            const row = { command: s.command, label: s.label, keys: s.raw, l1: null, l2: [], note: "" };
            rows.push(row);
            const windowsBefore = new Set((await appWindows(args.port)).map((t) => t.id));
            try {
                await checkRow(cdp, s, row, tree);
                await closeNewWindows(args.port, windowsBefore);
            } catch (e) {
                // One row's failure doesn't end the run.
                row.note = `FAIL: threw ${String(e.message ?? e).split("\n")[0]}`;
                await settle(cdp).catch(() => {});
            }
        }
    } finally {
        await tearDownTree(cdp, tree);
    }
    await close(cdp);
    report(rows);
}

/** L1, then L2 for each key, into `row`; or a note saying why not. `tree` is
 *  the temp tree with --files-mutate (or why there isn't one). */
async function checkRow(cdp, s, row, tree) {
    if (MANUAL[s.command]) {
        row.note = `manual: ${MANUAL[s.command]}`;
        return;
    }
    const mutates = FILES_MUTATE.has(s.command);
    if (mutates && !args.filesMutate) {
        row.note = "skipped: changes files on disk (pass --files-mutate: the script then works in its own temp folder)";
        return;
    }
    if (mutates && !tree?.root) {
        row.note = `skipped: changes files on disk, and ${tree?.refused ?? "there's no temp tree"}`;
        return;
    }
    const target = paneFor(s);
    if (needsPane(s) && !target) {
        row.note = `skipped: needs a ${s.pane ?? "term"} pane (--${s.pane === "doctabs" ? "editor or --media" : (s.pane ?? "term")})`;
        return;
    }
    if (target && !(await cdp.evaluate(`${S}.focus(${js(target)})`))) {
        row.note = `skipped: pane ${target} isn't in the active tab of this window`;
        return;
    }
    const setup = async () => {
        if (!SETUP[s.command]) return;
        await cdp.evaluate(`${S}.run(${js(SETUP[s.command])}, ${js(target)})`);
        await sleep(150);
    };
    // Rechecked before every run and key: an earlier row may have moved the pane.
    const guard = async () => {
        if (!mutates) return false;
        const refused = await outsideTree(cdp, tree);
        if (refused) row.note = refused;
        return Boolean(refused);
    };
    if (args.layer !== "2") {
        await setup();
        if (await guard()) return;
        const r = await cdp.evaluate(`${S}.run(${js(s.command)}, ${js(target)})`);
        row.l1 = r.ran ? { ok: true } : { ok: false, why: r.reason };
        await settle(cdp);
    }
    if (args.layer === "1") return;
    for (const keys of s.raw) {
        await setup();
        const plan = await cdp.evaluate(`${S}.plan(${js(keys)}, ${js(target)})`);
        if (plan.reason) {
            row.l2.push({ keys, ok: false, why: plan.reason });
            continue;
        }
        if (await guard()) return;
        const before = await cdp.evaluate("Date.now()");
        const dialogs = await cdp.evaluate(DIALOGS);
        for (const ev of plan.events) await sendKey(cdp, ev);
        // A handler can note what it resolved a little later: wait up to 500 ms.
        let got = null;
        for (let waited = 0; waited < 500 && got === null; waited += 50) {
            await sleep(50);
            const last = await cdp.evaluate(`${S}.last()`);
            if (last && last.at >= before) got = last.command;
        }
        const closedDialog = CLOSES_DIALOG.has(s.command) && dialogs > 0 && (await cdp.evaluate(DIALOGS)) < dialogs;
        row.l2.push({ keys, ok: got === s.command || closedDialog, got: closedDialog && got !== s.command ? "the dialog's own handler" : got, modifiers: plan.modifiers });
        await settle(cdp);
    }
}

function report(rows) {
    const failed = rows.filter((r) => (r.l1 && !r.l1.ok) || r.l2.some((k) => !k.ok) || r.note.startsWith("FAIL"));
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
