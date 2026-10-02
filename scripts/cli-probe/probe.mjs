// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Run the real Claude CLI, in the mode AgentMux runs it in, against the fake API
 * (fake-anthropic.mjs), safely.
 *
 * Safe by construction:
 * - NO account. `ANTHROPIC_BASE_URL` is the local fake and the API key is a dummy.
 * - NO keychain. The CLI reads the macOS keychain by shelling out to the
 *   `security` tool, looked up on PATH. A shim first on PATH logs what it was
 *   asked for and answers "not found" (exit 44), so no real item is touched and
 *   no consent dialog can appear. The log shows exactly what the CLI tried to read.
 * - NO user state. HOME and CLAUDE_CONFIG_DIR are throwaway directories, and the
 *   environment is built from scratch rather than inherited (an inherited
 *   ANTHROPIC_* or AGENTMUX_* variable would silently change what is measured).
 *
 * POSIX only (the shim is a shell script).
 */

import { spawn } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startFakeAnthropic } from "./fake-anthropic.mjs";

/** `security` exits 44 when the item is not found. */
export const SECURITY_ITEM_NOT_FOUND = 44;

/**
 * Write a fake `security` into `dir` and return the log file it appends to.
 * Every call is logged (one line of arguments) and answered "item not found".
 */
export function makeSecurityShim(dir) {
    mkdirSync(dir, { recursive: true });
    const log = join(dir, "security-calls.log");
    writeFileSync(log, "");
    const shim = join(dir, "security");
    writeFileSync(shim, `#!/bin/sh\nprintf '%s\\n' "$*" >> '${log.replace(/'/g, "'\\''")}'\nexit ${SECURITY_ITEM_NOT_FOUND}\n`);
    chmodSync(shim, 0o755);
    return { dir, log };
}

/** A clean environment for the CLI: nothing inherited except what is passed. */
export function probeEnv({ baseUrl, home, configDir, shimDir, path = process.env.PATH ?? "" }) {
    return {
        PATH: `${shimDir}:${path}`,
        HOME: home,
        TMPDIR: home,
        CLAUDE_CONFIG_DIR: configDir,
        ANTHROPIC_BASE_URL: baseUrl,
        ANTHROPIC_API_KEY: "sk-ant-api03-probe-not-a-real-key",
        DISABLE_AUTOUPDATER: "1",
        DISABLE_TELEMETRY: "1",
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
        TERM: "dumb",
        NO_COLOR: "1",
    };
}

/**
 * @param {object} o
 * @param {string} o.cli                    path to the claude binary
 * @param {string[]} [o.args]               flags after the fixed stream-json ones (e.g. ["--model","sonnet"])
 * @param {string[]} [o.messages]           user messages to send, one turn each
 * @param {object[]} [o.controlRequests]    `request` bodies to send as control_requests once the CLI is up
 * @param {number} [o.timeoutMs]
 * @param {number} [o.settleMs]             with no messages, how long to let the CLI talk on its own
 * @param {Function} [o.script]             scripted API replies (see fake-anthropic.mjs): e.g. make the model call a tool; called (n, body, {cwd})
 * @param {(req: object) => ("allow"|"deny")} [o.decide]  answer to each can_use_tool request the CLI sends (default allow)
 */
export async function runProbe({ cli, args = [], messages = ["hello"], controlRequests = [], timeoutMs = 60000, settleMs = 3000, script, decide = () => "allow" }) {
    const root = mkdtempSync(join(tmpdir(), "amx-cli-probe-"));
    const home = join(root, "home");
    const configDir = join(root, "config");
    mkdirSync(home, { recursive: true });
    mkdirSync(configDir, { recursive: true });
    const shim = makeSecurityShim(join(root, "shim"));
    let api;
    let child;
    try {
        api = await startFakeAnthropic({ script: script && ((n, body) => script(n, body, { cwd: home })) });

        const argv = ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", ...args];
        child = spawn(cli, argv, {
            cwd: home,
            env: probeEnv({ baseUrl: api.url, home, configDir, shimDir: shim.dir }),
            stdio: ["pipe", "pipe", "pipe"],
        });

        const events = [];
        const permissionRequests = []; // every can_use_tool the CLI asked, in order
        const timeline = []; // [label, ms since start] for ordering questions
        const t0 = Date.now();
        let stderr = "";
        let buf = "";
        let results = 0;
        let spawnError = null;
        // A failed spawn (bad path) or a write to a CLI that has already exited must
        // fail THIS probe, not crash the test worker with an unhandled 'error'.
        child.on("error", (e) => (spawnError = e));
        child.stdin.on("error", () => {});
        const send = (obj) => {
            if (child.exitCode === null && !child.stdin.destroyed) child.stdin.write(JSON.stringify(obj) + "\n");
        };
        child.stderr.on("data", (d) => (stderr += d.toString("utf8")));
        child.stdout.on("data", (d) => {
            buf += d.toString("utf8");
            let i;
            while ((i = buf.indexOf("\n")) >= 0) {
                const line = buf.slice(0, i).trim();
                buf = buf.slice(i + 1);
                if (!line) continue;
                try {
                    const ev = JSON.parse(line);
                    events.push(ev);
                    timeline.push([`${ev.type}${ev.subtype ? "/" + ev.subtype : ""}`, Date.now() - t0]);
                    if (ev.type === "control_request" && ev.request?.subtype === "can_use_tool") {
                        permissionRequests.push({ tool: ev.request.tool_name, input: ev.request.input });
                        const verdict = decide(ev.request);
                        send({
                            type: "control_response",
                            response: {
                                subtype: "success",
                                request_id: ev.request_id,
                                response:
                                    verdict === "allow"
                                        ? { behavior: "allow", updatedInput: ev.request.input }
                                        : { behavior: "deny", message: "denied by the probe" },
                            },
                        });
                    }
                    if (ev.type === "result") results++;
                } catch {
                    /* non-JSON stdout line: ignored */
                }
            }
        });
        const exit = new Promise((resolve) => child.once("close", (code, signal) => resolve({ code, signal })));

        // One at a time: each control_request waits for its own answer, so a later
        // request observes the effect of an earlier one (a request that is merely
        // QUEUED behind it would otherwise look like it had no effect).
        for (const request of controlRequests) {
            const id = `probe-${Math.random().toString(36).slice(2, 8)}`;
            send({ type: "control_request", request_id: id, request });
            timeline.push([`sent control_request/${request.subtype}`, Date.now() - t0]);
            const until = Date.now() + 8000;
            while (Date.now() < until && !spawnError && child.exitCode === null && !events.some((e) => e.type === "control_response" && e.response?.request_id === id)) {
                await new Promise((r) => setTimeout(r, 25));
            }
        }
        for (const m of messages) {
            send({ type: "user", message: { role: "user", content: [{ type: "text", text: m }] } });
            timeline.push(["sent user message", Date.now() - t0]);
        }

        const deadline = Date.now() + timeoutMs;
        if (messages.length === 0) {
            // Nothing to wait for a result of: let the CLI say what it says on its own.
            await new Promise((r) => setTimeout(r, settleMs));
        }
        while (results < messages.length && Date.now() < deadline && !spawnError) {
            if (child.exitCode !== null) break;
            await new Promise((r) => setTimeout(r, 50));
        }
        const timedOut = results < messages.length && child.exitCode === null && !spawnError;
        child.stdin.end();
        const done = await Promise.race([exit, new Promise((r) => setTimeout(() => r(null), 5000))]);
        if (!done) child.kill("SIGKILL");
        const status = done ?? (await Promise.race([exit, new Promise((r) => setTimeout(() => r({ code: null, signal: "SIGKILL" }), 2000))]));
        if (spawnError) throw new Error(`could not run ${cli}: ${spawnError.message}`);

        let securityCalls = [];
        try {
            securityCalls = readFileSync(shim.log, "utf8").split("\n").filter(Boolean);
        } catch {
            /* none */
        }
        return { argv, events, timeline, requests: api.requests.slice(), permissionRequests, securityCalls, stderr, timedOut, ...status };
    } finally {
        if (child && child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
        await api?.close();
        rmSync(root, { recursive: true, force: true });
    }
}
