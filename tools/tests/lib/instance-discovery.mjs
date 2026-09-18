// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Resolve WHICH running AgentMux instance a tool should talk to.
//
// Why this exists
// ---------------
// Tools used to hardcode the CDP port: "9223 dev / 9222 release". Those are
// only PREFERRED values. `agentmux-cef/src/lib.rs` binds the preferred port if
// it is free and otherwise takes an OS-assigned one, because several instances
// run in parallel by design (isolation I1-I6). So on any machine with more than
// one instance up, the constant identifies whichever instance won the race —
// not the one you meant.
//
// That failure is silent and it is dangerous. An agent debugging its own test
// build attached to port 9222, got the repo owner's live instance instead, and
// dispatched clicks and keystrokes into it. Nothing errored. The fix is not a
// better default port; it is to stop using a constant as an identity.
//
// The rule this module enforces: discover instances from their own
// `authkey.dev`, verify the owning process is alive, and REFUSE to guess when
// the choice is ambiguous. An explicit error beats acting on the wrong target.

import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

function isAlive(pid) {
    try { process.kill(pid, 0); return true; } catch { return false; }
}

function walkForAuthfiles(root, depth, out) {
    if (depth < 0 || !existsSync(root)) return out;
    let entries;
    try { entries = readdirSync(root, { withFileTypes: true }); } catch { return out; }
    for (const e of entries) {
        const p = join(root, e.name);
        if (e.isFile() && e.name === "authkey.dev") out.push(p);
        else if (e.isDirectory()) walkForAuthfiles(p, depth - 1, out);
    }
    return out;
}

/**
 * Every live instance that published an authkey.dev, newest first.
 *
 * `root` and `alive` are injectable so this is testable without a running
 * AgentMux — the tests build a fake tree and a fake liveness predicate.
 */
export function listLiveInstances({ root = null, alive = isAlive } = {}) {
    const base = root || join(homedir(), ".agentmux");
    const files = [
        ...walkForAuthfiles(join(base, "channels"), 4, []),
        ...walkForAuthfiles(join(base, "dev"), 4, []),
    ];
    const live = [];
    for (const f of files) {
        let j;
        try { j = JSON.parse(readFileSync(f, "utf8")); } catch { continue; }
        if (!j.host_pid || !alive(j.host_pid)) continue;
        live.push({
            authFile: f,
            authKey: j.auth_key,
            webEndpoint: j.web_endpoint,
            wsEndpoint: j.ws_endpoint,
            // Absent on instances predating the field — callers must handle it
            // rather than silently falling back to 9222, which is the bug.
            debugPort: j.debug_port ?? null,
            dataDir: j.data_dir || "",
            instance: j.instance || "",
            hostPid: j.host_pid,
            mtime: (() => { try { return statSync(f).mtimeMs; } catch { return 0; } })(),
        });
    }
    return live.sort((a, b) => b.mtime - a.mtime);
}

/**
 * Pick exactly one instance.
 *
 * `match` is a substring tested against the data dir / instance label — e.g. a
 * channel name. With several instances up and no `match`, this throws rather
 * than picking one: guessing here is what caused the incident above.
 */
export function resolveInstance({ match = null, root = null, alive = isAlive } = {}) {
    const live = listLiveInstances({ root, alive });
    if (live.length === 0) throw new Error("no live AgentMux instance found (no authkey.dev with a running host_pid)");
    const pool = match
        ? live.filter((i) => i.dataDir.includes(match) || i.instance.includes(match) || i.authFile.includes(match))
        : live;
    if (pool.length === 0) {
        throw new Error(`no live instance matching ${JSON.stringify(match)}.\nLive:\n` + describe(live));
    }
    if (pool.length > 1) {
        throw new Error(
            `${pool.length} live instances match${match ? ` ${JSON.stringify(match)}` : ""} — refusing to guess.\n` +
            `Pass --instance <substring of channel or data dir>.\nCandidates:\n` + describe(pool)
        );
    }
    return pool[0];
}

export function describe(list) {
    return list.map((i) => `  - pid ${i.hostPid} ${i.instance} cdp=${i.debugPort ?? "unknown"} ${i.dataDir}`).join("\n");
}

/**
 * The CDP port for a chosen instance, verified end-to-end.
 *
 * Verification matters: the port number alone does not prove whose it is. We
 * confirm the debugger's page targets are served by that instance's own web
 * endpoint before handing the port back.
 */
export async function resolveCdpPort(inst) {
    if (inst.debugPort == null) {
        throw new Error(
            `instance ${inst.instance} (pid ${inst.hostPid}) published no debug_port.\n` +
            `It predates the field. Read it from its log instead:\n` +
            `  grep 'remote-debugging port' <log dir>/cef-debug.log\n` +
            `Do NOT fall back to 9222/9223 — that may be a different instance.`
        );
    }
    const res = await fetch(`http://127.0.0.1:${inst.debugPort}/json`).catch((e) => {
        throw new Error(`CDP ${inst.debugPort} unreachable for pid ${inst.hostPid}: ${e.message}`);
    });
    const targets = await res.json();
    const host = inst.webEndpoint?.split(":").pop();
    const pages = targets.filter((t) => t.type === "page");
    if (host && pages.length && !pages.some((t) => (t.url || "").includes(`:${host}`))) {
        throw new Error(
            `CDP ${inst.debugPort} does not serve ${inst.instance}'s frontend (expected :${host}).\n` +
            `Refusing to drive it — this is the cross-instance mistargeting this module exists to prevent.\n` +
            `Saw: ${pages.slice(0, 3).map((t) => t.url).join(", ")}`
        );
    }
    return inst.debugPort;
}
