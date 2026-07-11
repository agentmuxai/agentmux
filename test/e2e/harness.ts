// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared harness for the opt-in E2E suites (`AGENTMUX_E2E_BUILD_DIR`-gated,
 * Windows-only). Extracted verbatim from `crash-reproject.e2e.test.ts`
 * (SPEC_PILLAR1_STEP4 Phase 5) when the window-close-baseline suite was
 * added — the CDP wire helpers, the isolated-instance boot/teardown, and
 * the wmic-based process discovery are identical for every suite that
 * drives a real packaged build.
 *
 * Deliberately NOT the `ws` npm package for CDP websockets: its
 * package.json maps a "browser" field to a stub that throws ("ws does not
 * work in the browser"), and the repo's shared vitest.config.ts merges in
 * the frontend's own vite.config (browser-oriented resolution needed for
 * the rest of the test suite) — that resolution applies even under
 * `@vitest-environment node` and even via `createRequire` (vite-node
 * intercepts Node's own `require` too, not just ESM imports). Node's own
 * native `WebSocket` (global since Node 22, undici-backed) sidesteps this
 * entirely — it's a process global, not something resolved through Vite's
 * module graph at all. Its API is DOM-style (`addEventListener`, not
 * `ws`'s EventEmitter `.on()`).
 */

import { spawn, execSync, type ChildProcess } from "node:child_process";
import path from "node:path";
import fs from "node:fs";

// ── CDP helpers ───────────────────────────────────────────────────────────

export interface CdpTarget {
    title: string;
    url: string;
    webSocketDebuggerUrl: string;
}

export async function listCdpTargets(port: number): Promise<CdpTarget[]> {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`);
    if (!res.ok) throw new Error(`CDP /json/list returned ${res.status}`);
    return (await res.json()) as CdpTarget[];
}

export function wsConnect(url: string): Promise<WebSocket> {
    return new Promise((resolve, reject) => {
        const ws = new WebSocket(url);
        ws.addEventListener("open", () => resolve(ws));
        ws.addEventListener("error", (ev) => reject(new Error(String((ev as any).message ?? ev))));
    });
}

let msgId = 1;
export function wsSend(ws: WebSocket, method: string, params?: unknown): Promise<any> {
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
export async function evalInMainWindow(port: number, expression: string): Promise<any> {
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
 *  state with any already-running dev/portable instance on this machine.
 *  `AGENTMUX_DEBUG_CLOSE` is the one deliberate pass-through: it's a
 *  pure diagnostic (close-path trace to %TEMP%\agentmux-close-debug.txt),
 *  not instance state, and being able to set it on an E2E run is exactly
 *  how close-path failures in these suites get diagnosed. */
export function scrubbedEnv(): NodeJS.ProcessEnv {
    const env: NodeJS.ProcessEnv = {};
    for (const [k, v] of Object.entries(process.env)) {
        if (!k.startsWith("AGENTMUX_") || k === "AGENTMUX_DEBUG_CLOSE") env[k] = v;
    }
    return env;
}

export function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Poll `netstat` for the two ports a freshly-created host PID listens on
 *  (IPC + CDP — CDP is whichever of the pair actually answers /json/list). */
export async function findCdpPort(pid: number, timeoutMs: number): Promise<number> {
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

export function killPid(pid: number): void {
    try {
        execSync(`taskkill /PID ${pid} /F`, { stdio: "ignore" });
    } catch {
        // already gone — fine
    }
}

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
export async function findVersionedHostPid(
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

// ── Isolated-instance boot / teardown ─────────────────────────────────────

export interface E2eInstance {
    outerWrapperPid: number | null;
    hostPid: number | null;
    logPath: string;
}

/** Spawn the packaged build's outer `agentmux.exe` wrapper detached with a
 *  scrubbed env, wait for its versioned host process to appear, and return
 *  both PIDs. `logName` disambiguates each suite's launcher output file. */
export async function bootInstance(buildDir: string, logName: string): Promise<E2eInstance> {
    const exePath = path.join(buildDir, "agentmux.exe");
    if (!fs.existsSync(exePath)) {
        throw new Error(`AGENTMUX_E2E_BUILD_DIR does not contain agentmux.exe: ${exePath}`);
    }
    const logPath = path.join(buildDir, "..", logName);
    const logFd = fs.openSync(logPath, "w");
    const child: ChildProcess = spawn(exePath, [], {
        cwd: buildDir,
        env: scrubbedEnv(),
        detached: true,
        stdio: ["ignore", logFd, logFd],
    });
    child.unref();

    // The spawned `agentmux.exe` is the outer launcher wrapper; the exe
    // name inside the build dir's `runtime/` (the actual CEF host) is
    // versioned (`agentmux-X.Y.Z.exe`) — discovered by process name once
    // it starts.
    await sleep(3000); // let the wrapper spawn its children
    const hostPid = await findVersionedHostPid(buildDir, 20000);
    return { outerWrapperPid: child.pid ?? null, hostPid, logPath };
}

/** Tear down an instance booted by `bootInstance`.
 *
 *  Kill the wrapper FIRST, not the host: killing `hostPid` while the
 *  wrapper is still alive makes the launcher's own crash-restart logic
 *  treat that as a real crash and spawn yet another replacement — a race
 *  that can leave an orphaned instance behind if the two kills interleave
 *  with the launcher's own respawn. Killing the wrapper first tears down
 *  its Job Object, which should cascade-kill every child (including any
 *  host it's mid-respawn on) in one shot.
 *
 *  Then a defensive final sweep guarantees no leaked process from this
 *  specific build dir survives, regardless of any race above — never by
 *  image name (that would hit every other AgentMux instance on the
 *  machine, including a real one the user has open; see CLAUDE.md's
 *  "CRITICAL: Never Kill AgentMux by Image Name"). Scoped to this exact
 *  build dir's own process tree only. */
export async function teardownInstance(buildDir: string, inst: E2eInstance): Promise<void> {
    if (inst.outerWrapperPid) killPid(inst.outerWrapperPid);
    if (inst.hostPid) killPid(inst.hostPid);
    await sleep(1000);
    try {
        const out = execSync(
            `wmic process where "(name like 'agentmux-%' or name='agentmux.exe')" get ProcessId,CommandLine /format:csv`,
            { encoding: "utf8" },
        );
        const buildDirBackslash = buildDir.replace(/\//g, "\\");
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
}
