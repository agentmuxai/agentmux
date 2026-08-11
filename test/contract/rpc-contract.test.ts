// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * RPC contract guard (architecture-audit action A1).
 *
 * The frontend↔backend RPC wire contract is hand-maintained. The FE
 * declares command bindings in `frontend/app/store/rpc-api.ts` (each
 * method wraps exactly one `client.rpcCall("name", …)`), and the
 * backend registers handlers in `agentmux-srv` via
 * `engine.register_handler(NAME, …)` where NAME is either a string
 * literal or a `pub const … : &str = "name"`. The Go-era generator that
 * used to keep the two sides in sync (`cmd/generate/main-generatets.go`)
 * was removed with the Go backend and never replaced — so nothing but
 * reviewer diligence prevents the contract from drifting. See
 * `docs/analysis/ANALYSIS_CODEBASE_ARCHITECTURE_AUDIT_2026_06_18.md` (§2, A1).
 *
 * This test re-derives both sides from source at run time and freezes
 * the three drift sets as committed baselines that may only SHRINK:
 *
 *   • liveUnregistered — commands the FE actually CALLS (`RpcApi.X(…)`
 *     or a direct `rpcCall("name")`) that the backend never registers.
 *     These are latent "not-found" calls (the engine logs-once and the
 *     FE ignores them — see `rpc-client.ts` `notFoundLogMap`). Mostly
 *     Wave-inherited telemetry / conn / wsl surface never reimplemented
 *     in Rust. Fix by implementing the handler or deleting the dead FE
 *     call — never add a new entry here.
 *   • declaredUnregistered — `rpc-api.ts` methods with no backend
 *     handler (dead Wave-inherited binding surface; see A12). Shrinks as
 *     dead methods are deleted.
 *   • registeredUndeclared — backend handlers with no FE binding:
 *     server-driven `agent.*` verbs, `pane.open` (called via a direct
 *     `rpcCall`, not an `RpcApi` method), and the RPC engine's test
 *     stubs (`echo`/`failme`/`slow`/`checkctx`).
 *
 * A NEW drift — a fresh `rpc-api.ts` method without a handler, a new
 * live call to an unhandled command, or a removed handler still bound
 * by the FE — changes one of these sets and fails this test. That is
 * the guardrail the deleted generator used to provide, at a fraction of
 * the cost. When a change legitimately shrinks a set (a cleanup or a
 * newly-wired handler), update the corresponding baseline below.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// ── Locate the repo root (the dir holding both trees) ─────────────────
function repoRoot(): string {
    let dir = path.dirname(fileURLToPath(import.meta.url));
    for (let i = 0; i < 8; i++) {
        if (
            fs.existsSync(path.join(dir, "agentmux-srv")) &&
            fs.existsSync(path.join(dir, "frontend"))
        ) {
            return dir;
        }
        dir = path.dirname(dir);
    }
    throw new Error("rpc-contract: could not locate repo root from " + import.meta.url);
}

function walk(dir: string, exts: string[], acc: string[] = []): string[] {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
        if (ent.name === "node_modules" || ent.name === "target" || ent.name === ".git") {
            continue;
        }
        const p = path.join(dir, ent.name);
        if (ent.isDirectory()) walk(p, exts, acc);
        else if (exts.some((e) => ent.name.endsWith(e))) acc.push(p);
    }
    return acc;
}

const sortedDiff = (a: Set<string>, b: Set<string>): string[] =>
    [...a].filter((x) => !b.has(x)).sort();

interface Contract {
    registered: Set<string>;
    declared: Set<string>;
    liveCommands: Set<string>;
}

function deriveContract(root: string): Contract {
    // ── Backend: resolve `register_handler(NAME, …)` to command names.
    // NAME is a string literal or a `pub const … &str = "…"`. Both forms
    // appear; consts are resolved against every const defined in the
    // crate (not just COMMAND_*-prefixed ones).
    const rsFiles = walk(path.join(root, "agentmux-srv", "src"), [".rs"]);
    const constMap = new Map<string, string>();
    const constRe = /pub const (\w+)\s*:\s*&str\s*=\s*"([^"]+)"\s*;/g;
    for (const f of rsFiles) {
        const src = fs.readFileSync(f, "utf8");
        let m: RegExpExecArray | null;
        while ((m = constRe.exec(src))) constMap.set(m[1], m[2]);
    }

    const registered = new Set<string>();
    const unresolved: string[] = [];
    const regRe = /register_handler\s*\(\s*(?:"([^"]+)"|([A-Za-z_][A-Za-z0-9_:]*))/g;
    for (const f of rsFiles) {
        const src = fs.readFileSync(f, "utf8");
        let m: RegExpExecArray | null;
        while ((m = regRe.exec(src))) {
            if (m[1] !== undefined) {
                registered.add(m[1]);
                continue;
            }
            const ident = m[2].split("::").pop() as string;
            const val = constMap.get(ident);
            if (val !== undefined) registered.add(val);
            else unresolved.push(ident);
        }
    }
    // A register_handler arg we can't resolve means the extractor is
    // blind to part of the contract — fail loudly rather than pass with
    // an incomplete `registered` set.
    expect(unresolved, `unresolved register_handler args: ${unresolved.join(", ")}`).toEqual([]);

    // ── Frontend: declared bindings (method → command) in rpc-api.ts.
    // Each binding is `Method(client: RpcClient, …): Ret { return
    // client.rpcCall|rpcStream("name", …); }`. Bound the search to each
    // method's own body — `[its signature, the next signature)` — and
    // take the first rpcCall/rpcStream inside it. A single lazy
    // `…*?rpcCall` regex would run past a `rpcStream`-only method into
    // the next one, mismapping commands and dropping every stream
    // binding from the contract (a guard that passes while drift slips
    // through). The per-method window also tolerates braces in return
    // types. Both rpcCall and rpcStream count as a binding.
    // rpc-api was split from a single rpc-api.ts into a domain-module
    // directory (rpc-api/{agent,block,file,...}.ts) composed by index.ts.
    // Concatenate every domain file so the binding extractor still sees the
    // full method surface.
    const rpcApiDir = path.join(root, "frontend", "app", "store", "rpc-api");
    const rpcApiSrc = fs
        .readdirSync(rpcApiDir)
        .filter((f) => f.endsWith(".ts"))
        .map((f) => fs.readFileSync(path.join(rpcApiDir, f), "utf8"))
        .join("\n");
    const sigRe = /(\w+)\s*\(\s*client:\s*RpcClient/g;
    const sigs: Array<{ name: string; idx: number }> = [];
    let sm: RegExpExecArray | null;
    while ((sm = sigRe.exec(rpcApiSrc))) sigs.push({ name: sm[1], idx: sm.index });
    const bindRe = /rpc(?:Call|Stream)\(\s*"([^"]+)"/;
    const methodToCmd = new Map<string, string>();
    const skippedMethods: string[] = [];
    for (let i = 0; i < sigs.length; i++) {
        const end = i + 1 < sigs.length ? sigs[i + 1].idx : rpcApiSrc.length;
        const m = bindRe.exec(rpcApiSrc.slice(sigs[i].idx, end));
        if (m) methodToCmd.set(sigs[i].name, m[1]);
        else skippedMethods.push(sigs[i].name);
    }
    // Every `(client: RpcClient …)` binding must resolve to a command —
    // a non-empty list means the extractor went blind to part of the
    // surface (exactly the regression Codex flagged on the first draft).
    expect(skippedMethods, `unresolved rpc-api bindings: ${skippedMethods.join(", ")}`).toEqual([]);
    const declared = new Set<string>(methodToCmd.values());

    // ── Frontend: live usage — `RpcApi.Method(…)` resolved via the map,
    // plus any direct `rpcCall("name")` outside rpc-api.ts.
    const feFiles = walk(path.join(root, "frontend"), [".ts", ".tsx"]).filter(
        (f) =>
            !f.includes(`${path.sep}store${path.sep}rpc-api${path.sep}`) &&
            !/\.(test|spec)\.[tj]sx?$/.test(f) &&
            !f.includes(`${path.sep}__tests__${path.sep}`),
    );
    const liveCommands = new Set<string>();
    for (const f of feFiles) {
        const src = fs.readFileSync(f, "utf8");
        let m: RegExpExecArray | null;
        const useRe = /RpcApi\.(\w+)\s*\(/g;
        while ((m = useRe.exec(src))) {
            const cmd = methodToCmd.get(m[1]);
            if (cmd !== undefined) liveCommands.add(cmd);
        }
        const directRe = /rpc(?:Call|Stream)\(\s*"([^"]+)"/g;
        while ((m = directRe.exec(src))) liveCommands.add(m[1]);
    }

    return { registered, declared, liveCommands };
}

// ── Committed baselines (sorted). These may only shrink. ──────────────

/** FE makes live calls to these, but the backend registers no handler. */
const KNOWN_LIVE_UNREGISTERED = [
    "activity",
    "connconnect",
    "conndisconnect",
    "connensure",
    "connlist",
    "connlistaws",
    "fileappend",
    "filejoin",
    "recordtevent",
    "resolveids",
    "setrtinfo",
    "workspacelist",
    "wsllist",
];

/** rpc-api.ts declares these methods, but no backend handler exists. */
const KNOWN_DECLARED_UNREGISTERED = [
    "activity",
    "aisendmessage",
    "authenticate",
    "authenticatetoken",
    "blockinfo",
    "blockslist",
    "captureblockscreenshot",
    "connconnect",
    "conndisconnect",
    "connensure",
    "connlist",
    "connlistaws",
    "connstatus",
    "controllerappendoutput",
    "controllerstop",
    "createblock",
    "deleteblock",
    "dispose",
    "disposesuggestions",
    "eventpublish",
    "eventrecv",
    "fetchsuggestions",
    "fileappend",
    "fileappendijson",
    "filecopy",
    "filecreate",
    "filedelete",
    "fileinfo",
    "filejoin",
    "filelist",
    "fileliststream",
    "filemkdir",
    "filemove",
    "fileread",
    "filereadstream",
    "filesharecapability",
    "filestreamtar",
    "filewrite",
    "focuswindow",
    "getrtinfo",
    "gettab",
    "getupdatechannel",
    "getvar",
    "message",
    "notify",
    "path",
    "recordtevent",
    "remotefilecopy",
    "remotefiledelete",
    "remotefileinfo",
    "remotefilejoin",
    "remotefilemove",
    "remotefiletouch",
    "remotegetinfo",
    "remoteinstallrcfiles",
    "remotelistentries",
    "remotemkdir",
    "remotestreamcpudata",
    "remotestreamfile",
    "remotetarstream",
    "remotewritefile",
    "resolveids",
    "sendtelemetry",
    "setconnectionsconfig",
    "setrtinfo",
    "setvar",
    "setview",
    "streamcpudata",
    "streamtest",
    "termgetscrollbacklines",
    "test",
    "waitforroute",
    "webselector",
    "workspacelist",
    "wshactivity",
    "wsldefaultdistro",
    "wsllist",
    "wslstatus",
];

/**
 * Backend registers these handlers, but rpc-api.ts has no method.
 */
const KNOWN_REGISTERED_UNDECLARED = [
    "agent.define",
    "agent.list",
    "agent.open",
    "agent.output",
    "agent.send",
    "agent.status",
    "agent.stop",
    "bundle.delete",
    "bundle.export",
    "bundle.export_for_agent",
    "bundle.get",
    "bundle.import",
    "bundle.import_for_agent",
    "bundle.list",
    "bundle.self.get",
    "bundle.upsert",
    "bundle.validate",
    "checkctx",
    "echo",
    "failme",
    "getwaveairatelimit",
    "identity.account.upsert",
    "identity.account.validate",
    "identity.self.accounts",
    "identity.self.unlink",
    "memory.list",
    "memory.read",
    "memory.write",
    "pane.open",
    "slow",
];

describe("RPC frontend↔backend contract (A1)", () => {
    const { registered, declared, liveCommands } = deriveContract(repoRoot());

    it("extracts a plausible contract surface (extractor sanity)", () => {
        // Guards against a silently-broken regex passing the diff
        // assertions vacuously (empty − empty = empty).
        expect(registered.size).toBeGreaterThan(120);
        expect(declared.size).toBeGreaterThan(180);
        expect(liveCommands.size).toBeGreaterThan(80);
        // A known happy-path command must be visible on every axis.
        expect(registered.has("setmeta")).toBe(true);
        expect(declared.has("setmeta")).toBe(true);
        expect(liveCommands.has("setmeta")).toBe(true);
        // A known `rpcStream` binding must be in the declared surface —
        // guards the per-method extraction against dropping streams.
        expect(declared.has("fileliststream")).toBe(true);
    });

    it("no NEW live FE call resolves to a missing backend handler", () => {
        // The most actionable set: commands the FE invokes at run time
        // with no handler. Shrink by wiring the handler or deleting the
        // dead call; never grow.
        expect(sortedDiff(liveCommands, registered)).toEqual(KNOWN_LIVE_UNREGISTERED);
    });

    it("no NEW rpc-api.ts binding lacks a backend handler", () => {
        expect(sortedDiff(declared, registered)).toEqual(KNOWN_DECLARED_UNREGISTERED);
    });

    it("no NEW backend handler lacks a frontend binding", () => {
        expect(sortedDiff(registered, declared)).toEqual(KNOWN_REGISTERED_UNDECLARED);
    });
});
