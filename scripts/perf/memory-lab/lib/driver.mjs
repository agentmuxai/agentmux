// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Starts an isolated AgentMux instance and drives it over CDP for the memory
// lab: a demo Claude account, window tabs, scripted agents (the fake Claude of
// scripts/ui-screenshots/fake-claude, so the panes run the real pipeline:
// srv, the translator, the stores and the renderer, with no model, no account
// and no tokens) and the messages that make them play a turn.
//
// The instance has its own data folder (AGENTMUX_HOME_OVERRIDE) and its own
// CDP port, and is never the machine's own ~/.agentmux (installFakeClaude
// refuses that), nor port 9222.

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, openSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { findOpenLines, OPEN_TAP, READ_OPENS } from "./agentopen.mjs";
import { writeSettings } from "./settings.mjs";
import { DETACH_FRAMES, FRAME_PROBE, mergeFrameRecords, READ_FRAMES } from "./frames.mjs";
import { DETACH_LONG_TASKS, LONG_TASK_OBSERVER, mergeLongTaskRecords, READ_LONG_TASKS } from "./longtasks.mjs";
import { timedCall } from "./sendsegments.mjs";
import { buildInstanceEnv } from "./instanceenv.mjs";
import { installFakeClaude } from "../../../ui-screenshots/install-fake-claude.mjs";
import { CdpSession, findMainTarget } from "../../../ui-screenshots/lib/session.mjs";
import { openNewTab } from "../../../ui-screenshots/widget-shots.mjs";

/** The demo account the scripted agents run as. */
export const DEMO = { id: "6f1d2c3b-0a9e-4c7d-8b5a-acme00000001", email: "dev@acme.example" };

/** How many agents go in each window tab, for a total number of panes. */
export function layoutFor(panes) {
    if (panes === 12) return [4, 2, 4, 2]; // the live-like layout of the report: 12 agents over 4 tabs
    const tabs = [];
    for (let left = panes; left > 0; left -= 4) tabs.push(Math.min(4, left));
    return tabs;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
/** A short name for a page expression the driver evaluates: the RPC it calls, or its first characters. */
export function commandName(expr) {
    const text = String(expr);
    return /RpcApi\.(\w+?)(?:Command)?\(/.exec(text)?.[1] ?? /rpcCall\(\s*"([^"]+)"/.exec(text)?.[1] ?? text.replace(/\s+/g, " ").slice(0, 60);
}

/** How long a CDP command may take before the driver gives up on it. */
const EVAL_TIMEOUT_MS = 90_000;

/**
 * The executable of a packaged build, from the folder (or bundle) `task package` made:
 * Windows `agentmux.exe`; macOS `AgentMux.app/Contents/MacOS/agentmux-launcher` (the
 * `.app` itself, or the folder holding it); Linux's extracted tarball `agentmux.sh`, else
 * `agentmux`. A Linux AppImage is passed to `--exe` directly.
 */
export function portableExe(dir, platform = process.platform, exists = existsSync) {
    if (platform === "win32") return join(dir, "agentmux.exe");
    if (platform === "darwin") {
        const inBundle = (app) => join(app, "Contents", "MacOS", "agentmux-launcher");
        if (dir.endsWith(".app")) return inBundle(dir);
        return exists(join(dir, "AgentMux.app")) ? inBundle(join(dir, "AgentMux.app")) : join(dir, "agentmux-launcher");
    }
    return exists(join(dir, "agentmux.sh")) ? join(dir, "agentmux.sh") : join(dir, "agentmux");
}

/** Whether something already answers CDP on `port`. */
export async function portInUse(port) {
    try {
        await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(2000) });
        return true;
    } catch {
        return false;
    }
}

/**
 * Starts the instance with a data folder of its own, emptied first: a run
 * starts from nothing, so the same options always build the same layout.
 * `env` is what it gets besides its own variables: any AGENTMUX_* variable of
 * the shell that started the lab is dropped, so it can't join another
 * instance's channel or data folder.
 *
 * The CDP port is only the instance's preferred one (it takes another when
 * that is busy), so a busy port is refused up front: the lab would drive the
 * other instance.
 * @returns {Promise<{ pid: number, port: number, home: string, stop: () => void }>}
 */
export async function launch({ exe, home, port, script, log, env = process.env, extraEnv = {}, keepHome = false }) {
    if (Number(port) === 9222) throw new Error("port 9222 is the production instance's");
    if (await portInUse(port)) {
        throw new Error(`something already answers on CDP port ${port} (another instance, or one a lab run left behind); stop it or pick another --port`);
    }
    if (!keepHome) rmSync(home, { recursive: true, force: true });
    mkdirSync(home, { recursive: true });
    const built = buildInstanceEnv(env, { AGENTMUX_HOME_OVERRIDE: home, AGENTMUX_CDP_PORT: String(port), FAKE_CLAUDE_SCRIPT: script }, extraEnv);
    const out = openSync(log, "a");
    const child = spawn(exe, [], {
        env: built.env,
        stdio: ["ignore", out, out],
        windowsHide: false,
        detached: process.platform !== "win32", // its own process group, so stopTree reaches everything under it
    });
    let spawnError = null;
    child.on("error", (e) => {
        spawnError = e;
    });
    try {
        const end = Date.now() + 120_000;
        for (;;) {
            if (spawnError) throw spawnError;
            if (Date.now() > end) throw new Error(`${exe} didn't open a page on CDP port ${port} in 2 minutes (see ${log})`);
            try {
                const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
                if (targets.some((t) => t.type === "page")) break;
            } catch {
                // not listening yet
            }
            await sleep(1000);
        }
        await sleep(8000); // the first run writes the data folder's shared/ part
    } catch (e) {
        await stopTree(child.pid); // don't leave it holding the port and the data folder
        throw e;
    }
    return { pid: child.pid, port, home, envRecord: built.record, stop: () => stopTree(child.pid) };
}

/**
 * `launch`, with app settings in place before the instance reads them: the first start (on an empty data folder) shows where this
 * build keeps its settings, the file is written there, the instance is stopped and started again on the same folder (see lib/settings.mjs).
 * Without settings it is `launch`. The result has `settingsFiles`, the paths written.
 */
export async function launchWithSettings(opts, settings) {
    if (!settings || !Object.keys(settings).length) return await launch(opts);
    const first = await launch(opts);
    let files;
    try {
        files = writeSettings(opts.home, settings);
    } finally {
        await first.stop();
    }
    await sleep(2000); // the port and the data folder's lock are free again
    const second = await launch({ ...opts, keepHome: true });
    return Object.assign(second, { settingsFiles: files });
}

/** Whether anything is left in the process group `pid` leads. */
const groupAlive = (pid) => {
    try {
        process.kill(-pid, 0);
        return true;
    } catch {
        return false;
    }
};

/**
 * Ends the instance: the process it was started as, and everything under it,
 * by process id. On macOS and Linux that is its process group: SIGTERM, then
 * SIGKILL for whatever is still there after `graceMs`; it returns when the
 * group is gone, so the port and the data folder are free for the next run.
 */
export async function stopTree(pid, graceMs = 5000) {
    if (process.platform === "win32") {
        try {
            execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
        } catch {
            // already gone
        }
        return;
    }
    try {
        process.kill(-pid, "SIGTERM");
    } catch {
        return; // already gone
    }
    const end = Date.now() + graceMs;
    while (groupAlive(pid) && Date.now() < end) await sleep(100);
    if (groupAlive(pid)) {
        try {
            process.kill(-pid, "SIGKILL");
        } catch {
            // gone in between
        }
        while (groupAlive(pid)) await sleep(50);
    }
}

/** Puts the scripted Claude in the instance's data folder (after its first start). */
export function installScript(home, scriptPath) {
    return installFakeClaude(home, scriptPath);
}

/** The driver of one running instance. */
export async function connect(port, home, workDir) {
    let session = await CdpSession.connect((await findMainTarget(port)).webSocketDebuggerUrl);
    // The page's main thread is shared by every agent pane: with 12 agents starting a turn at once (and a slower machine), one RPC
    // from the page can take longer than the 10 s the screenshot scripts allow. A hang is still caught, at 90 s.
    session.defaultTimeoutMs = EVAL_TIMEOUT_MS;
    // The slowest page evaluation of the run, so a machine that is close to the limit shows in the result before it fails.
    let slowestEvalMs = 0;
    const longTaskArchive = []; // the records of documents a reload has since replaced
    const frameArchive = []; // and the slow-frame probe's, the same way
    // The commands that took 5 s or more, the 30 slowest if there are more (slowEvalsTotal says how many there were).
    let slowEvals = [];
    let slowEvalsTotal = 0;
    const ev = async (expr) => {
        const t0 = Date.now();
        try {
            return await session.evaluate(expr);
        } catch (e) {
            // Which command it was: a timeout from srv ("EC-TIME") and one from the driver look alike without it.
            if (e instanceof Error && !e.message.includes("[command: ")) e.message += ` [command: ${commandName(expr)}, after ${Date.now() - t0} ms]`;
            throw e;
        } finally {
            const ms = Date.now() - t0;
            slowestEvalMs = Math.max(slowestEvalMs, ms);
            if (ms >= 5000) {
                slowEvalsTotal++;
                slowEvals.push({ command: commandName(expr), ms });
                if (slowEvals.length > 30) slowEvals = slowEvals.sort((a, b) => b.ms - a.ms).slice(0, 30);
            }
        }
    };
    const rpc = (command, data) => ev(`TabRpcClient.rpcCall(${JSON.stringify(command)}, ${JSON.stringify(data)})`);
    const activeTab = () => ev(`document.querySelector('.tab.active')?.getAttribute('data-tab-id') ?? null`);
    const agents = [];
    mkdirSync(workDir, { recursive: true });

    return {
        slowestEvalMs: () => slowestEvalMs,
        slowEvals: () => ({ total: slowEvalsTotal, slowest: [...slowEvals].sort((a, b) => b.ms - a.ms) }),
        get session() {
            return session;
        },
        agents,
        eval: ev,

        /** The main window's page was replaced (a recycled renderer): connect to the new one. */
        async reconnect(timeoutMs = 90_000) {
            session.close();
            const end = Date.now() + timeoutMs;
            for (;;) {
                try {
                    session = await CdpSession.connect((await findMainTarget(port)).webSocketDebuggerUrl);
                    session.defaultTimeoutMs = EVAL_TIMEOUT_MS;
                    await session.evaluate("1");
                    await session.evaluate(LONG_TASK_OBSERVER).catch(() => {}); // the reload dropped the observer
                    await session.evaluate(FRAME_PROBE).catch(() => {}); // and the slow-frame probe
                    return;
                } catch (e) {
                    if (Date.now() > end) throw new Error(`no main window page on port ${port} after ${timeoutMs / 1000} s: ${e.message}`);
                    await sleep(1000);
                }
            }
        },

        /** Runs a command from the palette, as a user would: open it, type, Enter. */
        async paletteCommand(query) {
            await session.pressKey("p", { ctrl: true, shift: true });
            await sleep(600);
            await session.typeText(query);
            await sleep(600);
            await session.pressKey("Enter");
        },

        /** A demo Claude account the scripted agents are linked to, then a reload. */
        async setupAccount() {
            const accountDir = join(home, "demo-accounts", "claude");
            mkdirSync(accountDir, { recursive: true });
            writeFileSync(
                join(accountDir, ".credentials.json"),
                JSON.stringify({ claudeAiOauth: { accessToken: "demo-placeholder", refreshToken: "demo-placeholder", expiresAt: 4102444800000, scopes: ["user:inference"] } }) + "\n"
            );
            await ev(`RpcApi.UpsertIdentityAccountCommand(TabRpcClient, { id: ${JSON.stringify(DEMO.id)}, name: ${JSON.stringify(DEMO.email)},
                provider: "claude", kind: "oauth", display_name: ${JSON.stringify(DEMO.email)}, status: "valid",
                secret_ref: { backend: "oauth_config_dir", dir: ${JSON.stringify(accountDir.replace(/\\/g, "/"))} } })`);
            await ev("location.reload()").catch(() => {});
            await sleep(6000);
        },

        /** A new window tab, which becomes the active one. The menu click is flaky under load, so it retries. */
        async newTab() {
            const t = { tabId: null };
            for (let attempt = 1; ; attempt++) {
                try {
                    await openNewTab(session, t);
                    return t.tabId;
                } catch (e) {
                    if (attempt >= 4) throw e;
                    await session.pressKey("Escape");
                    await sleep(1500);
                }
            }
        },

        /** `n` scripted Claude agents, opened in the active tab. */
        async addAgents(n, prefix) {
            const tabId = await activeTab();
            for (let i = 0; i < n; i++) {
                const name = `${prefix}-${agents.length + 1}`;
                const def = await rpc("createagent", { name, provider: "claude", working_directory: workDir });
                const id = def?.id ?? def?.agent?.id;
                if (!id) throw new Error("createagent returned " + JSON.stringify(def));
                await ev(`RpcApi.LinkAgentIdentityCommand(TabRpcClient, { agent_id: ${JSON.stringify(id)}, account_id: ${JSON.stringify(DEMO.id)}, provider: "claude" })`);
                const opened = await rpc("agent.open", { agent_id: id, tab_id: tabId, focus: false });
                agents.push({ name, id, blockId: opened.block_id, tabId });
                await sleep(1500);
            }
        },

        /** Makes the tabs and agents of `layout` (agents per tab), and leaves the first tab active. */
        async build(layout) {
            for (let t = 0; t < layout.length; t++) {
                if (t > 0) {
                    await this.newTab();
                    await sleep(2000);
                }
                await this.addAgents(layout[t], `t${t + 1}`);
            }
            await this.activate(0);
        },

        async activate(index) {
            const id = await ev(`[...document.querySelectorAll('.tab[data-tab-id]')][${index}]?.getAttribute('data-tab-id')`);
            await session.clickSelector(`.tab[data-tab-id="${id}"]`);
            await sleep(800);
        },

        /**
         * Closes an agent's pane and opens the agent again in the same tab, as a user does (close the pane, then My Agents or the
         * palette), and waits for the app's own `[agent-open]` line for the new pane (and, up to `restTimeoutS`, its `[agent-open-rest]`).
         * The agent keeps its conversation: it is the store's copy that the new pane reads. Changes `agent.blockId` to the new pane's.
         */
        async reopen(agent, { settleMs = 1500, timeoutS = 60, restTimeoutS = 30 } = {}) {
            await session.clickSelector(`.tab[data-tab-id="${agent.tabId}"]`);
            await sleep(800);
            const closed = await ev(`(async () => {
              const layout = window.getLayoutModelForStaticTab && window.getLayoutModelForStaticTab();
              const node = layout && layout.getNodeByBlockId(${JSON.stringify(agent.blockId)});
              if (!node) return "no-node";
              await layout.closeNode(node.id);
              return "closed";
            })()`);
            if (closed !== "closed") throw new Error(`could not close ${agent.name}'s pane (${closed})`);
            for (let waited = 0; ; waited += 250) {
                if (!(await ev(`!!document.querySelector('[data-blockid="${agent.blockId}"]')`))) break;
                if (waited > 15_000) throw new Error(`${agent.name}'s pane was still on screen 15 s after it was closed`);
                await sleep(250);
            }
            await sleep(settleMs); // srv deletes the block and frees the agent's lease
            await ev(OPEN_TAP);
            const rpcStart = Date.now();
            const opened = await rpc("agent.open", { agent_id: agent.id, tab_id: agent.tabId, focus: true });
            const rpcMs = Date.now() - rpcStart;
            if (!opened?.block_id) throw new Error(`agent.open for ${agent.name} returned ${JSON.stringify(opened)}`);
            agent.blockId = opened.block_id;
            let lines = { open: null, rest: null };
            let tapRewraps = 0;
            const look = async () => {
                const record = await ev(READ_OPENS);
                tapRewraps = record?.rewraps ?? tapRewraps;
                lines = findOpenLines(record, agent.blockId);
            };
            // The open's line comes when the page has been quiet for a while after the reveal; the rest's line, when the older history has loaded.
            const until = Date.now() + timeoutS * 1000;
            while (Date.now() < until && !lines.open) {
                await sleep(500);
                await look();
            }
            const restUntil = Date.now() + restTimeoutS * 1000;
            // A build without the first-screen open loads the whole history before it paints, and logs no rest line.
            while (lines.open && lines.open.first_screen != null && !lines.rest && Date.now() < restUntil) {
                await sleep(500);
                await look();
            }
            return { rpcStart, rpcMs, ...lines, tapRewraps };
        },

        /** One message to every agent (or to `blockIds`): each plays the next scripted turn. `staggerMs` offsets them, as real agents aren't in lockstep. */
        async send(blockIds = agents.map((a) => a.blockId), { staggerMs = 0, text = "Continue with the next step of the refactor." } = {}) {
            // Each send's start (epoch ms, as srv's log timestamps are UTC) and how long the command took, to set beside srv's lines.
            const timings = [];
            for (const b of blockIds) {
                const mid = `lab-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
                const start = Date.now();
                let page = null;
                try {
                    page = await ev(timedCall(`RpcApi.AgentInputCommand(TabRpcClient, { blockid: ${JSON.stringify(b)}, message: ${JSON.stringify(text)}, message_id: ${JSON.stringify(mid)} })`));
                    if (page?.error) throw new Error(`AgentInput failed: ${page.error}`);
                } finally {
                    // the page's own clock and stopwatch for this call: see lib/sendsegments.mjs
                    timings.push({ start, ms: Date.now() - start, pageStart: page?.pageStart ?? null, pageMs: page?.pageMs ?? null, respArrival: page?.respArrival ?? null, eventAgeMs: page?.eventAgeMs ?? null });
                }
                if (staggerMs) await sleep(staggerMs);
            }
            return timings;
        },
        /** Starts recording the page's long tasks (main-thread tasks of 50 ms or more); a reload loses it, so call again after one. */
        async installLongTasks() {
            const installed = await ev(LONG_TASK_OBSERVER);
            // The slow-frame probe goes in beside it. Report-only: on a Chromium without Long Animation Frames it records none and says so.
            await ev(FRAME_PROBE).catch(() => {});
            return installed;
        },
        /** The long tasks recorded since `installLongTasks`, across any reloads archived with `archiveLongTasks`, or null. */
        async longTasks() {
            return mergeLongTaskRecords([...longTaskArchive, await ev(READ_LONG_TASKS)]);
        },
        /** The slow frames recorded since `installLongTasks`, across any reloads archived with `archiveLongTasks`, or null. */
        async frames() {
            return mergeFrameRecords([...frameArchive, await ev(READ_FRAMES)]);
        },
        /** Keeps what the page has recorded so far and starts a fresh record: call before a reload, which would destroy it. */
        async archiveLongTasks() {
            const current = await ev(READ_LONG_TASKS);
            if (current) longTaskArchive.push(current);
            await ev(DETACH_LONG_TASKS);
            await ev(LONG_TASK_OBSERVER);
            const frames = await ev(READ_FRAMES).catch(() => null);
            if (frames) frameArchive.push(frames);
            await ev(DETACH_FRAMES).catch(() => {});
            await ev(FRAME_PROBE).catch(() => {});
        },

        /**
         * How many tool rows each agent's transcript shows: a played turn of the
         * scripted conversation has 8-20 of them (the fake Claude names its tool
         * ids `toolu_...`), an agent that never played has none.
         */
        async toolRows() {
            const counts = [];
            for (const a of agents) {
                const n = await ev(`(document.querySelector('[data-blockid="${a.blockId}"]')?.querySelectorAll('[data-node-id^="toolu_"]') ?? []).length`);
                counts.push({ name: a.name, blockId: a.blockId, tools: n });
            }
            return counts;
        },

        /** The names of the agent definitions srv holds that the lab made (`t<tab>-<n>`, with any collision suffix). */
        async labAgentNames() {
            const list = await rpc("listagents", {});
            const defs = Array.isArray(list) ? list : (list?.agents ?? list?.definitions ?? []);
            return defs.map((d) => d.name).filter((n) => /^t\d+-\d+/.test(String(n)));
        },

        /** How many agents are in the middle of a turn. */
        busy: () => ev(`document.querySelectorAll('.agent-working-row:not(.agent-working-row--worked)').length`),

        close: () => session.close(),
    };
}
