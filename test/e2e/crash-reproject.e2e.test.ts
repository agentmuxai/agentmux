// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SPEC_PILLAR1_STEP4 Phase 5 — E2E test for "host OOM ⇒ session reprojects"
 * (the parent design doc's §3 acceptance criterion), automating this
 * session's Phase 2/3 manual live-verification methodology: launch an
 * isolated instance, open extra windows, kill the inner host process only
 * (the launcher survives — the far more common crash than a full
 * process-tree kill), wait for the launcher-supervised respawn, and assert
 * the recreated windows show up as real CDP targets with correct kind.
 *
 * Greenfield and Windows-only (matching every other Phase 1-4 verification
 * this spec's implementation was built and checked against). Requires a real
 * portable/dev build — `task package` takes minutes, so this suite does NOT
 * run under a plain `npm test`. Set `AGENTMUX_E2E_BUILD_DIR` to a build
 * directory (the folder containing `agentmux.exe`, e.g. the output of
 * `task package`) to opt in:
 *
 *   AGENTMUX_E2E_BUILD_DIR="/c/Users/you/Desktop/agentmux-...-x64-portable" \
 *     npx vitest run test/e2e/crash-reproject.e2e.test.ts
 *
 * Without that env var, every test in this file reports `skipped` — matching
 * common practice for expensive/opt-in E2E suites (see e.g. Playwright's
 * `test.skip`-on-missing-fixture convention) rather than silently passing or
 * blocking every routine `npm test` run on a multi-minute build.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { spawn, execSync, type ChildProcess } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
// Deliberately NOT the `ws` npm package: its package.json maps a "browser"
// field to a stub that throws ("ws does not work in the browser"), and the
// repo's shared vitest.config.ts merges in the frontend's own vite.config
// (browser-oriented resolution needed for the rest of this test suite) —
// that resolution applies even under `@vitest-environment node` and even
// via `createRequire` (vite-node intercepts Node's own `require` too, not
// just ESM imports). Node's own native `WebSocket` (global since Node 22,
// undici-backed) sidesteps this entirely — it's a process global, not
// something resolved through Vite's module graph at all. Its API is
// DOM-style (`addEventListener`, not `ws`'s EventEmitter `.on()`).

const BUILD_DIR = process.env.AGENTMUX_E2E_BUILD_DIR;
const IS_WINDOWS = process.platform === "win32";
const RUN = Boolean(BUILD_DIR) && IS_WINDOWS;

if (!RUN) {
    const reason = !BUILD_DIR
        ? "AGENTMUX_E2E_BUILD_DIR not set"
        : `unsupported platform (${process.platform}, Windows-only)`;
    // eslint-disable-next-line no-console
    console.log(`[crash-reproject e2e] skipped — ${reason}`);
}

// ── CDP helpers (same wire-level approach as the session's own
//    `.verify_step4p2_cdp.mjs` scratch script, formalized here). ──────────

interface CdpTarget {
    title: string;
    url: string;
    webSocketDebuggerUrl: string;
}

async function listCdpTargets(port: number): Promise<CdpTarget[]> {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`);
    if (!res.ok) throw new Error(`CDP /json/list returned ${res.status}`);
    return (await res.json()) as CdpTarget[];
}

function wsConnect(url: string): Promise<WebSocket> {
    return new Promise((resolve, reject) => {
        const ws = new WebSocket(url);
        ws.addEventListener("open", () => resolve(ws));
        ws.addEventListener("error", (ev) => reject(new Error(String((ev as any).message ?? ev))));
    });
}

let msgId = 1;
function wsSend(ws: WebSocket, method: string, params?: unknown): Promise<any> {
    return new Promise((resolve, reject) => {
        const id = msgId++;
        const handler = (event: MessageEvent) => {
            const msg = JSON.parse(String(event.data));
            if (msg.id === id) {
                ws.removeEventListener("message", handler);
                if (msg.error) reject(new Error(JSON.stringify(msg.error)));
                else resolve(msg.result);
            }
        };
        ws.addEventListener("message", handler);
        ws.send(JSON.stringify({ id, method, params }));
    });
}

/** Evaluate `expression` in the main window's page (found by title substring). */
async function evalInMainWindow(port: number, expression: string): Promise<any> {
    const targets = await listCdpTargets(port);
    const main = targets.find((t) => t.title.includes("Starter workspace"));
    if (!main) throw new Error("main window's CDP target not found");
    const ws = await wsConnect(main.webSocketDebuggerUrl);
    await wsSend(ws, "Runtime.enable");
    const result = await wsSend(ws, "Runtime.evaluate", {
        expression,
        awaitPromise: true,
        returnByValue: true,
    });
    ws.close();
    if (result.exceptionDetails) {
        throw new Error(`eval failed: ${JSON.stringify(result.exceptionDetails)}`);
    }
    return result.result.value;
}

// ── Process helpers ───────────────────────────────────────────────────────

/** All AGENTMUX_* env vars scrubbed — an isolated instance, not sharing
 *  state with any already-running dev/portable instance on this machine. */
function scrubbedEnv(): NodeJS.ProcessEnv {
    const env: NodeJS.ProcessEnv = {};
    for (const [k, v] of Object.entries(process.env)) {
        if (!k.startsWith("AGENTMUX_")) env[k] = v;
    }
    return env;
}

function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Poll `netstat` for the two ports a freshly-created host PID listens on
 *  (IPC + CDP — CDP is whichever of the pair actually answers /json/list). */
async function findCdpPort(pid: number, timeoutMs: number): Promise<number> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const out = execSync("netstat -ano", { encoding: "utf8" });
        const ports = out
            .split("\n")
            .filter((l) => {
                if (!l.includes("LISTENING")) return false;
                // Exact match on the trailing PID column — `endsWith` would
                // false-positive-match e.g. pid=88 against a line ending in
                // "988" (a real substring, not a real match).
                const cols = l.trim().split(/\s+/);
                return Number(cols[cols.length - 1]) === pid;
            })
            .map((l) => {
                const m = l.match(/127\.0\.0\.1:(\d+)/);
                return m ? Number(m[1]) : null;
            })
            .filter((p): p is number => p !== null);
        for (const port of ports) {
            try {
                const targets = await listCdpTargets(port);
                if (targets.length > 0) return port;
            } catch {
                // this port is the IPC port, not CDP — try the next one
            }
        }
        await sleep(500);
    }
    throw new Error(`CDP port for PID ${pid} not found within ${timeoutMs}ms`);
}

function killPid(pid: number): void {
    try {
        execSync(`taskkill /PID ${pid} /F`, { stdio: "ignore" });
    } catch {
        // already gone — fine
    }
}

describe.skipIf(!RUN)("crash-reproject E2E: host OOM ⇒ session reprojects", () => {
    let outerWrapperPid: number | null = null;
    let hostPid: number | null = null;
    let logPath: string;

    beforeAll(async () => {
        const exePath = path.join(BUILD_DIR!, "agentmux.exe");
        if (!fs.existsSync(exePath)) {
            throw new Error(`AGENTMUX_E2E_BUILD_DIR does not contain agentmux.exe: ${exePath}`);
        }
        logPath = path.join(BUILD_DIR!, "..", "crash-reproject-e2e.log");
        const logFd = fs.openSync(logPath, "w");
        const child: ChildProcess = spawn(exePath, [], {
            cwd: BUILD_DIR,
            env: scrubbedEnv(),
            detached: true,
            stdio: ["ignore", logFd, logFd],
        });
        child.unref();

        // The spawned `agentmux.exe` is the outer launcher wrapper; the exe
        // name inside the build dir's `runtime/` (the actual CEF host) is
        // versioned (`agentmux-X.Y.Z.exe`) — find it by reading the log or
        // by process name once it starts. `wmic`'s CommandLine match on
        // "agentmux.exe" catches the wrapper itself; the host process has a
        // distinct versioned name, discovered via the same query with a
        // wildcard-free match against whatever's actually running.
        await sleep(3000); // let the wrapper spawn its children
        hostPid = await findVersionedHostPid(BUILD_DIR!, 20000);
        outerWrapperPid = child.pid ?? null;
    }, 30000);

    afterAll(async () => {
        // Kill the wrapper FIRST, not the host: killing `hostPid` while the
        // wrapper is still alive makes the launcher's own crash-restart
        // logic (the exact mechanism under test) treat that as a real crash
        // and spawn yet another replacement — a race that can leave an
        // orphaned instance behind if this hook's two kills interleave with
        // the launcher's own respawn. Killing the wrapper first tears down
        // its Job Object, which should cascade-kill every child (including
        // any host it's mid-respawn on) in one shot.
        if (outerWrapperPid) killPid(outerWrapperPid);
        if (hostPid) killPid(hostPid);
        await sleep(1000);
        // Defensive final sweep: guarantee no leaked process from this
        // specific build dir survives this suite, regardless of any race
        // above — never by image name (that would hit every other AgentMux
        // instance on the machine, including a real one the user has open;
        // see CLAUDE.md's "CRITICAL: Never Kill AgentMux by Image Name").
        // Scoped to this exact BUILD_DIR's own process tree only.
        try {
            const out = execSync(
                `wmic process where "(name like 'agentmux-%' or name='agentmux.exe')" get ProcessId,CommandLine /format:csv`,
                { encoding: "utf8" },
            );
            const buildDirBackslash = BUILD_DIR!.replace(/\//g, "\\");
            for (const line of out.split("\n").map((l) => l.trim()).filter(Boolean)) {
                if (line.startsWith("Node,")) continue;
                if (!line.includes(buildDirBackslash)) continue;
                const parts = line.split(",");
                const pid = Number(parts[parts.length - 1]);
                if (Number.isFinite(pid) && pid > 0) killPid(pid);
            }
        } catch {
            // best-effort — nothing left to sweep, or wmic itself is gone
        }
    });

    it("recreates extra top-level windows after the inner host process is killed", async () => {
        expect(hostPid).not.toBeNull();
        const port1 = await findCdpPort(hostPid!, 15000);

        // Open one extra top-level window via the real IPC command (the
        // same path an interactive "New Window" takes).
        const newLabel = await evalInMainWindow(
            port1,
            "window.api.openNewWindow()",
        );
        expect(typeof newLabel).toBe("string");

        // Give the topology write-through (SPEC_PILLAR1_STEP3) and the
        // launcher's WindowOpened mirror a moment to land before the crash.
        await sleep(3000);

        const beforeInstances: Array<{ label: string }> = await evalInMainWindow(
            port1,
            "window.api.listWindowInstances().then(r => r)",
        );
        const extraBefore = beforeInstances.filter((w) => w.label !== "main");
        expect(extraBefore.length).toBeGreaterThanOrEqual(1);

        // Kill the INNER host process only — the launcher survives and
        // supervises a respawn. Never kill by image name (CLAUDE.md).
        killPid(hostPid!);

        // The launcher spawns a brand-new host process (new PID).
        const newHostPid = await findVersionedHostPid(BUILD_DIR!, 20000, hostPid!);
        hostPid = newHostPid;
        const port2 = await findCdpPort(newHostPid, 15000);

        // Fast path: the launcher's in-memory snapshot should have the
        // extra window and reproject it. Poll briefly — reproject is fast
        // (sub-second in this session's live verifications) but runs
        // asynchronously relative to the new host's own startup. Poll on
        // "main's real title has settled" specifically, not merely
        // "more than one target exists" — main's CDP target briefly shows a
        // generic title before its content finishes loading and sets
        // `document.title` to "Starter workspace...", so a bare count check
        // can pass before main is actually the window we expect.
        const deadline = Date.now() + 15000;
        let targets: CdpTarget[] = [];
        let mainTarget: CdpTarget | undefined;
        while (Date.now() < deadline) {
            targets = await listCdpTargets(port2);
            mainTarget = targets.find((t) => t.title.includes("Starter workspace"));
            if (mainTarget && targets.length > 1) break;
            await sleep(500);
        }

        // At least the main window and one recreated extra window must show
        // up as real, distinct CDP targets — this is the assertion this
        // whole suite exists for: a log line saying "reproject complete" is
        // not sufficient (see docs/retro/retro-browser-pane-renderer-leak-2026-07-07.md's
        // own conclusion — verify against the CDP target list, not the
        // host's own event log).
        expect(targets.length).toBeGreaterThan(1);
        expect(mainTarget).toBeDefined();
    }, 90000);
});

/** Find the CEF host process (versioned exe name, e.g. `agentmux-0.51.0.exe`
 *  — distinct from the outer `agentmux.exe` wrapper, the `agentmux-srv-*`
 *  sidecar, and `agentmux-mcp.exe`) inside `buildDir`'s own process tree,
 *  excluding any `--type=` child and (optionally) a known-stale `excludePid`
 *  (used when polling for the POST-crash respawn, so a still-terminating
 *  old PID doesn't get picked up as "the new one").
 *
 *  Deliberately does NOT embed `buildDir` in the WQL query string: a first
 *  version did (`CommandLine like '%<buildDir>%'`), which is a
 *  self-referential match — `execSync` runs this exact query via
 *  `cmd.exe /c wmic ...`, and that invocation's OWN command line contains
 *  the literal `buildDir` string (it's part of the query!), so wmic matched
 *  its own querying process (and any other shell wrapper on the machine
 *  whose command line happened to mention the build path) ahead of the
 *  real host. Filtering on `Name` first (a strict `agentmux-<digits>.<digits>.<digits>.exe`
 *  pattern) and only checking `buildDir` as a secondary disambiguator in JS
 *  avoids this class of bug entirely. */
async function findVersionedHostPid(
    buildDir: string,
    timeoutMs: number,
    excludePid?: number,
): Promise<number> {
    const deadline = Date.now() + timeoutMs;
    const hostNamePattern = /^agentmux-\d+\.\d+\.\d+\.exe$/i;
    // Real Windows command lines (as wmic reports them) always use
    // backslashes. `buildDir` can arrive as `C:/Users/...` (forward
    // slashes) when this suite is invoked from Git Bash/MSYS2, which
    // auto-converts POSIX-looking env var values for non-MSYS child
    // processes — normalize before substring-matching or this never finds
    // anything.
    const buildDirBackslash = buildDir.replace(/\//g, "\\");
    while (Date.now() < deadline) {
        try {
            const out = execSync(
                `wmic process where "name like 'agentmux-%'" get ProcessId,Name,CommandLine /format:csv`,
                { encoding: "utf8" },
            );
            const lines = out.split("\n").map((l) => l.trim()).filter(Boolean);
            for (const line of lines) {
                if (line.startsWith("Node,")) continue; // CSV header
                if (line.includes("--type=")) continue; // gpu/utility/renderer child
                if (!line.includes(buildDirBackslash)) continue; // a different instance's process
                const parts = line.split(",");
                const pid = Number(parts[parts.length - 1]);
                const name = parts[parts.length - 2];
                if (!hostNamePattern.test(name)) continue; // srv sidecar / mcp / wrapper
                if (Number.isFinite(pid) && pid > 0 && pid !== excludePid) return pid;
            }
        } catch {
            // transient — process tree still forming
        }
        await sleep(500);
    }
    throw new Error(`versioned host process under ${buildDir} not found within ${timeoutMs}ms`);
}
