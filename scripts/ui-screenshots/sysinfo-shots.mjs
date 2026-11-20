// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sysinfo suite: one shot per plot the Sysinfo pane can show, for the docs' System Metrics page
// (agentmux-docs `src/content/docs/system-metrics.md`). Run with
//
//   node scripts/ui-screenshots/capture.mjs --suite sysinfo --port N
//
// Each shot opens a new window tab, opens Sysinfo there, maximizes its pane (the widget suite's own steps), then
// writes the pane's plot choice the way the pane's menu does (`sysinfo:type` and `graph:metrics` for a simple
// view; `sysinfo:type` = Overview and `sysinfo:focus` for an Advanced one) and crops to the pane. The shot's id is
// the docs image's name, so `publish.mjs` finds it.
//
// setup() starts a little local activity (a busy loop and file writes in the temp folder) for the window the
// charts show, so the plots are not flat lines, and waits until the pane has a window of samples. The activity
// stops when the run ends. The shots show this machine's real readings: the Advanced Disk plot lists its volumes
// (mount points and free space), so every image is reviewed before it is published.

import { spawn } from "node:child_process";
import { shots as widgetShots } from "./widget-shots.mjs";

const AREA_TYPE = "Overview";
/** Seconds of samples a 2-minute plot needs before it fills its width. */
const WARM_UP_SECS = 135;

/** The Advanced Disk plot lists every mounted volume. A machine that runs AppImages lists each one's temporary mount
 *  (`/tmp/.mount_AgentXXXXXX`, a read-only image with no free space), which says nothing about the machine's disks
 *  and is not what a reader's machine shows. This keeps them out of the shot: an observer removes those entries
 *  whenever the list is drawn again, until the shot's cleanup. */
const HIDE_APP_MOUNTS = `(() => {
    const run = () => {
        for (const li of document.querySelectorAll('.sysinfo-focus-volumes li')) {
            const name = li.querySelector('.sysinfo-focus-volume-name')?.textContent ?? '';
            if (/^\\/tmp\\/\\.mount_/.test(name)) li.remove();
        }
    };
    window.__sysinfoShotsTidy?.disconnect();
    window.__sysinfoShotsTidy = new MutationObserver(run);
    window.__sysinfoShotsTidy.observe(document.body, { childList: true, subtree: true });
    run();
})()`;

/** Docs image id, the plot's saved type and the extra meta it needs, and what the docs say it shows. */
export const PLOTS = [
    { id: "sysinfo-cpu", type: "CPU", metrics: ["cpu"], description: "The CPU plot: overall CPU use over the last two minutes." },
    { id: "sysinfo-all-cores", type: "All CPU", metrics: "cores", description: "The All cores plot: one small plot per core." },
    { id: "sysinfo-memory", type: "Mem", metrics: ["mem:used"], description: "The Memory plot: memory in use against the machine's total." },
    { id: "sysinfo-network", type: "Net", metrics: ["net:bytestotal"], description: "The Network plot: total network throughput." },
    { id: "sysinfo-network-in-out", type: "Net (Sent/Recv)", metrics: ["net:bytessent", "net:bytesrecv"], description: "The Network sent and received plots, one per direction." },
    { id: "sysinfo-disk", type: "Disk I/O", metrics: ["disk:total"], description: "The Disk plot: total disk throughput." },
    { id: "sysinfo-disk-read-write", type: "Disk I/O (R/W)", metrics: ["disk:read", "disk:write"], description: "The Disk read and write plots, one per direction." },
    { id: "sysinfo-cpu-memory", type: "CPU + Mem", metrics: ["cpu", "mem:used"], description: "The CPU and memory plots, one above the other." },
    { id: "sysinfo-cpu-memory-network", type: "CPU + Mem + Net", metrics: ["cpu", "mem:used", "net:bytestotal"], description: "The CPU, memory and network plots in one column." },
    { id: "sysinfo-advanced-cpu", type: AREA_TYPE, focus: "cpu", description: "The Advanced CPU plot: CPU, the busiest core and a heatmap of every core." },
    { id: "sysinfo-advanced-memory", type: AREA_TYPE, focus: "mem", description: "The Advanced Memory plot: used and available memory." },
    { id: "sysinfo-advanced-disk", type: AREA_TYPE, focus: "disk", tidy: HIDE_APP_MOUNTS, description: "The Advanced Disk plot: read above and write below, then each alone, with the volumes." },
    { id: "sysinfo-advanced-network", type: AREA_TYPE, focus: "net", description: "The Advanced Network plot: in above and out below, then each alone." },
    { id: "sysinfo-advanced-everything", type: AREA_TYPE, focus: "everything", description: "The Everything plot: a tile per area over shared charts for CPU, cores, memory, network and disk." },
];

/** The pane's block id, from the selector the widget shot found it by. */
const BLOCK_ID_OF = (selector) =>
    `document.querySelector(${JSON.stringify(selector)})?.querySelector('[data-blockid]')?.getAttribute('data-blockid') ?? null`;

const base = widgetShots.find((s) => s.id === "widget-sysinfo");
if (!base) throw new Error("the widget suite has no Sysinfo shot to build on");

/** Local activity for the warm-up: one busy loop and file writes to the temp folder. Returns a stopper. */
function startActivity() {
    const busy = spawn(process.execPath, ["-e", "const end=Date.now()+" + (WARM_UP_SECS + 240) * 1000 + ";let x=0;while(Date.now()<end){for(let i=0;i<1e6;i++)x+=Math.sqrt(i);}"], { stdio: "ignore" });
    const writer = spawn(
        process.execPath,
        [
            "-e",
            `const fs=require('fs'),os=require('os'),p=require('path').join(os.tmpdir(),'sysinfo-shots-'+process.pid);const b=Buffer.alloc(8<<20,1);const end=Date.now()+${(WARM_UP_SECS + 240) * 1000};
             while(Date.now()<end){fs.writeFileSync(p,b);fs.fsyncSync(fs.openSync(p,'r+'));const t=Date.now()+700;while(Date.now()<t){}}
             try{fs.unlinkSync(p)}catch{}`,
        ],
        { stdio: "ignore" }
    );
    return () => {
        for (const c of [busy, writer]) {
            try {
                c.kill("SIGTERM");
            } catch {
                // Already gone.
            }
        }
    };
}

let stopActivity = null;

export async function setup(session) {
    stopActivity = startActivity();
    process.once("exit", () => stopActivity?.());
    console.log(`  warming up for ${WARM_UP_SECS} s so the plots have a window of samples and some activity...`);
    await session.wait(WARM_UP_SECS * 1000);
}

function plotShot(p) {
    let blockId = null;
    return {
        id: p.id,
        title: p.id.replace(/^sysinfo-/, "").replace(/-/g, " "),
        description: p.description,
        retries: 1,
        settleMs: 3000,
        containsWorkspaceData: "review",
        prep: async (session) => {
            await base.prep(session);
            const selector = base.selector();
            blockId = await session.evaluate(BLOCK_ID_OF(selector));
            if (!blockId) throw new Error("the Sysinfo pane's block id was not found");
            // "All cores" lists every core the machine has, as the pane's own menu saves it.
            const metrics = p.metrics === "cores" ? await coreKeys(session) : p.metrics;
            const meta = p.focus
                ? { "sysinfo:type": p.type, "sysinfo:focus": p.focus }
                : { "sysinfo:type": p.type, "graph:metrics": metrics };
            await session.evaluate(`RpcApi.SetMetaCommand(TabRpcClient, { oref: "block:${blockId}", meta: ${JSON.stringify(meta)} })`);
            await session.wait(1500);
            if (p.tidy) await session.evaluate(p.tidy);
        },
        selector: base.selector,
        cleanup: async (session) => {
            await session.evaluate("window.__sysinfoShotsTidy?.disconnect(); window.__sysinfoShotsTidy = null").catch(() => {});
            await base.cleanup(session);
        },
    };
}

/** The cpu:N keys of the machine: one per logical CPU, at most the 32 the pane plots. */
async function coreKeys(session) {
    const n = await session.evaluate("navigator.hardwareConcurrency");
    return Array.from({ length: Math.min(Number(n) || 8, 32) }, (_, i) => `cpu:${i}`);
}

export const shots = PLOTS.map(plotShot);
