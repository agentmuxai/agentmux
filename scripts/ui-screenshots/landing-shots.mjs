// Shots for agentmux.ai, the landing site. Each one proves a specific claim on a specific
// section, so it shows real content, not a blank first-run pane.
//
// Run against an isolated instance prepared with demo-env.mjs (the terminals open in the demo
// project): node capture.mjs --suite landing --port <port>
//
// Terminal text is drawn on a canvas, which redaction can't reach. So terminal shots only run
// fixed, read-only commands in the demo project, and their verify reads the terminal buffers
// back and refuses the shot if anything but the expected output is on screen.
import { openNewTab, openWidget, closeTab, VISIBLE_BLOCK_IDS } from "./widget-shots.mjs";
import { demoPathProblem } from "./demo-env.mjs";

/** The demo project the terminals work in (demo-env.mjs's --demo). */
const DEMO = process.env.DEMO_DIR || "D:/demo/acme-web";
const DEMO_PROMPT = `PS ${DEMO.replace(/\//g, "\\")}`;

const BLOCK_IDS = `[...document.querySelectorAll('.pane-stack [data-blockid]')].map(e => e.getAttribute('data-blockid'))`;

/** The commands each terminal runs, and a line of their output the verify expects to see. */
// Short enough not to wrap in a third of the window. `tree`'s first two lines name the disk's
// volume and serial number, so they're skipped.
const TERMINALS = [
    { command: "tree /F /A src | Select -Skip 2", expect: "App.tsx" },
    { command: "Get-Content package.json", expect: '"name"' },
    { command: "ls -R -File src, tests | ft Name, Length", expect: "cart.test.ts" },
];

const PSREADLINE_OFF = "Set-PSReadLineOption -PredictionSource None -HistorySaveStyle SaveNothing; Clear-Host";
const MENU_OPEN = `[...document.querySelectorAll('.menu')].some((m) => m.offsetParent)`;

/** Types a line into a terminal's shell, as if typed and Enter pressed. */
async function sendLine(session, blockId, line) {
    const data = Buffer.from(`${line}\r`).toString("base64");
    await session.evaluate(`RpcApi.ControllerInputCommand(TabRpcClient, { blockid: ${JSON.stringify(blockId)}, inputdata64: ${JSON.stringify(data)} })`);
}

/** Keeps a terminal's TermWrap once it mounts. The app only exposes the newest one, as
 *  `window.term` (term.tsx), so each is stashed by block id as it opens. */
const stashTerminal = (blockId) => `(() => {
    if (window.term?.blockId !== ${JSON.stringify(blockId)}) return false;
    (window.__landingTerms ??= {})[${JSON.stringify(blockId)}] = window.term;
    return true;
})()`;

/** The text in a terminal's buffer, through xterm's own API (the canvas has no DOM text). */
const terminalText = (blockId) => `(() => {
    const b = window.__landingTerms?.[${JSON.stringify(blockId)}]?.terminal?.buffer?.active;
    if (!b) return null;
    const lines = [];
    for (let i = 0; i < b.length; i++) lines.push(b.getLine(i)?.translateToString(true) ?? "");
    return lines.join("\\n");
})()`;

function landingTerminals() {
    const tab = { tabId: null };
    let terminals = [];
    return {
        id: "landing-terminals",
        title: "Terminals in a split",
        description: "Three PowerShell terminals side by side in one tab, each running a command in the demo project: a file tree, package.json and a file listing. For the landing site's \"The terminal you know.\"",
        retries: 1,
        settleMs: 800,
        containsWorkspaceData: "review",
        prep: async (session) => {
            const problem = demoPathProblem(DEMO);
            if (problem) throw new Error(`DEMO_DIR: ${problem}`);
            await session.setViewport({ width: 1280, height: 800, scale: 1 });
            await session.pressKey("Escape");
            tab.tabId = null;
            await openNewTab(session, tab);
            // A new tab starts with the default layout; close its panes so the terminals fill it.
            for (const id of (await session.evaluate(VISIBLE_BLOCK_IDS)).split(",").filter(Boolean)) {
                await session.clickSelector(`.pane-stack:has([data-blockid="${id}"]) .block-frame-default-close`);
                for (let t = 0; t < 20 && (await session.evaluate(BLOCK_IDS)).includes(id); t++) await session.wait(150);
            }
            // Open three terminals the way a person would (pinned icon, More list, menu or palette);
            // each opens a pane in this tab.
            terminals = [];
            for (let i = 0; i < TERMINALS.length; i++) {
                const before = new Set(await session.evaluate(BLOCK_IDS));
                await openWidget(session, { icon: "terminal", label: "Terminal" });
                let id = null;
                for (let t = 0; t < 40 && !id; t++) {
                    await session.wait(250);
                    id = (await session.evaluate(BLOCK_IDS)).find((b) => !before.has(b)) ?? null;
                }
                if (!id) throw new Error(`terminal ${i + 1} didn't open`);
                let stashed = false;
                for (let t = 0; t < 40 && !stashed; t++) {
                    stashed = await session.evaluate(stashTerminal(id));
                    if (!stashed) await session.wait(250);
                }
                if (!stashed) throw new Error(`terminal ${i + 1} didn't mount`);
                terminals.push(id);
            }
            // However the terminal was opened (the palette's Open Terminal ignores the demo
            // widget's cmd:cwd), start each shell over in the demo project.
            for (const id of terminals) {
                await session.evaluate(`(async () => {
                    const t = window.__landingTerms[${JSON.stringify(id)}].terminal;
                    await RpcApi.SetMetaCommand(TabRpcClient, { oref: "block:${id}", meta: { "cmd:cwd": ${JSON.stringify(DEMO)} } });
                    await RpcApi.ControllerResyncCommand(TabRpcClient, { tabid: ${JSON.stringify(tab.tabId)}, blockid: "${id}", forcerestart: true, rtopts: { termsize: { rows: t.rows, cols: t.cols } } });
                })()`);
            }
            await session.wait(3000);
            // PSReadLine predicts from, and saves to, the capturing user's real history file: its
            // grey suggestions could show a real past command. Turn both off in this session, then
            // Clear-Host (which also clears the scrollback) so the shot starts on a clean screen.
            for (const id of terminals) await sendLine(session, id, PSREADLINE_OFF);
            await session.wait(2000);
            for (let i = 0; i < TERMINALS.length; i++) await sendLine(session, terminals[i], TERMINALS[i].command);
            await session.wait(2500);
            // openWidget's fallbacks can leave the main menu open; Escape goes to the focused
            // terminal, so close it with its own button.
            if (await session.evaluate(MENU_OPEN)) await session.clickSelector(".hamburger-btn");
            await session.parkMouse();
            await session.wait(300);
        },
        verify: async (session) => {
            if (await session.evaluate(MENU_OPEN)) return "a menu is still open";
            const visible = (await session.evaluate(VISIBLE_BLOCK_IDS)).split(",").filter(Boolean);
            for (let i = 0; i < TERMINALS.length; i++) {
                if (!visible.includes(terminals[i])) return `terminal ${i + 1} isn't visible`;
                const text = await session.evaluate(terminalText(terminals[i]));
                if (text == null) return `can't read terminal ${i + 1}'s buffer, so its content can't be checked`;
                if (!text.includes(TERMINALS[i].expect)) return `terminal ${i + 1} doesn't show its expected output (${TERMINALS[i].expect})`;
                // Nothing from the capturing machine: the prompt must be the demo project's.
                const prompts = text.split("\n").filter((l) => /^PS [A-Z]:\\/.test(l));
                if (prompts.length === 0 || prompts.some((l) => !l.startsWith(DEMO_PROMPT))) return `terminal ${i + 1} has a prompt outside the demo project`;
            }
            return true;
        },
        selector: "body",
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (tab.tabId) await closeTab(session, tab.tabId);
            tab.tabId = null;
            terminals = [];
            await session.clearViewport();
        },
    };
}

/** A block opened with `pane.open` (the RPC the app's own panes use), waited for until it mounts. */
async function openPane(session, args) {
    const before = new Set(await session.evaluate(BLOCK_IDS));
    await session.evaluate(`TabRpcClient.rpcCall("pane.open", ${JSON.stringify(args)}, {})`);
    for (let t = 0; t < 40; t++) {
        const id = (await session.evaluate(BLOCK_IDS)).find((b) => !before.has(b));
        if (id) return id;
        await session.wait(250);
    }
    throw new Error(`a ${args.view} pane didn't open`);
}

/** The workspace an agent lives in: the default layout (agent picker, a live CPU graph, Swarm)
 *  with the Hangar file browser and a terminal on the demo project beside it. */
function landingWorkspace() {
    const tab = { tabId: null };
    let terminal = null;
    let files = null;
    const COMMAND = TERMINALS[2];
    return {
        id: "landing-workspace",
        title: "The workspace",
        description: "A whole window: the agent picker, a live CPU graph and Swarm (the default layout), with the Hangar file browser on the demo project and a PowerShell terminal in it, side by side. For the landing site's workspace sections (Home, Features).",
        retries: 1,
        settleMs: 1200,
        containsWorkspaceData: "review",
        prep: async (session) => {
            const problem = demoPathProblem(DEMO);
            if (problem) throw new Error(`DEMO_DIR: ${problem}`);
            await session.setViewport({ width: 1280, height: 800, scale: 1 });
            await session.pressKey("Escape");
            tab.tabId = null;
            await openNewTab(session, tab);
            // The new tab has the default layout; the Hangar and the terminal open as columns beside it.
            files = await openPane(session, { view: "files", cwd: DEMO });
            terminal = await openPane(session, { view: "term", cwd: DEMO });
            let stashed = false;
            for (let t = 0; t < 40 && !stashed; t++) {
                stashed = await session.evaluate(stashTerminal(terminal));
                if (!stashed) await session.wait(250);
            }
            if (!stashed) throw new Error("the terminal didn't mount");
            // As landing-terminals does: restart the shell at the size the pane laid out to, or
            // xterm can hold output it never paints.
            await session.evaluate(`(async () => {
                const t = window.__landingTerms[${JSON.stringify(terminal)}].terminal;
                await RpcApi.ControllerResyncCommand(TabRpcClient, { tabid: ${JSON.stringify(tab.tabId)}, blockid: ${JSON.stringify(terminal)}, forcerestart: true, rtopts: { termsize: { rows: t.rows, cols: t.cols } } });
            })()`);
            await session.wait(3000);
            await sendLine(session, terminal, PSREADLINE_OFF);
            await session.wait(1500);
            await sendLine(session, terminal, COMMAND.command);
            await session.wait(2500);
            if (await session.evaluate(MENU_OPEN)) await session.clickSelector(".hamburger-btn");
            await session.parkMouse();
            await session.wait(300);
        },
        verify: async (session) => {
            if (await session.evaluate(MENU_OPEN)) return "a menu is still open";
            const visible = (await session.evaluate(VISIBLE_BLOCK_IDS)).split(",").filter(Boolean);
            if (!visible.includes(files)) return "the Hangar isn't visible";
            if (!visible.includes(terminal)) return "the terminal isn't visible";
            if (visible.length < 5) return `only ${visible.length} panes are visible; the default layout's three plus the Hangar and the terminal are expected`;
            const text = await session.evaluate(terminalText(terminal));
            if (text == null) return "can't read the terminal's buffer, so its content can't be checked";
            if (!text.includes(COMMAND.expect)) return `the terminal doesn't show its expected output (${COMMAND.expect})`;
            const prompts = text.split("\n").filter((l) => /^PS [A-Z]:\\/.test(l));
            if (prompts.length === 0 || prompts.some((l) => !l.startsWith(DEMO_PROMPT))) return "the terminal has a prompt outside the demo project";
            return true;
        },
        selector: "body",
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (tab.tabId) await closeTab(session, tab.tabId);
            tab.tabId = null;
            terminal = null;
            files = null;
            await session.clearViewport();
        },
    };
}

export const shots = [landingTerminals(), landingWorkspace()];
