// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
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
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { checkLiveness, filterByInstance, glob, matchesOwnChannel, pickCandidate, printLastLines, renderLine, siblingDataDir } from "./muxlog.mjs";

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

describe("muxlog siblingDataDir", () => {
    it("swaps a trailing 'logs' segment for 'data'", () => {
        const logs = path.join("channels", "chan-a", "versions", "0.55.19", "logs");
        const data = path.join("channels", "chan-a", "versions", "0.55.19", "data");
        expect(siblingDataDir(logs)).toBe(data);
    });

    it("returns null for a directory that isn't named 'logs'", () => {
        expect(siblingDataDir(path.join("channels", "chan-a", "versions", "0.55.19", "data"))).toBeNull();
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
        fs.mkdirSync(logDir, { recursive: true });
        fs.mkdirSync(dataDir, { recursive: true });
        return { logDir, dataDir };
    }

    function listenOnEphemeralPort() {
        return new Promise((resolve) => {
            server = net.createServer();
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

    it("returns 'live' when something is actually listening on the recorded port", async () => {
        const { logDir, dataDir } = makeInstance();
        const port = await listenOnEphemeralPort();
        fs.writeFileSync(path.join(dataDir, "ipc-port-abc123"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("live");
    });

    it("returns 'dead' when the recorded port has nothing listening on it", async () => {
        const { logDir, dataDir } = makeInstance();
        // Bind then immediately close — the port is very likely free again,
        // and nothing else in this test process will grab it in between.
        const port = await listenOnEphemeralPort();
        await new Promise((resolve) => server.close(resolve));
        fs.writeFileSync(path.join(dataDir, "ipc-port-abc123"), `${port}:some-token`);
        expect(await checkLiveness(logDir)).toBe("dead");
    });
});
