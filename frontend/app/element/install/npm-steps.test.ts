// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import type { LineTone } from "./install-types";
import { DOWNLOAD_IDLE_MS, NpmStepTracker } from "./npm-steps";

// Real `npm install @mariozechner/pi-coding-agent@0.73.1 --loglevel=verbose`
// runs (npm 11.13, cold cache), trimmed and with home paths scrubbed. Each
// line is prefixed with its stream: `err| ` or `out| `.
const fixtureDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../../test/fixtures/install");
const readFixture = (name: string): string[] =>
    readFileSync(resolve(fixtureDir, name), "utf8")
        .trimEnd()
        .split("\n")
        .map((l) => l.replace(/^(err|out)\| /, ""));

const statuses = (t: NpmStepTracker) => Object.fromEntries(t.snapshot().map((s) => [s.id, s.status]));

const ECHO = "$ npm install @mariozechner/pi-coding-agent@0.73.1 --prefix /x --no-audit --no-fund --progress=false --loglevel=verbose";

describe("NpmStepTracker", () => {
    it("starts with requirements active and the rest pending", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        expect(statuses(t)).toEqual({
            requirements: "active",
            download: "pending",
            setup: "pending",
            scripts: "pending",
            verify: "pending",
        });
        expect(t.snapshot().map((s) => s.label)).toEqual([
            "Check requirements",
            "Download packages",
            "Set up files",
            "Run setup scripts",
            "Check Pi is installed",
        ]);
    });

    it("treats the backend's command echo as a command line, not proof npm started", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        expect(t.line(ECHO, 0)).toBe("command");
        expect(statuses(t).requirements).toBe("active");
    });

    it("walks a real Pi install through every step in order", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        const lines = [ECHO, ...readFixture("npm-install-pi-success.txt")];
        const seen: string[] = [];
        let now = 0;
        for (const line of lines) {
            now += 10;
            t.line(line, now);
            const active = t.snapshot().find((s) => s.status === "active")?.id;
            if (active && seen[seen.length - 1] !== active) seen.push(active);
        }
        // Setup has no line of its own in npm 11, so on a replay with no
        // quiet gap it goes straight from download to scripts.
        expect(seen).toEqual(["requirements", "download", "scripts", "verify"]);
        expect(t.snapshot().find((s) => s.id === "download")?.hint).toBe("189 packages");

        t.succeed();
        expect(statuses(t)).toEqual({
            requirements: "done",
            download: "done",
            setup: "done",
            scripts: "done",
            verify: "done",
        });
        expect(t.snapshot().every((s) => s.subline === undefined)).toBe(true);
    });

    it("counts each tarball once even though npm logs it as both fetch and cache", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        const url = "https://registry.npmjs.org/@protobufjs/float/-/float-1.0.2.tgz";
        t.line(`npm http fetch GET 200 ${url} 312ms (cache miss)`, 1);
        t.line(`npm http cache @protobufjs/float@${url} 0ms (cache hit)`, 2);
        t.line("npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms (cache miss)", 3);
        const download = t.snapshot().find((s) => s.id === "download")!;
        expect(download.status).toBe("active");
        expect(download.hint).toBe("2 fetched");
        expect(download.subline).toBe("Downloading chalk");
    });

    it("shows the package being looked up, decoding scoped names", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm http fetch GET 200 https://registry.npmjs.org/@mariozechner%2fpi-coding-agent 201ms (cache miss)", 1);
        expect(t.snapshot().find((s) => s.id === "download")?.subline).toBe("Looking up @mariozechner/pi-coding-agent");
    });

    it("moves from download to setup after a quiet gap", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms (cache miss)", 1000);
        t.tick(1000 + DOWNLOAD_IDLE_MS - 1);
        expect(statuses(t).download).toBe("active");
        t.tick(1000 + DOWNLOAD_IDLE_MS);
        expect(statuses(t).download).toBe("done");
        expect(statuses(t).setup).toBe("active");
    });

    it("does not end download on a quiet gap before any tarball arrived", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm http fetch GET 200 https://registry.npmjs.org/chalk 12ms (cache miss)", 0);
        t.tick(DOWNLOAD_IDLE_MS * 10);
        expect(statuses(t).download).toBe("active");
    });

    it("names the running setup script", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm info run koffi@2.16.3 install node_modules/koffi node src/cnoke/cnoke.js -P . -D src/koffi --prebuild", 1);
        expect(statuses(t).scripts).toBe("active");
        expect(t.snapshot().find((s) => s.id === "scripts")?.subline).toBe("Running install for koffi");
        // The completion line is not a new script.
        t.line("npm info run koffi@2.16.3 install { code: 0, signal: null }", 2);
        expect(t.snapshot().find((s) => s.id === "scripts")?.subline).toBe("Running install for koffi");
    });

    it("skips setup scripts when none ran", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms (cache miss)", 1);
        t.line("added 1 package in 1s", 2);
        expect(statuses(t).verify).toBe("active");
        // Not settled until the install ends — script lines may still arrive.
        expect(statuses(t).scripts).toBe("pending");
        t.succeed();
        const scripts = t.snapshot().find((s) => s.id === "scripts")!;
        expect(scripts.status).toBe("skipped");
        expect(scripts.hint).toBe("none needed");
    });

    it("doesn't call scripts skipped when npm's stdout summary overtakes its stderr script lines", () => {
        // stdout and stderr are read by separate backend tasks, so the
        // summary can arrive before the lifecycle lines it follows.
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm http fetch GET 200 https://registry.npmjs.org/koffi/-/koffi-2.16.3.tgz 12ms", 1);
        t.line("added 189 packages in 7s", 2);
        expect(statuses(t).verify).toBe("active");
        t.line("npm info run koffi@2.16.3 install node_modules/koffi node src/cnoke/cnoke.js", 3);
        t.line("npm info run koffi@2.16.3 install { code: 0, signal: null }", 4);
        // Late script evidence doesn't reopen an earlier step or add a
        // second active one.
        expect(t.snapshot().filter((s) => s.status === "active").map((s) => s.id)).toEqual(["verify"]);
        t.succeed();
        const scripts = t.snapshot().find((s) => s.id === "scripts")!;
        expect(scripts.status).toBe("done");
        expect(scripts.hint).toBeUndefined();
    });

    it("colours only real errors and warnings", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        const tones: Record<string, LineTone> = {};
        for (const line of readFixture("npm-install-pi-success.txt")) tones[line] = t.line(line, 0);
        const coloured = Object.entries(tones).filter(([, tone]) => tone !== "normal");
        // The only non-normal lines in a healthy Pi install are npm's own
        // deprecation warnings — none of the stderr verbose chatter.
        expect(coloured.length).toBeGreaterThan(0);
        expect(coloured.every(([line, tone]) => tone === "warning" && line.startsWith("npm warn deprecated"))).toBe(true);
    });

    it("classifies a real network failure and points at the first error line", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        const lines = [ECHO, ...readFixture("npm-install-pi-network-error.txt")];
        const tones = lines.map((l, i) => t.line(l, i));
        const failure = t.fail("npm exited Some(1)");
        expect(failure.category).toBe("network");
        expect(failure.message).toBe("Couldn't reach the package server.");
        expect(lines[failure.firstErrorLine!]).toBe("npm error code ENOTFOUND");
        expect(tones[failure.firstErrorLine!]).toBe("error");
        expect(statuses(t)).toEqual({
            requirements: "done",
            download: "failed",
            setup: "pending",
            scripts: "pending",
            verify: "pending",
        });
        expect(t.snapshot().find((s) => s.id === "download")?.subline).toBe("Couldn't reach the package server.");
    });

    it("maps npm's EACCES and ENOSPC codes, in both old and new error prefixes", () => {
        for (const [line, category] of [
            ["npm error code EACCES", "permission"],
            ["npm ERR! code EPERM", "permission"],
            ["npm error code ENOSPC", "disk"],
            ["npm error code E404", "unknown"],
        ] as const) {
            const t = new NpmStepTracker("Pi");
            t.start();
            t.line(line, 0);
            expect(t.fail("npm exited Some(1)").category).toBe(category);
        }
    });

    it("fails requirements when npm can't be started", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line(ECHO, 0);
        const failure = t.fail("spawn npm: No such file or directory (os error 2)");
        expect(failure.category).toBe("missing_prereq");
        expect(failure.message).toBe("npm is needed first.");
        expect(statuses(t).requirements).toBe("failed");
    });

    it("fails the last step when npm succeeded but the program is missing", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("added 189 packages in 7s", 0);
        const failure = t.fail("npm install reported success but pi not found in /x/node_modules/.bin/");
        expect(failure.category).toBe("not_on_path");
        expect(statuses(t)).toMatchObject({ download: "done", setup: "done", verify: "failed" });
    });

    it("says something plain for an unrecognised failure", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm info run koffi@2.16.3 install node_modules/koffi node x", 0);
        const failure = t.fail({ code: "AMX_SOMETHING" });
        expect(failure.category).toBe("unknown");
        expect(failure.message).toBe('Something went wrong while running "Run setup scripts".');
    });

    it("treats registry 5xx responses as a network problem", () => {
        for (const code of ["E500", "E502", "E503", "E504"]) {
            const t = new NpmStepTracker("Pi");
            t.start();
            t.line(`npm error code ${code}`, 0);
            expect(t.fail("npm exited Some(1)").category).toBe("network");
        }
    });

    it("classifies the backend's typed disk-full and permission errors", () => {
        for (const [code, category, message] of [
            ["AMX-IO-001", "disk", "The disk is full."],
            ["AMX-IO-002", "permission", "AgentMux wasn't allowed to write the files."],
        ] as const) {
            const t = new NpmStepTracker("Pi");
            t.start();
            // The install directory is created before npm is spawned.
            const failure = t.fail({ code, message: "raw", details: { path: "/x" } });
            expect(failure.category).toBe(category);
            expect(failure.message).toBe(message);
            expect(statuses(t).requirements).toBe("failed");
        }
    });

    it("blames the first install step, not the passed requirements check, when npm fails early", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        // npm started (so requirements passed) but died before any download.
        t.line("npm verbose cli /usr/bin/node /usr/bin/npm", 0);
        t.line("npm error code EACCES", 1);
        const failure = t.fail("npm exited Some(1)");
        expect(failure.category).toBe("permission");
        expect(statuses(t)).toMatchObject({ requirements: "done", download: "failed" });
    });

    it("start() resets a failed run for Retry", () => {
        const t = new NpmStepTracker("Pi");
        t.start();
        t.line("npm error code ENOTFOUND", 0);
        t.fail("npm exited Some(1)");
        t.start();
        expect(statuses(t).download).toBe("pending");
        const t2 = t.fail("npm exited Some(1)");
        expect(t2.category).toBe("unknown");
        expect(t2.firstErrorLine).toBeUndefined();
    });
});
