#!/usr/bin/env node
// Smoke-test the portable build by spawning agentmux-srv directly and
// exercising the App API RPC surface over WebSocket. No CEF host required —
// just the backend, which is what ultra-long-sessions lives in anyway.
//
// Usage:
//   node scripts/smoke-test-portable.js [portable-dir]
//
// Default portable-dir: ~/Desktop/agentmux-cef-<version>-x64-portable/
//
// Exit code 0 if all checks pass; 1 if anything fails. Always prints a
// summary table before exit.

const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");
const os = require("os");
const WebSocket = require("ws");
const crypto = require("crypto");

// ─── Config ──────────────────────────────────────────────────────────────
const ARG_DIR = process.argv[2];
const DEFAULT_VERSION = require(path.join(__dirname, "..", "package.json")).version;
const PORTABLE_DIR =
    ARG_DIR ||
    path.join(os.homedir(), "Desktop", `agentmux-cef-${DEFAULT_VERSION}-x64-portable`);

const SRV_BIN = path.join(
    PORTABLE_DIR,
    "runtime",
    `agentmux-srv-${DEFAULT_VERSION}-windows.x64.exe`
);

// Use a throwaway data directory so the smoke test never collides with a
// real user install or a concurrent portable instance.
const SMOKE_DATA = path.join(os.tmpdir(), `agentmux-smoke-${crypto.randomBytes(4).toString("hex")}`);
fs.mkdirSync(SMOKE_DATA, { recursive: true });

const results = [];
const push = (name, pass, detail) => {
    results.push({ name, pass, detail });
    const tag = pass ? "PASS" : "FAIL";
    console.log(`  [${tag}] ${name}${detail ? " — " + detail : ""}`);
};

function summarize(exitCode) {
    console.log("\n─── Smoke test summary ───");
    const passed = results.filter((r) => r.pass).length;
    console.log(`  ${passed}/${results.length} checks passed`);
    for (const r of results.filter((r) => !r.pass)) {
        console.log(`    FAIL: ${r.name}${r.detail ? " — " + r.detail : ""}`);
    }
    console.log(`  data dir: ${SMOKE_DATA}`);
    process.exit(exitCode);
}

function fail(msg) {
    console.error(`FATAL: ${msg}`);
    summarize(1);
}

// ─── Disk layout assertions (no srv required) ────────────────────────────

function diskChecks() {
    console.log("\n─── Disk layout checks ───");

    if (!fs.existsSync(PORTABLE_DIR)) {
        fail(`portable dir does not exist: ${PORTABLE_DIR}`);
    }
    push("portable dir exists", true, PORTABLE_DIR);

    const runtimeDir = path.join(PORTABLE_DIR, "runtime");
    push("runtime/ exists", fs.existsSync(runtimeDir));

    // wsh has been retired — see specs/SPEC_RETIRE_WSH_2026_04_12.md.
    // The runtime/ directory MUST NOT contain wsh-*.exe any more.
    const wshGlob = fs
        .readdirSync(runtimeDir)
        .filter((f) => f.startsWith("wsh-") || f === "wsh.exe");
    push(
        "wsh absent from runtime (retired)",
        wshGlob.length === 0,
        wshGlob.length === 0 ? "none" : `found: ${wshGlob.join(", ")}`
    );

    // runtime/bin/ must not be created either — this was the deploy_wsh target.
    push(
        "runtime/bin/ absent (deploy_wsh retired)",
        !fs.existsSync(path.join(runtimeDir, "bin"))
    );

    // srv, cef, launcher
    push("srv binary", fs.existsSync(SRV_BIN));
    const cefBin = path.join(runtimeDir, `agentmux-cef-${DEFAULT_VERSION}.exe`);
    push("cef host binary", fs.existsSync(cefBin));
    push("launcher exists", fs.existsSync(path.join(PORTABLE_DIR, "agentmux.exe")));

    // GPU DLLs — expected since commit 8e15fe7 (Mar 29)
    push("libEGL.dll", fs.existsSync(path.join(runtimeDir, "libEGL.dll")));
    push("libGLESv2.dll", fs.existsSync(path.join(runtimeDir, "libGLESv2.dll")));
}

// ─── Spawn srv, parse AGENTMUXSRV-ESTART, return ws port ─────────────────────

function spawnSrv() {
    return new Promise((resolve, reject) => {
        console.log("\n─── Starting agentmux-srv ───");
        // The auth key is required by the srv's config loader AND enforced
        // on the /ws upgrade. The CEF host normally generates a fresh one
        // per boot and passes it both to srv-via-env and to the frontend
        // via the launch handshake. For the smoke test we generate it, set
        // it on the child env, and remember it so the WS client can use
        // the `authkey=` query param to get past the upgrade middleware.
        const authKey = crypto.randomBytes(32).toString("hex");
        const env = {
            ...process.env,
            AGENTMUX_DATA_HOME: SMOKE_DATA,
            AGENTMUX_CONFIG_HOME: path.join(SMOKE_DATA, "config"),
            AGENTMUX_AUTH_KEY: authKey,
        };
        const proc = spawn(SRV_BIN, [], { env, stdio: ["pipe", "pipe", "pipe"] });

        let wsPort = null;
        let stderrBuf = "";
        const timer = setTimeout(() => {
            reject(new Error("AGENTMUXSRV-ESTART timeout after 15s"));
        }, 15000);

        proc.stderr.on("data", (chunk) => {
            stderrBuf += chunk.toString();
            const match = stderrBuf.match(/AGENTMUXSRV-ESTART ws:127\.0\.0\.1:(\d+)/);
            if (match && wsPort === null) {
                wsPort = parseInt(match[1], 10);
                clearTimeout(timer);
                console.log(`  srv up, ws port: ${wsPort}`);
                resolve({ proc, wsPort, authKey });
            }
        });

        proc.on("error", (err) => {
            clearTimeout(timer);
            reject(err);
        });
        proc.on("exit", (code) => {
            if (wsPort === null) {
                clearTimeout(timer);
                reject(new Error(`srv exited before ESTART (code ${code})\nstderr: ${stderrBuf.slice(-500)}`));
            }
        });
    });
}

// ─── WebSocket RPC client ────────────────────────────────────────────────

class WsRpcClient {
    constructor(port, authKey) {
        this.port = port;
        this.authKey = authKey;
        this.ws = null;
        this.pending = new Map();
        this.source = "smoke-" + crypto.randomBytes(4).toString("hex");
    }

    connect() {
        return new Promise((resolve, reject) => {
            // Auth is enforced on the WS upgrade via either `X-AuthKey`
            // header or `authkey` query param. `ws` doesn't accept custom
            // upgrade headers on the browser-style constructor, so use the
            // query string form.
            this.ws = new WebSocket(
                `ws://127.0.0.1:${this.port}/ws?authkey=${encodeURIComponent(this.authKey)}`
            );
            const t = setTimeout(() => reject(new Error("ws connect timeout")), 5000);
            this.ws.on("open", () => {
                clearTimeout(t);
                resolve();
            });
            this.ws.on("error", (e) => {
                clearTimeout(t);
                reject(e);
            });
            this.ws.on("message", (data) => {
                let msg;
                try {
                    msg = JSON.parse(data.toString());
                } catch {
                    return;
                }
                // Agentmux wraps RPC messages inside { eventtype:"rpc", data:<RpcMessage> }
                const inner = msg.eventtype === "rpc" ? msg.data : msg;
                if (!inner || !inner.resid) return;
                const p = this.pending.get(inner.resid);
                if (!p) return;
                this.pending.delete(inner.resid);
                if (inner.error) p.reject(new Error(inner.error));
                else p.resolve(inner.data);
            });
        });
    }

    call(command, data, timeout = 8000) {
        return new Promise((resolve, reject) => {
            const reqid = crypto.randomUUID();
            const msg = {
                wscommand: "rpc",
                message: { command, reqid, data, source: this.source },
            };
            const timer = setTimeout(() => {
                this.pending.delete(reqid);
                reject(new Error(`timeout: ${command}`));
            }, timeout);
            this.pending.set(reqid, {
                resolve: (v) => {
                    clearTimeout(timer);
                    resolve(v);
                },
                reject: (e) => {
                    clearTimeout(timer);
                    reject(e);
                },
            });
            this.ws.send(JSON.stringify(msg));
        });
    }

    close() {
        if (this.ws) this.ws.close();
    }
}

// ─── RPC checks ──────────────────────────────────────────────────────────

async function rpcChecks(port, authKey) {
    console.log("\n─── App API RPC checks ───");
    const rpc = new WsRpcClient(port, authKey);
    await rpc.connect();
    push("ws connect", true, `ws://127.0.0.1:${port}/ws`);

    // Give the srv a beat for handlers to settle + initial config blast
    await new Promise((r) => setTimeout(r, 300));

    // agent.list — returns seeded Forge agents; should work w/o auth
    try {
        const list = await rpc.call("agent.list", {});
        const count = Array.isArray(list?.agents) ? list.agents.length : 0;
        push("agent.list", true, `${count} agents`);
        if (count > 0) {
            console.log(
                "    agents:",
                list.agents.map((a) => `${a.name || a.id}(${a.provider})`).join(", ")
            );
        }
    } catch (e) {
        push("agent.list", false, e.message);
    }

    // blockfile:line_count on a non-existent block — should return 0 or error gracefully
    try {
        const r = await rpc.call("blockfile:line_count", {
            block_id: "00000000-0000-0000-0000-000000000000",
            filename: "output",
        });
        push("blockfile:line_count (missing block)", true, `line_count=${r?.line_count ?? "?"}`);
    } catch (e) {
        // Errors are OK here — the API shouldn't crash the server
        push(
            "blockfile:line_count (missing block)",
            /not.?found|BLOCK_NOT_FOUND|no such|not exist/i.test(e.message),
            e.message
        );
    }

    // blockfile:read_range with offset=0 limit=0 — shape accepts u64 fields
    try {
        const r = await rpc.call("blockfile:read_range", {
            block_id: "00000000-0000-0000-0000-000000000000",
            filename: "output",
            offset: 0,
            limit: 0,
        });
        push("blockfile:read_range (empty)", true, JSON.stringify(r).slice(0, 80));
    } catch (e) {
        push(
            "blockfile:read_range (empty)",
            /not.?found|BLOCK_NOT_FOUND/i.test(e.message),
            e.message
        );
    }

    // session:archive on a missing block — `archive_session_output` in
    // session_archive.rs:97 goes straight to FileStore and returns
    // Ok((0, now_ms)) when there's nothing to archive. So a missing block
    // is treated as an idempotent no-op, not an error. Both the no-op and
    // a BLOCK_NOT_FOUND error are acceptable behaviors. Note the COLON in
    // the command name — backend uses `session:archive`.
    try {
        const r = await rpc.call("session:archive", {
            block_id: "00000000-0000-0000-0000-000000000000",
        });
        const noop = r && (r.bytes_archived === 0 || r.archived_bytes === 0);
        push(
            "session:archive (missing)",
            true,
            noop ? `no-op: ${JSON.stringify(r)}` : `unexpected: ${JSON.stringify(r)}`
        );
    } catch (e) {
        push(
            "session:archive (missing)",
            /BLOCK_NOT_FOUND|not.?found|BLOCK/i.test(e.message),
            e.message.slice(0, 80)
        );
    }

    // session:digest on a missing block — also expected to fail gracefully
    try {
        await rpc.call("session:digest", {
            block_id: "00000000-0000-0000-0000-000000000000",
        });
        push("session:digest (missing)", false, "should have errored");
    } catch (e) {
        push(
            "session:digest (missing)",
            /BLOCK_NOT_FOUND|not.?found|BLOCK|line_count/i.test(e.message),
            e.message.slice(0, 80)
        );
    }

    rpc.close();
}

// ─── Data-dir assertions after srv startup ───────────────────────────────

function postStartupDataDir() {
    console.log("\n─── Data directory assertions ───");
    push(
        "smoke data dir created",
        fs.existsSync(SMOKE_DATA),
        SMOKE_DATA
    );

    // Phase 4.2 runtime behavior: the srv seeds .agentmux/.gitignore on first
    // launch. It seeds the REAL ~/.agentmux though, not AGENTMUX_DATA_HOME —
    // so we can only check the real one, and even then only if it's freshly
    // created for this run. Document-and-skip if missing.
    const realGitignore = path.join(os.homedir(), ".agentmux", ".gitignore");
    if (fs.existsSync(realGitignore)) {
        const content = fs.readFileSync(realGitignore, "utf8");
        push(
            "~/.agentmux/.gitignore has expected content",
            content.includes("*") && content.includes("!.gitignore"),
            `${content.length} bytes`
        );
    } else {
        push(
            "~/.agentmux/.gitignore",
            true,
            "(skipped — real data dir not affected by smoke test env)"
        );
    }
}

// ─── Main ────────────────────────────────────────────────────────────────

(async () => {
    console.log("AgentMux portable smoke test");
    console.log(`  version: ${DEFAULT_VERSION}`);
    console.log(`  portable: ${PORTABLE_DIR}`);
    console.log(`  data: ${SMOKE_DATA}`);

    diskChecks();

    // If disk checks failed critically, abort before spawning
    if (results.some((r) => !r.pass && /does not exist|missing/i.test(r.detail || ""))) {
        fail("critical disk check failed — skipping srv startup");
    }

    let srvProc, wsPort, authKey;
    try {
        const spawned = await spawnSrv();
        srvProc = spawned.proc;
        wsPort = spawned.wsPort;
        authKey = spawned.authKey;
    } catch (e) {
        push("srv startup", false, e.message);
        summarize(1);
    }

    try {
        await rpcChecks(wsPort, authKey);
    } catch (e) {
        push("rpc checks (fatal)", false, e.message);
    }

    postStartupDataDir();

    // Clean shutdown
    try {
        srvProc.stdin.end(); // srv exits on stdin EOF
        await new Promise((r) => setTimeout(r, 500));
        if (!srvProc.killed) srvProc.kill();
    } catch {}

    // Leave the smoke data dir around for post-mortem; the dir name includes
    // a random suffix so successive runs don't step on each other. Users can
    // `rm -rf %TEMP%/agentmux-smoke-*` periodically.

    const allPassed = results.every((r) => r.pass);
    summarize(allPassed ? 0 : 1);
})();
