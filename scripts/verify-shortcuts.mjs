#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Checks every shortcut the Help pane lists in a running AgentMux instance,
// at layers L1 and L2 of docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md:
//   L1  the command runs (RunCommand's code path: `__agentmux_shortcuts.run`)
//   L1b after it ran, the window shows something different (scripts/verify-shortcuts-effect.mjs)
//   L2  each of its keys, sent as real key events with modifiers through the
//       DevTools protocol, resolves to that command (PressKeys' code path)
// L3, whether the OS lets the key reach the app at all, is the grab reports
// (scripts/gnome-grabs.mjs, scripts/mac-grabs.mjs) and a person at the keys.
//
// Usage:
//   node scripts/verify-shortcuts.mjs [--port N] [--layer 1|2|both] [--only REGEX]
//       [--files ID [--files-mutate]] [--editor ID] [--editor-text ID] [--media ID]
//       [--term ID] [--agent ID] [--json]
//
// Run it against a dev build or a throwaway instance (the port is
// AGENTMUX_CDP_PORT, 9223 by default in dev): it really runs each command,
// so it opens tabs, panes and a window, and changes focus. It leaves them
// behind: run it on a fresh instance, once. After a few runs the window holds
// hundreds of panes and handles keys slower than the verifier waits for. It never runs a
// command that closes or deletes, or one that opens an agent; those are
// listed as "manual".
//
// --agent and --term also give the rows that act on one kind of pane (agent:focusComposer,
// pane:find, pane:changeConnection) the pane they need.
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

import { appWindows, close, closeNewWindows, connect, js, MANUAL, S, sendKey, settle, sleep } from "./verify-shortcuts-cdp.mjs";
import { changed, describe, idleNoise, judge, OPPOSITE, SNAPSHOT } from "./verify-shortcuts-effect.mjs";
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
        else if (["--files", "--editor", "--editor-text", "--media", "--term", "--agent"].includes(k)) a.panes[k.slice(2)] = v();
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


/** Files rows that change files on disk: only with --files-mutate. */
const FILES_MUTATE = new Set(["files:newFolder", "files:rename", "files:trash", "files:cut", "files:paste", "files:undo"]);
/** Files rows that leave the folder or tab the pane was given: run last
 *  (files:newTabHere makes a new tab the pane's visible one). */
const FILES_NAVIGATE = new Set(["files:back", "files:forward", "files:up", "files:newTabHere"]);

/** A command to run first, so the row has something to act on. Escape is
 *  tried on the replace-pane confirmation (it never confirms: no Enter is
 *  sent, and the next Escape closes it anyway). */
const SETUP = {
    // The replace-pane confirmation is a prompt only the person can answer, which the API refuses to press
    // keys past; the palette is a dialog Escape closes.
    "app:escape": "view:command-palette",
    // The first and last tab are the same one until the active tab moves off it.
    "tab:goto:last": "tab:goto:1",
    "tab:goto:1": "tab:goto:last",
    // Cycling and moving need a second document.
    "doctab:next": "doctab:new",
    "doctab:prev": "doctab:new",
    "doctab:moveRight": "doctab:new",
    "doctab:moveLeft": "doctab:new",
};

/** Keys that are not shortcuts in a terminal: a row without `skipShell` in the key table hands the key to the
 *  shell there (ctrl+f is the shell's forward-char; ctrl+shift+f is the terminal's find). Only rows that are given the terminal
 *  pane as their target are listed: a global row's keys go wherever focus is, and the global rows start in a new tab. */
const KEY_TO_SHELL_IN_TERM = { "pane:find": ["ctrl+f"] };

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
    // Rows that act on one kind of pane, whichever pane has focus: the verifier must give them one.
    if (s.command === "agent:focusComposer") return args.panes.agent;
    if (s.command === "pane:find") return args.panes.term ?? args.panes["editor-text"] ?? args.panes.editor ?? args.panes.media;
    if (s.command === "pane:changeConnection") return args.panes.term;
    return undefined;
}

/** The pane kinds a row that has no `pane` of its own acts on, as the flag that gives it one. */
const PANE_KIND_FLAG = { "agent:focusComposer": "--agent", "pane:find": "--term, --editor or --media", "pane:changeConnection": "--term" };

function needsPane(s) {
    return Boolean(s.pane) || /^term:(copy|paste|clear)$/.test(s.command) || s.command in PANE_KIND_FLAG;
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
    // A CDP call can time out and throw: the tree must still go.
    const why = await moveFilesPane(cdp, blockId, tree.start).catch((e) => e.message);
    if (why) {
        // It may have got there before failing.
        await moveFilesPane(cdp, blockId, shown.path).catch(() => {});
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
        cdp = await connect(args.port, Object.values(args.panes)[0]);
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
    let tree = args.filesMutate && list.some((s) => s.pane === "files") ? await setUpTree(cdp).catch((e) => ({ refused: e.message })) : null;
    if (tree?.refused) console.error(`not running the rows that change files: ${tree.refused}`);
    let grid;
    let startedGlobal = false;
    try {
        for (const s of list) {
            if (grid === undefined && DIRECTIONAL.test(s.command)) grid = await makeGrid(cdp).catch(() => null);
            const now = grid && DIRECTIONAL.test(s.command) ? corners(await visiblePanes(cdp)) : null;
            s.gridPane = now?.[CORNER_FOR[s.command.split(":")[2]]];
            if (!startedGlobal && rank(s) === 3) {
                // The pane rows leave focus in a pane (a terminal, say) that takes some keys for itself: the
                // global rows start in a new tab, where focus is in the default pane.
                startedGlobal = true;
                if (Object.keys(args.panes).length) {
                    await cdp.evaluate(`${S}.run("tab:new")`);
                    await sleep(700);
                }
            }
            if (tree && treeDone(s)) {
                await tearDownTree(cdp, tree);
                tree = null;
            }
            const row = { command: s.command, label: s.label, keys: s.raw, l1: null, l2: [], note: "" };
            rows.push(row);
            const windowsBefore = new Set((await appWindows(args.port)).map((t) => t.id));
            try {
                await checkRow(cdp, s, row, tree);
                await noteOperatorPrompt(cdp, row);
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
    const target = paneFor(s) ?? s.gridPane;
    if (needsPane(s) && !target) {
        row.note = `skipped: needs ${s.pane ? `a ${s.pane} pane` : "a pane of the kind it acts on"} (${PANE_KIND_FLAG[s.command] ?? `--${s.pane === "doctabs" ? "editor or --media" : (s.pane ?? "term")}`})`;
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
        const first = await runWithEffect(cdp, s.command, target);
        row.l1 = first.ran ? { ok: true } : { ok: false, why: first.reason };
        let effect = first;
        // At the edge a move does nothing (the first tab, the left-most pane). Step the other way, which leaves
        // the edge, and run the row again: now it has room, so it must show something.
        const opposite = s.command.startsWith("tab:") ? OPPOSITE[s.command] : undefined;
        if (first.ran && first.effect && !first.effect.fields.length && opposite) {
            const back = await runWithEffect(cdp, opposite, target);
            if (back.ran && back.effect?.fields.length) {
                await settle(cdp);
                effect = await runWithEffect(cdp, s.command, target);
                row.effectRetried = opposite;
            }
        }
        if (effect.ran && effect.effect) {
            const { fields, ctx } = effect.effect;
            row.effect = describe(s.command, fields, true, ctx) + (row.effectRetried ? ` (after ${row.effectRetried}, which left the edge)` : "");
            const verdict = judge(s.command, fields, true, ctx);
            if (verdict) row.l1 = { ok: false, why: verdict };
        }
        await settle(cdp);
    }
    if (args.layer === "1") return;
    for (const keys of s.raw) {
        if (target && target === args.panes.term && KEY_TO_SHELL_IN_TERM[s.command]?.includes(keys)) {
            row.l2.push({ keys, ok: true, got: "goes to the shell in a terminal (not a shortcut there): not sent" });
            continue;
        }
        await setup();
        if (s.gridPane) {
            // The L1 run (or an earlier key) may have moved the panes: focus the corner that has a neighbour now.
            const now = corners(await visiblePanes(cdp));
            if (now) await cdp.evaluate(`${S}.focus(${js(now[CORNER_FOR[s.command.split(":")[2]]])})`);
        }
        const plan = await cdp.evaluate(`${S}.plan(${js(keys)}, ${js(target)})`);
        if (plan.reason) {
            row.l2.push({ keys, ok: false, why: plan.reason });
            continue;
        }
        if (await guard()) return;
        const before = await cdp.evaluate("Date.now()");
        const dialogs = await cdp.evaluate(DIALOGS);
        for (const ev of plan.events) {
            if (s.when?.includes("!textInputFocus")) await leaveTextField(cdp);
            await sendKey(cdp, ev);
        }
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

/** A prompt addressed to the person (an approval or a grant: a modal dialog with `data-operator-only`; the status
 *  bar's vault button carries the attribute too, and is not a prompt) takes no command or key until they answer it. When
 *  a row leaves one open, say which prompt, so the rows after it are not read as failures of their own. */
async function noteOperatorPrompt(cdp, row) {
    const text = await cdp.evaluate(`(() => { const m = document.querySelector("[role=dialog][data-operator-only]"); return m ? (m.textContent ?? "").trim().replace(/\\s+/g, " ").slice(0, 160) : null; })()`);
    if (text && !row.note) row.note = `FAIL: left a prompt for the user open: "${text}"`;
}

/** Rows that a text field takes the key from (`when: "!textInputFocus"`): the pane a key is planned for may put
 *  the caret in its own text field (a Shortcuts pane's filter), where the key is not a shortcut. A person
 *  would be on the pane, not in the field; this takes the focus out of the field before each key. */
async function leaveTextField(cdp) {
    const editable = `(() => { const a = document.activeElement; return !!a && (a.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(a.tagName)); })()`;
    for (let i = 0; i < 6 && (await cdp.evaluate(editable)); i++) {
        await cdp.evaluate("document.activeElement.blur()");
        await sleep(80);
    }
    return !(await cdp.evaluate(editable));
}

/** Runs a command and says what the window showed: `{ ran, reason, effect: { fields, ctx } | null }`. The
 *  effect is null when the page couldn't be read. */
async function runWithEffect(cdp, command, target) {
    const before = await snapshot(cdp);
    const r = await cdp.evaluate(`${S}.run(${js(command)}, ${js(target)})`);
    if (!r.ran || !before) return { ran: r.ran, reason: r.reason, effect: null };
    // A change can land a little late (a tab move goes through the server): look for up to about 1.5 s.
    let after = null;
    let fields = [];
    for (let waited = 0; waited < 1500; waited += 250) {
        await sleep(250);
        after = await snapshot(cdp);
        if (!after) return { ran: true, effect: null };
        fields = changed(before.snap, after.snap, { noisy: before.noisy });
        if (fields.length) break;
    }
    const ctx = { tabs: before.snap.tabs.split("|").length, panes: before.snap.tiles ? before.snap.tiles.split(";").length : 0 };
    return { ran: true, effect: { fields, ctx } };
}

/** The rows that act on a pane's neighbour in a direction (focus, swap, resize). They need a pane with a
 *  neighbour that way, so they run in a 2x2 grid made for them, from the corner that has one. */
const DIRECTIONAL = /^pane:(focus|swap|resize):(up|down|left|right)$/;
/** Which corner of the grid has a neighbour in each direction. */
const CORNER_FOR = { up: "bottomRight", down: "topLeft", left: "bottomRight", right: "topLeft" };

/** The visible panes of the active tab, by block id and place. */
async function visiblePanes(cdp) {
    return cdp.evaluate(`[...document.querySelectorAll(".tile-node")].filter((t) => t.offsetParent !== null).map((t) => {
        const r = t.getBoundingClientRect();
        return { id: t.querySelector("[data-blockid]")?.getAttribute("data-blockid"), x: r.x, y: r.y };
    }).filter((p) => p.id)`);
}

/** A new tab with its pane split into a 2x2 grid; the block ids by corner, or null if it didn't come out as
 *  four panes. */
async function makeGrid(cdp) {
    const run = async (command, pane) => cdp.evaluate(`${S}.run(${js(command)}, ${js(pane)})`);
    await run("tab:new");
    await sleep(600);
    const first = (await visiblePanes(cdp))[0]?.id;
    if (!first) return null;
    await run("split:right", first);
    await sleep(500);
    for (const p of await visiblePanes(cdp)) {
        await run("split:down", p.id);
        await sleep(500);
    }
    return corners(await visiblePanes(cdp));
}

/** The four panes by corner, from where they are now (a swap moves them); null unless there are four. */
function corners(panes) {
    if (panes.length !== 4) return null;
    const byY = [...panes].sort((a, b) => a.y - b.y || a.x - b.x);
    const [top, bottom] = [byY.slice(0, 2).sort((a, b) => a.x - b.x), byY.slice(2).sort((a, b) => a.x - b.x)];
    return { topLeft: top[0].id, topRight: top[1].id, bottomLeft: bottom[0].id, bottomRight: bottom[1].id };
}

/** What the window shows now, and whether it changes by itself (the clock, a spinner): two snapshots 150 ms
 *  apart. Null when the page can't be read. */
async function snapshot(cdp) {
    const read = async () => JSON.parse(await cdp.evaluate(SNAPSHOT));
    try {
        const a = await read();
        await sleep(150);
        const b = await read();
        b.windows = (await appWindows(args.port)).length;
        return { snap: b, noisy: idleNoise(a, b).includes("html") };
    } catch {
        return null;
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
            const effect = r.effect ? `<br>${r.effect}` : "";
            const l2 = r.l2
                .map((k) => `\`${k.keys}\` ${k.ok ? "pass" : k.why ? `FAIL: ${k.why}` : `FAIL: resolved ${k.got ?? "nothing"}`}`)
                .join("<br>");
            console.log(`| \`${r.command}\` | ${l1}${effect} | ${l2} | ${r.note} |`);
        }
    }
    process.exit(failed.length ? 1 : 0);
}

main().catch((e) => {
    console.error(e.stack ?? e);
    process.exit(2);
});
