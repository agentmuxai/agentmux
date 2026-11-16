// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Forces the real Node environment (vitest.config.ts's global default is
// jsdom, for frontend component tests) — muxlog.mjs is a standalone Node
// CLI script that never runs in a browser, and jsdom's `fetch` polyfill
// doesn't actually reach a real local HTTP server the way checkLiveness's
// tests below need (probePort speaks real HTTP as of reagent P2 on
// PR #2742 — every "live"/"multiple ipc-port files" case silently failed
// under jsdom's fetch until this was added, passing fine under plain Node).
//
// Unit tests for muxlog.mjs's glob()/filterByInstance()/pickCandidate() —
// pure logic (filesystem reads against a real temp dir for glob(), plain
// data for the other two — no mocking needed, no network, no process.exit).
// Runs as part of `npm test` (vitest), same discipline as muxspect.test.mjs.
//
// Pins three fixes/features from docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md:
// - §2.2/§2.3: the channels/ log-discovery glob had one wildcard segment more
//   than any real on-disk channel-build layout actually has
//   (`channels/*/versions/*/*/logs` vs. the real `channels/*/versions/*/logs`),
//   so it silently matched zero channel-build logs on every platform, the
//   entire time that source existed — logRoots() now tries both depths.
// - §2.1: `resolveFile` picked "freshest across every instance on the
//   machine" with no way to prefer the CALLER's own running instance —
//   `swarm` resolved to a stale same-version sibling's log on the very first
//   live repro. `pickCandidate` now prefers a candidate matching the
//   caller's own $AGENTMUX_CHANNEL when no explicit `-i` is given.
// - Ext 3: `muxlog ls` inferred liveness from log mtime alone (a dead
//   process's log looks identical to a live-but-idle one). checkLiveness()
//   TCP-probes the real `ipc-port-*` file agentmux-cef already writes per
//   instance.

import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
    admissionVerdict,
    checkLiveness,
    expandAgentIds,
    filterByInstance,
    glob,
    groupInstances,
    holderChannel,
    inlineFieldsSuffix,
    instanceTag,
    logNameParts,
    makeAgentMatcher,
    matchesOwnChannel,
    mergeTimelines,
    newestLogFirst,
    parseAgentOpenLine,
    percentile,
    pickCandidate,
    printLastLines,
    renderLine,
    selectWindowNames,
    siblingCandidateDirs,
    summarizeInstance,
    summarizeOpens,
} from "./muxlog.mjs";

let root;

beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "muxlog-glob-test-"));
});

afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
});

function makeDir(...segments) {
    const p = path.join(root, ...segments);
    fs.mkdirSync(p, { recursive: true });
    return p;
}

describe("muxlog glob", () => {
    it("matches a literal-only pattern that exists", () => {
        makeDir("a", "b", "c");
        expect(glob(path.join(root, "a", "b", "c"))).toEqual([path.join(root, "a", "b", "c")]);
    });

    it("returns empty for a literal-only pattern that doesn't exist", () => {
        expect(glob(path.join(root, "nope"))).toEqual([]);
    });

    it("matches the real 1-level channel-build depth: channels/*/versions/*/logs", () => {
        const logs = makeDir("channels", "local-main-abc123", "versions", "0.55.19", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs"));
        expect(found).toEqual([logs]);
    });

    it("does NOT match a 2-level layout with the 1-level pattern (proves the depths are genuinely distinct)", () => {
        makeDir("channels", "local-main-abc123", "versions", "0.55.19", "extra", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs"));
        expect(found).toEqual([]);
    });

    it("matches a 2-level layout with the 2-level pattern (in case some install shape uses it)", () => {
        const logs = makeDir("channels", "local-main-abc123", "versions", "0.55.19", "extra", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "*", "logs"));
        expect(found).toEqual([logs]);
    });

    it("matches multiple channels and versions", () => {
        const a = makeDir("channels", "chan-a", "versions", "0.55.18", "logs");
        const b = makeDir("channels", "chan-a", "versions", "0.55.19", "logs");
        const c = makeDir("channels", "chan-b", "versions", "0.55.19", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs")).sort();
        expect(found).toEqual([a, b, c].sort());
    });

    it("a sibling directory (data/cef-cache/runtime) next to logs is never picked up by the logs glob", () => {
        makeDir("channels", "chan-a", "versions", "0.55.19", "data");
        makeDir("channels", "chan-a", "versions", "0.55.19", "cef-cache");
        const logs = makeDir("channels", "chan-a", "versions", "0.55.19", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs"));
        expect(found).toEqual([logs]);
    });
});

describe("muxlog filterByInstance", () => {
    const cands = [
        { file: "/logs/agentmuxsrv-v0.55.19.log", source: "channel:local-main-abc123", version: "0.55.19" },
        { file: "/logs/agentmuxsrv-v0.55.18.log", source: "shared", version: "0.55.18" },
        { file: "/logs/agentmux-host-v0.55.19.log", source: "dev:agenta-feature", version: "0.55.19" },
    ];

    it("matches on file path substring", () => {
        expect(filterByInstance(cands, "0.55.18")).toEqual([cands[1]]);
    });

    it("matches on source label", () => {
        expect(filterByInstance(cands, "local-main-abc123")).toEqual([cands[0]]);
    });

    it("matches on version", () => {
        expect(filterByInstance(cands, "0.55.19")).toEqual([cands[0], cands[2]]);
    });

    it("is case-insensitive across all three fields (the original inline filter was NOT — only .file was lowercased)", () => {
        expect(filterByInstance(cands, "LOCAL-MAIN-ABC123")).toEqual([cands[0]]);
        expect(filterByInstance(cands, "DEV:AGENTA-FEATURE")).toEqual([cands[2]]);
    });

    it("returns empty when nothing matches", () => {
        expect(filterByInstance(cands, "no-such-instance")).toEqual([]);
    });
});

describe("muxlog pickCandidate", () => {
    const cands = [
        { file: "/logs/agentmuxsrv-v0.55.19.log.fresh", source: "shared", version: "0.55.19" },
        { file: "/logs/channels/local-main-abc123/agentmuxsrv-v0.55.19.log.mine", source: "channel:local-main-abc123", version: "0.55.19" },
    ];

    it("an explicit -i always wins, regardless of $AGENTMUX_CHANNEL", () => {
        const opt = { instance: "local-main-abc123" };
        expect(pickCandidate(cands, opt, "some-other-channel")).toBe(cands[1].file);
    });

    it("no explicit -i: prefers a candidate matching the caller's own $AGENTMUX_CHANNEL over 'freshest first'", () => {
        // cands[0] is first in the list (would win under old "freshest/first" behavior);
        // cands[1] is what should win once we know our own channel.
        const opt = {};
        expect(pickCandidate(cands, opt, "local-main-abc123")).toBe(cands[1].file);
    });

    it("no explicit -i, no own channel found among candidates: falls back to the first (freshest) candidate", () => {
        const opt = {};
        expect(pickCandidate(cands, opt, "a-channel-with-no-log-here")).toBe(cands[0].file);
    });

    it("no explicit -i, $AGENTMUX_CHANNEL unset entirely: falls back to the first (freshest) candidate — old behavior preserved", () => {
        const opt = {};
        expect(pickCandidate(cands, opt, undefined)).toBe(cands[0].file);
    });

    it("explicit -i matching nothing returns null (caller turns this into an error+exit)", () => {
        const opt = { instance: "nonexistent" };
        expect(pickCandidate(cands, opt, "local-main-abc123")).toBeNull();
    });

    // reagent P1 on PR #2741: $AGENTMUX_CHANNEL's dev-mode format is
    // `dev-<branch>[-<clone_id>]` (hyphen), but logRoots() labels dev
    // candidates `"dev:" + branch` (colon) with a slash-separated `.file`
    // path — a plain substring check on the raw channel string never
    // matches either, so the own-channel default silently never fired for
    // task dev, reproducing the exact stale-sibling-log bug this PR exists
    // to fix. Real formats from a live system, not invented ones.
    it("no explicit -i: matches a dev-mode instance despite the separator mismatch (hyphen channel vs. colon source / slash path)", () => {
        const devCands = [
            { file: String.raw`C:\Users\x\.agentmux\dev\main\bd69a405f49440de\logs\agentmux-host-v0.55.19.log`, source: "dev:main", version: "0.55.19" },
            { file: String.raw`C:\Users\x\.agentmux\dev\agentx-quick-fork-phase-3-4\a4649045a423d8c8\logs\agentmux-host-v0.55.19.log`, source: "dev:agentx-quick-fork-phase-3-4", version: "0.55.19" },
        ];
        const opt = {};
        expect(pickCandidate(devCands, opt, "dev-agentx-quick-fork-phase-3-4")).toBe(devCands[1].file);
    });

    it("no explicit -i: matches a dev-mode instance with a clone_id suffix on the channel", () => {
        const devCands = [
            { file: String.raw`C:\Users\x\.agentmux\dev\main\bd69a405f49440de\logs\agentmux-host-v0.55.19.log`, source: "dev:main", version: "0.55.19" },
            { file: String.raw`C:\Users\x\.agentmux\dev\main\abc123clone\logs\agentmux-host-v0.55.19.log`, source: "dev:main", version: "0.55.19" },
        ];
        const opt = {};
        // Both candidates' source is "dev:main" (identical, source alone can't
        // disambiguate) — the clone_id in the channel only appears in the
        // SECOND candidate's file path, which is what should decide it.
        expect(pickCandidate(devCands, opt, "dev-main-abc123clone")).toBe(devCands[1].file);
    });

    it("empty candidate list returns null", () => {
        expect(pickCandidate([], {}, "local-main-abc123")).toBeNull();
    });
});

describe("muxlog matchesOwnChannel", () => {
    it("matches despite hyphen vs. colon separator (dev-mode channel vs. muxlog's 'dev:' source label)", () => {
        expect(matchesOwnChannel({ file: "", source: "dev:agentx-feature", version: "" }, "dev-agentx-feature")).toBe(true);
    });

    it("matches despite hyphen vs. slash separator (dev-mode channel vs. a filesystem path)", () => {
        expect(matchesOwnChannel({ file: String.raw`C:\x\dev\agentx-feature\hash\logs\y.log`, source: "", version: "" }, "dev-agentx-feature")).toBe(true);
    });

    it("matches the portable channel:<name> format unchanged", () => {
        expect(matchesOwnChannel({ file: "", source: "channel:local-main-abc123", version: "" }, "local-main-abc123")).toBe(true);
    });

    it("does not match an unrelated channel", () => {
        expect(matchesOwnChannel({ file: "", source: "dev:agentx-feature", version: "" }, "dev-someone-else")).toBe(false);
    });

    it("an empty ownChannel never matches anything (guards against normalizing '' -> '' and matching every candidate)", () => {
        expect(matchesOwnChannel({ file: "anything", source: "anything", version: "" }, "")).toBe(false);
    });

    // reagent P1 round 2 on PR #2741: the first fix (strip-all-separators,
    // then substring check) false-positive matched a genuinely different
    // sibling channel whose branch name happens to start with the caller's
    // own branch name as a prefix. Real repro shape: "dev-phase-3" is a
    // char-for-char prefix of "dev-phase-3-repro" once separators are
    // flattened away, even though these are two unrelated instances.
    it("does NOT match a sibling channel whose branch name is a prefix-extension of the caller's own (the exact false-positive reagent found)", () => {
        const own = normalizeCandFor("dev:phase-3", String.raw`C:\x\dev\phase-3\hash1\logs\y.log`);
        const sibling = normalizeCandFor("dev:phase-3-repro", String.raw`C:\x\dev\phase-3-repro\hash2\logs\y.log`);
        expect(matchesOwnChannel(sibling, "dev-phase-3")).toBe(false);
        expect(matchesOwnChannel(own, "dev-phase-3")).toBe(true);
    });

    it("does NOT match when the caller's branch is a prefix-extension of a sibling's shorter branch name (the reverse direction)", () => {
        const shortSibling = { file: String.raw`C:\x\dev\phase\hash\logs\y.log`, source: "dev:phase", version: "" };
        expect(matchesOwnChannel(shortSibling, "dev-phase-3")).toBe(false);
    });

    function normalizeCandFor(source, file) {
        return { file, source, version: "" };
    }
});

// Ext 6 (docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md):
// `-d/--dispatch <id>` productizes the manual correlation that report's
// whole investigation had to do by hand. Checked on the RAW line text
// (before/regardless of JSON parsing) because a dispatch id can appear
// either in the rendered message or as a bare structured-field value —
// `--grep` only ever matches the message text, which would silently miss
// a field-only occurrence.
describe("muxlog renderLine --dispatch filter", () => {
    const opt = { dispatch: "dispatch-abc123" };

    it("matches when the id appears in the message text", () => {
        const line = JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "INFO", fields: { message: "processing dispatch-abc123 now" }, target: "agentmux_srv::backend::subagent_watcher" });
        expect(renderLine(line, opt)).not.toBeNull();
    });

    it("matches when the id appears ONLY as a structured field value, not in the message", () => {
        const line = JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "INFO", fields: { message: "backfilling session subagents", dispatch_id: "dispatch-abc123" }, target: "agentmux_srv::backend::subagent_watcher" });
        expect(renderLine(line, opt)).not.toBeNull();
    });

    it("excludes a line that doesn't mention the id anywhere", () => {
        const line = JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "INFO", fields: { message: "unrelated line", dispatch_id: "dispatch-xyz789" }, target: "agentmux_srv::backend::subagent_watcher" });
        expect(renderLine(line, opt)).toBeNull();
    });

    it("still composes with other filters (e.g. --level) — both must pass", () => {
        const debugLine = JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "DEBUG", fields: { message: "dispatch-abc123 debug detail" }, target: "agentmux_srv::backend::subagent_watcher" });
        expect(renderLine(debugLine, { dispatch: "dispatch-abc123", level: ["info"] })).toBeNull();
        expect(renderLine(debugLine, { dispatch: "dispatch-abc123", level: ["debug"] })).not.toBeNull();
    });

    it("with no --dispatch set, every line passes through unaffected (existing behavior)", () => {
        const line = JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "INFO", fields: { message: "anything at all" }, target: "agentmux_srv::backend::subagent_watcher" });
        expect(renderLine(line, {})).not.toBeNull();
    });
});

describe("muxlog printLastLines return value (Ext 6's verdict count)", () => {
    let root;
    let file;

    beforeEach(() => {
        root = fs.mkdtempSync(path.join(os.tmpdir(), "muxlog-printlines-test-"));
        file = path.join(root, "test.log");
    });

    afterEach(() => {
        fs.rmSync(root, { recursive: true, force: true });
    });

    it("returns the total match count, not just what was printed within -n", () => {
        const lines = Array.from({ length: 5 }, (_, i) =>
            JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "INFO", fields: { message: `line ${i} dispatch-abc123` }, target: "x" }),
        );
        fs.writeFileSync(file, lines.join("\n") + "\n");
        const count = printLastLines(file, 2, { dispatch: "dispatch-abc123" }, true);
        expect(count).toBe(5); // full match count, even though -n 2 only printed 2
    });

    it("returns 0 for a file with no matching lines", () => {
        fs.writeFileSync(file, JSON.stringify({ timestamp: "2026-08-22T00:00:00Z", level: "INFO", fields: { message: "no match here" }, target: "x" }) + "\n");
        expect(printLastLines(file, 200, { dispatch: "dispatch-nonexistent" }, true)).toBe(0);
    });
});

describe("muxlog siblingCandidateDirs", () => {
    it("returns both the 'data' and 'cef-cache' siblings of a trailing 'logs' segment", () => {
        const logs = path.join("channels", "chan-a", "versions", "0.55.19", "logs");
        const data = path.join("channels", "chan-a", "versions", "0.55.19", "data");
        const cefCache = path.join("channels", "chan-a", "versions", "0.55.19", "cef-cache");
        expect(siblingCandidateDirs(logs)).toEqual([data, cefCache]);
    });

    it("returns an empty array for a directory that isn't named 'logs'", () => {
        expect(siblingCandidateDirs(path.join("channels", "chan-a", "versions", "0.55.19", "data"))).toEqual([]);
    });
});

describe("muxlog checkLiveness", () => {
    let root;
    let server;

    beforeEach(() => {
        root = fs.mkdtempSync(path.join(os.tmpdir(), "muxlog-liveness-test-"));
    });

    afterEach(() => {
        fs.rmSync(root, { recursive: true, force: true });
        server?.close();
        server = undefined;
    });

    function makeInstance() {
        const logDir = path.join(root, "logs");
        const dataDir = path.join(root, "data");
        const cefCacheDir = path.join(root, "cef-cache");
        fs.mkdirSync(logDir, { recursive: true });
        fs.mkdirSync(dataDir, { recursive: true });
        fs.mkdirSync(cefCacheDir, { recursive: true });
        return { logDir, dataDir, cefCacheDir };
    }

    // A real HTTP server answering GET /health with the exact shape
    // agentmux-cef's own IPC server returns ({"status":"ok",...}) — probePort
    // now speaks HTTP, not raw TCP, so the test double has to match (reagent
    // P2 on PR #2742).
    function listenWithHealthResponse() {
        return new Promise((resolve) => {
            server = http.createServer((req, res) => {
                if (req.url === "/health") {
                    res.writeHead(200, { "Content-Type": "application/json" });
                    res.end(JSON.stringify({ status: "ok", version: "0.55.19" }));
                } else {
                    res.writeHead(404);
                    res.end();
                }
            });
            server.listen(0, "127.0.0.1", () => resolve(server.address().port));
        });
    }

    // A listener that accepts the TCP connection (so a bare connect-only
    // probe would false-positive "live") but isn't AgentMux's IPC server at
    // all — the exact scenario reagent P2 described (OS reassigns a dead
    // instance's port to an unrelated local service).
    function listenAsUnrelatedService() {
        return new Promise((resolve) => {
            server = net.createServer((sock) => sock.end("not an agentmux server\n"));
            server.listen(0, "127.0.0.1", () => resolve(server.address().port));
        });
    }

    it("returns '?' when there's no sibling data dir at all", async () => {
        const logDir = path.join(root, "logs");
        fs.mkdirSync(logDir, { recursive: true });
        // no "data" dir created
        expect(await checkLiveness(logDir)).toBe("?");
    });

    it("returns '?' when the data dir exists but has no ipc-port-* file", async () => {
        const { logDir } = makeInstance();
        expect(await checkLiveness(logDir)).toBe("?");
    });

    it("returns '?' for a malformed port file (no ':' separator, or non-numeric port)", async () => {
        const { logDir, dataDir } = makeInstance();
        fs.writeFileSync(path.join(dataDir, "ipc-port-abc123"), "not-a-port-file");
        expect(await checkLiveness(logDir)).toBe("?");
    });

    it("returns 'live' when the recorded port answers GET /health with status:ok", async () => {
        const { logDir, dataDir } = makeInstance();
        const port = await listenWithHealthResponse();
        fs.writeFileSync(path.join(dataDir, "ipc-port-abc123"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("live");
    });

    // reagent P2 on PR #2752: crates/cef/src/lib.rs writes the bare
    // filename "ipc-port" (no trailing hyphen) when AGENTMUX_IPC_HASH is
    // unset (task dev:standalone, no launcher) — startsWith("ipc-port-")
    // alone misses this exact literal.
    it("returns 'live' via the bare 'ipc-port' filename (no hash suffix, e.g. task dev:standalone)", async () => {
        const { logDir, dataDir } = makeInstance();
        const port = await listenWithHealthResponse();
        fs.writeFileSync(path.join(dataDir, "ipc-port"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("live");
    });

    // reagent P1 on PR #2752: crates/cef/src/lib.rs writes the port file
    // to the `cef-cache` sibling (not `data`) for `task dev` instances
    // (is_dev_build_exe branch) — checking only `data` always reports '?'
    // for the primary dev workflow, even when it's genuinely live.
    it("returns 'live' via a port file in the 'cef-cache' sibling (task dev's actual layout)", async () => {
        const { logDir, cefCacheDir } = makeInstance();
        const port = await listenWithHealthResponse();
        fs.writeFileSync(path.join(cefCacheDir, "ipc-port"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("live");
    });

    // reagent P2 on PR #2742: a raw TCP connect alone isn't proof of
    // liveness — something else could be listening on a reassigned port.
    it("returns 'dead' when something IS listening but doesn't answer as AgentMux's IPC server", async () => {
        const { logDir, dataDir } = makeInstance();
        const port = await listenAsUnrelatedService();
        fs.writeFileSync(path.join(dataDir, "ipc-port-abc123"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("dead");
    });

    // reagent P1 on PR #2742: a dev-mode data dir can hold a stale port file
    // (crashed process, never cleaned up) alongside a live one — checking
    // only the first readdirSync result (unspecified order) risks probing
    // the dead one and reporting "dead" for a genuinely live instance.
    it("returns 'live' when multiple ipc-port-* files exist and only one is actually alive", async () => {
        const { logDir, dataDir } = makeInstance();
        const livePort = await listenWithHealthResponse();
        // A stale port file pointing at a port nothing is listening on —
        // simulates a crashed prior run's never-cleaned-up port file.
        fs.writeFileSync(path.join(dataDir, "ipc-port-stalehash"), "1:dead-token");
        fs.writeFileSync(path.join(dataDir, "ipc-port-livehash"), `${livePort}:live-token`);
        expect(await checkLiveness(logDir)).toBe("live");
    });

    it("returns 'dead' when the recorded port has nothing listening on it", async () => {
        const { logDir, dataDir } = makeInstance();
        // Bind then immediately close — the port is very likely free again,
        // and nothing else in this test process will grab it in between.
        const port = await listenWithHealthResponse();
        await new Promise((resolve) => server.close(resolve));
        fs.writeFileSync(path.join(dataDir, "ipc-port-abc123"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("dead");
    });
});

// ─── SPEC_MUXLOG_AGENT_ADMISSION_TIMELINE_2026_09_27.md ─────────────────────
// The 2026-09-27 AgentA incident, as fixture lines: two instances on one
// computer (0.57.6 holds a stale relay lease after AgentA quit there; 0.57.8
// is fenced, takes over, gets `released: 0`, is fenced again), spanning a
// UTC-midnight rotation.
const L = (ts, level, target, fields) => JSON.stringify({ timestamp: ts, level, fields, target });
const UID = "d76da857-56b7-402f-80a4-1ceb7ab1d457";
const OLD_CHAN = "local-main-b28b7a-7a8245ae";
const NEW_CHAN = "local-main-b28b7a-6addbd3a";
const HOLDER = `this computer, channel ${OLD_CHAN}, v0.57.6`;
const OLD_BLOCK = "2dd55998-aaaa-bbbb-cccc-000000000001";
const NEW_BLOCK = "f65742e8-aaaa-bbbb-cccc-000000000002";
const OLD_LINES = [
    L("2026-09-26T23:40:00.000000Z", "INFO", "agentmux_srv::backend::blockcontroller::spawn", { message: "persistent process spawned", block_id: OLD_BLOCK, pid: 1 }),
    L("2026-09-26T23:52:19.355229Z", "INFO", "agentmux_srv::backend::reactive::registry", { message: "registry: entry changed hands since this spawn registered — skipping remove", agent_id: "AgentA", expected_nonce: 2 }),
    L("2026-09-26T23:52:19.379843Z", "INFO", "agentmux_srv::server::self_quit", { message: "self-quit", block_id: OLD_BLOCK, agent: "AgentA", released_claims: 0 }),
    L("2026-09-26T23:57:57.171134Z", "INFO", "agentmux_srv::server::agent_takeover", { message: "agent_admission.takeover: release handled", uid: UID, released: 0, to: `channel ${NEW_CHAN}, v0.57.8` }),
];
const NEW_LINES_26 = [
    L("2026-09-26T23:53:11.905461Z", "INFO", "agentmux_srv::backend::agent_admission", { message: "agent_admission.granted", agent: "agenta", uid: UID, block_id: NEW_BLOCK, epoch: 6 }),
    L("2026-09-26T23:53:11.908446Z", "INFO", "agentmux_srv::backend::blockcontroller::spawn", { message: "persistent process spawned", block_id: NEW_BLOCK, pid: 19584 }),
    L("2026-09-26T23:56:31.080654Z", "ERROR", "agentmux_srv::backend::agent_admission", { message: "agent_admission.fenced: the muxbus relay says another instance holds this agent — stopping this one", agent: "agenta", holder: HOLDER }),
    L("2026-09-26T23:57:23.190986Z", "WARN", "agentmux_srv::backend::agent_admission", { message: "agent_admission.denied: the muxbus relay says another instance holds this agent", agent: "AgentA", uid: UID, holder: HOLDER }),
    L("2026-09-26T23:57:57.170634Z", "INFO", "agentmux_srv::server::agent_takeover", { message: "agent_admission.takeover: requesting release from the holder", uid: UID, block_id: NEW_BLOCK, holder_channel: OLD_CHAN }),
    L("2026-09-26T23:57:57.311102Z", "INFO", "agentmux_srv::backend::agent_admission", { message: "agent_admission.granted", agent: "AgentA", uid: UID, block_id: NEW_BLOCK, epoch: 7 }),
    L("2026-09-26T23:57:57.314815Z", "INFO", "agentmux_srv::backend::blockcontroller::spawn", { message: "persistent process spawned", block_id: NEW_BLOCK, pid: 2736 }),
    L("2026-09-26T23:58:09.290247Z", "ERROR", "agentmux_srv::backend::agent_admission", { message: "agent_admission.fenced: the muxbus relay says another instance holds this agent — stopping this one", agent: "agenta", holder: HOLDER }),
    L("2026-09-26T23:59:00.000000Z", "INFO", "agentmux_srv::backend::agent_admission", { message: "agent_admission.granted", agent: "SomeoneElse", uid: "00000000-0000-4000-8000-000000000000", block_id: "99999999-aaaa-bbbb-cccc-000000000009", epoch: 1 }),
];
const NEW_LINES_27 = [
    L("2026-09-27T00:00:05.000000Z", "WARN", "agentmux_srv::backend::agent_admission", { message: "agent_admission.denied: the muxbus relay says another instance holds this agent", agent: "AgentA", uid: UID, holder: HOLDER }),
];

function writeLog(home, chan, ver, date, lines) {
    const dir = path.join(home, ".agentmux", "channels", chan, "versions", ver, "logs");
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, `agentmuxsrv-v${ver}.log.${date}`), lines.join("\n") + "\n");
    return dir;
}

function writeIncident(home) {
    writeLog(home, OLD_CHAN, "0.57.6", "2026-09-26", OLD_LINES);
    writeLog(home, NEW_CHAN, "0.57.8", "2026-09-26", NEW_LINES_26);
    writeLog(home, NEW_CHAN, "0.57.8", "2026-09-27", NEW_LINES_27);
    // An unrelated old instance whose only file predates the window.
    writeLog(home, "local-main-b28b7a-11111111", "0.55.1", "2026-08-01", [NEW_LINES_26[0]]);
}

const MUXLOG = fileURLToPath(new URL("./muxlog.mjs", import.meta.url));
function runMuxlog(home, args) {
    const r = spawnSync(process.execPath, [MUXLOG, ...args], {
        env: { ...process.env, HOME: home, USERPROFILE: home, AGENTMUX_CHANNEL: "", AGENTMUX_AGENT_ID: "", AGENTMUX_BLOCKID: "" },
        encoding: "utf8",
    });
    return (r.stdout + r.stderr).replace(/\x1b\[[0-9;]*m/g, "");
}

describe("muxlog rotated files (§3.1)", () => {
    it("splits a daily-rotated name into stem and date", () => {
        expect(logNameParts("agentmuxsrv-v0.57.8.log.2026-09-26")).toEqual({ stem: "agentmuxsrv-v0.57.8", date: "2026-09-26" });
        expect(logNameParts("agentmux-launcher.log")).toEqual({ stem: "agentmux-launcher", date: null });
        expect(logNameParts("notes.txt")).toBeNull();
    });

    it("keeps the files a window spanning UTC midnight can touch, oldest first", () => {
        const names = ["s.log.2026-09-27", "s.log.2026-09-25", "s.log.2026-09-26", "s.log"];
        expect(selectWindowNames(names, "2026-09-26T23:50", undefined)).toEqual(["s.log.2026-09-26", "s.log.2026-09-27", "s.log"]);
        expect(selectWindowNames(names, "2026-09-26T23:50", "2026-09-26T23:59")).toEqual(["s.log.2026-09-26", "s.log"]);
        expect(selectWindowNames(names, undefined, "2026-09-25T12:00")).toEqual(["s.log.2026-09-25", "s.log"]);
    });

    it("reads yesterday's file when --since reaches it (the incident was invisible before)", () => {
        writeIncident(root);
        const out = runMuxlog(root, ["srv", "-i", "6addbd3a", "--since", "2026-09-26T23:50", "--grep", "admission", "cat"]);
        expect(out).toContain("23:56:31");
        expect(out).toContain("00:00:05");
    });

    it("breaks an mtime tie between daily files by the later date in the name", () => {
        const f = (date) => ({ file: `/l/agentmuxsrv-v0.58.0.log.${date}`, mtime: 5 });
        expect([f("2026-09-26"), f("2027-01-01"), f("2026-09-27")].sort(newestLogFirst).map((e) => e.file.slice(-10))).toEqual([
            "2027-01-01",
            "2026-09-27",
            "2026-09-26",
        ]);
        expect(newestLogFirst({ file: "/l/a.log.2026-09-26", mtime: 9 }, { file: "/l/a.log.2026-09-27", mtime: 5 })).toBeLessThan(0);
    });

    it("without --since still reads only the newest file", () => {
        writeIncident(root);
        const out = runMuxlog(root, ["srv", "-i", "6addbd3a", "cat"]);
        expect(out).toContain("00:00:05");
        expect(out).not.toContain("23:56:31");
    });
});

describe("muxlog --agent (§3.3)", () => {
    const lines = [...OLD_LINES, ...NEW_LINES_26];

    it("expands a name to its uid and pane block ids, and a uid back to the name", () => {
        const byName = expandAgentIds(lines, "AgentA");
        expect(byName.ids.has(UID)).toBe(true);
        expect(byName.blocks.has(NEW_BLOCK)).toBe(true);
        expect(byName.blocks.has(OLD_BLOCK)).toBe(true);
        expect(expandAgentIds(lines, UID).ids.has("agenta")).toBe(true);
    });

    it("matches every spelling and block-id-only lines, but not another agent", () => {
        const m = makeAgentMatcher(expandAgentIds(lines, "agenta"));
        expect(lines.filter((l) => m(JSON.parse(l))).length).toBe(lines.length - 1);
        expect(m(JSON.parse(NEW_LINES_26.at(-1)))).toBe(false);
    });

    it("filters renderLine through the matcher", () => {
        const agentMatch = makeAgentMatcher(expandAgentIds(lines, "AgentA"));
        expect(renderLine(NEW_LINES_26[1], { agentMatch })).toContain("persistent process spawned");
        expect(renderLine(NEW_LINES_26.at(-1), { agentMatch })).toBeNull();
    });
});

describe("muxlog --instances (§3.2)", () => {
    it("tags an instance with its version and channel hash", () => {
        expect(instanceTag({ source: `channel:${OLD_CHAN}`, version: "0.57.6" })).toBe("v0.57.6/7a8245ae");
        expect(instanceTag({ source: "dev:my-branch", version: "0.58.0" })).toBe("v0.58.0/my-branch");
        expect(instanceTag({ source: "shared", version: "0.55.1" })).toBe("v0.55.1/shared");
    });

    it("groups rotated files per instance and filters by substring or liveness", () => {
        const c = (dir, name, mtime) => ({ file: path.join(dir, name), source: "channel:" + dir, version: name.match(/v([\d.]+)\.log/)[1], mtime });
        const cands = [c("a", "agentmuxsrv-v1.0.0.log.2026-09-26", 1), c("a", "agentmuxsrv-v1.0.0.log.2026-09-27", 2), c("b", "agentmuxsrv-v2.0.0.log.2026-09-27", 3)];
        expect(groupInstances(cands, "all").map((g) => g.files.length)).toEqual([1, 2]);
        expect(groupInstances(cands, "b").length).toBe(1);
        expect(groupInstances(cands, "live", { b: "dead" }).map((g) => g.dir)).toEqual(["a"]);
    });

    it("merges chronologically and keeps instance order for equal timestamps", () => {
        const a = [{ ts: "2026-09-26T23:57:57.1Z", tag: "a" }, { ts: "2026-09-26T23:58:00Z", tag: "a" }];
        const b = [{ ts: "2026-09-26T23:52:00Z", tag: "b" }, { ts: "2026-09-26T23:57:57.1Z", tag: "b" }];
        expect(mergeTimelines([a, b]).map((e) => `${e.tag}@${e.ts.slice(14, 19)}`)).toEqual(["b@52:00", "a@57:57", "b@57:57", "a@58:00"]);
    });
});

describe("muxlog admission (§3.4)", () => {
    const entries = (lines, tag) => lines.map((l) => { const j = JSON.parse(l); return { ts: j.timestamp, tag, j }; });
    const requester = (lines) => ({ tag: "v0.57.8/6addbd3a", source: `channel:${NEW_CHAN}`, file: "x", entries: entries(lines, "n") });
    const holder = (lines) => ({ tag: "v0.57.6/7a8245ae", source: `channel:${OLD_CHAN}`, file: "y", entries: entries(lines, "o") });

    it("reads the relay's holder channel", () => {
        expect(holderChannel(HOLDER)).toBe(OLD_CHAN);
        expect(holderChannel(undefined)).toBeNull();
    });

    it("summarizes each side of the incident", () => {
        const h = summarizeInstance(entries(OLD_LINES, "o"));
        expect(h.live).toBe(false);
        expect(h.released0At).toBe("2026-09-26T23:57:57.171134Z");
        const r = summarizeInstance(entries(NEW_LINES_26.slice(0, -1), "n"));
        expect(r.state).toBe("fenced");
        expect(r.holder).toBe(HOLDER);
        expect(r.live).toBe(false);
    });

    it("diagnoses the stale relay lease and cites every fix", () => {
        const { diagnoses } = admissionVerdict([requester(NEW_LINES_26.slice(0, -1)), holder(OLD_LINES)]);
        expect(diagnoses).toHaveLength(1);
        expect(diagnoses[0]).toMatch(/^stale relay lease/);
        expect(diagnoses[0]).toContain("released: 0");
        for (const pr of ["#3897", "#3899", "#3903", "#3908"]) expect(diagnoses[0]).toContain(pr);
    });

    it("doesn't call it stale when the holder has a live pane", () => {
        const { diagnoses } = admissionVerdict([requester(NEW_LINES_26.slice(0, 3)), holder([OLD_LINES[0]])]);
        expect(diagnoses).toHaveLength(1);
        expect(diagnoses[0]).toMatch(/genuinely runs the agent/);
    });

    it("gives no diagnosis on a clean grant", () => {
        expect(admissionVerdict([requester(NEW_LINES_26.slice(0, 2))]).diagnoses).toEqual([]);
    });

    it("shows the admission facts inline", () => {
        const s = inlineFieldsSuffix({ holder: "h", released: 0, block_id: "f65742e8-aaaa", message: "m" }, ["holder", "released", "block_id"]);
        expect(s.replace(/\x1b\[[0-9;]*m/g, "")).toBe("  holder=h released=0 block_id=f65742e8");
    });

    it("end to end: one command prints both instances' timeline and the diagnosis, for any spelling of the agent", () => {
        writeIncident(root);
        const outs = ["AgentA", "agenta", UID].map((a) => runMuxlog(root, ["admission", a, "--since", "2026-09-26T23:50"]));
        const out = outs[0];
        expect(out).toContain("v0.57.6/7a8245ae");
        expect(out).toContain("v0.57.8/6addbd3a");
        expect(out).toContain("released=0");
        expect(out).toContain("00:00:05");
        expect(out).toMatch(/diagnosis: stale relay lease/);
        expect(out).not.toContain("v0.55.1");
        expect(out).not.toContain("SomeoneElse");
        const body = (s) => s.split("\n").slice(1).join("\n");
        expect(body(outs[1])).toBe(body(out));
        expect(body(outs[2])).toBe(body(out));
    });
});

describe("muxlog opens (SPEC_AGENT_OPEN_LATENCY §4.6)", () => {
    const OPEN_LAZO =
        '[agent-open] agent="Lazo" block=1a2b3c4d source=my-agents outcome=quiet total=1341 cli=180 cli_source=installed ' +
        "config=402 history_read=610 history_lines=5000 painted=812 revealed=815 quiet=1341 auth=authenticated";
    const OPEN_RESTORED = '[agent-open] agent="Agent A" block=99887766 source=mount outcome=unsettled total=15500 painted=300 revealed=320 hidden=1';
    const hostLine = (ts, message) =>
        JSON.stringify({ timestamp: ts, level: "INFO", fields: { message: `[fe] ${message}`, module: "console", data: "Some(Null)" }, target: "agentmux_cef::commands::backend" });
    function writeHostLog(home, chan, ver, date, lines) {
        const dir = path.join(home, ".agentmux", "channels", chan, "versions", ver, "logs");
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(path.join(dir, `agentmux-host-v${ver}.log.${date}`), lines.join("\n") + "\n");
    }

    it("parses an open line, with quoted names and numbers", () => {
        expect(parseAgentOpenLine(`[fe] ${OPEN_LAZO}`)).toMatchObject({
            agent: "Lazo", block: "1a2b3c4d", source: "my-agents", outcome: "quiet",
            total: 1341, cli: 180, cli_source: "installed", history_lines: 5000, painted: 812, quiet: 1341, auth: "authenticated",
        });
        expect(parseAgentOpenLine(OPEN_RESTORED)).toMatchObject({ agent: "Agent A", outcome: "unsettled", hidden: 1 });
        expect(parseAgentOpenLine("[fe] [agent] Launching agent definition Lazo")).toBeNull();
    });

    it("takes nearest-rank percentiles, ignoring missing values", () => {
        expect(percentile([5, 1, 3, undefined, 2, 4], 50)).toBe(3);
        expect(percentile([5, 1, 3, 2, 4], 95)).toBe(5);
        expect(percentile([], 50)).toBeNull();
    });

    it("summarizes per source; quiet percentiles only count opens that went quiet", () => {
        const opens = [
            { source: "my-agents", outcome: "quiet", painted: 800, quiet: 1300, cli_source: "installed" },
            { source: "my-agents", outcome: "quiet", painted: 400, quiet: 900, cli_source: "local_install" },
            { source: "my-agents", outcome: "unsettled", painted: 600 },
            { source: "mount", outcome: "quiet", painted: 200, quiet: 700 },
        ];
        const [mine, mount] = summarizeOpens(opens);
        expect(mine).toMatchObject({ source: "my-agents", n: 3, installs: 1, outcomes: { quiet: 2, unsettled: 1 } });
        expect(mine.painted).toEqual({ p50: 600, p95: 800 });
        expect(mine.quiet).toEqual({ p50: 900, p95: 1300 });
        expect(mount).toMatchObject({ source: "mount", n: 1 });
    });

    it("end to end: rows from every instance, filtered by agent, then the summary", () => {
        const home = fs.mkdtempSync(path.join(os.tmpdir(), "muxlog-opens-test-"));
        try {
            writeHostLog(home, "local-main-b28b7a-7a8245ae", "0.58.0", "2026-09-27", [
                hostLine("2026-09-27T04:01:57.100000Z", OPEN_LAZO),
                hostLine("2026-09-27T04:02:10.000000Z", "[perf] long-task 91ms"),
            ]);
            writeHostLog(home, "local-main-b28b7a-6addbd3a", "0.58.1", "2026-09-27", [hostLine("2026-09-27T05:00:00.000000Z", OPEN_RESTORED)]);
            const all = runMuxlog(home, ["opens", "--since", "2026-09-27T00:00"]);
            expect(all).toContain("v0.58.0/7a8245ae");
            expect(all).toContain("v0.58.1/6addbd3a");
            expect(all).toMatch(/Lazo\s+my-agents\s+quiet\s+180 installed\s+812\s+815\s+1341\s+1341 authenticated/);
            expect(all).toContain("Agent A");
            expect(all).toMatch(/my-agents 1 open \(quiet 1\); first row p50 812 p95 812; quiet p50 1341 p95 1341; 1 installed the CLI/);
            expect(all).not.toContain("long-task");

            const one = runMuxlog(home, ["opens", "lazo", "--since", "2026-09-27T00:00"]);
            expect(one).toContain("Lazo");
            expect(one).not.toContain("Agent A");

            expect(runMuxlog(home, ["opens", "nobody", "--since", "2026-09-27T00:00"])).toContain("no [agent-open] lines for 'nobody'");
        } finally {
            fs.rmSync(home, { recursive: true, force: true });
        }
    });
});
