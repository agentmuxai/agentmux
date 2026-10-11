// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// publish.mjs (which file in the docs a run's image replaces) and
// refresh.mjs (what a refresh runs, in which order).

import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { filesByName, plan, publish, runImages } from "./publish.mjs";
import { SUITE_ORDER, commands, parseArgs } from "./refresh.mjs";

describe("runImages", () => {
    it("names each image as the docs do: <id>, <id>-<size>, or the clip's file", () => {
        const imgs = runImages(
            {
                shots: [
                    { id: "chrome-window", file: "01-chrome-window.png" },
                    { id: "widget-agent", size: "small", file: "widget-agent-small.png" },
                    { id: "widget-toolchain", size: "small", file: "widget-toolchain-small.png", containsWorkspaceData: true },
                ],
                recordings: [{ id: "stash-tour", files: ["stash-tour.gif", "stash-tour.mp4"] }],
            },
            "/run"
        );
        expect(imgs.map((i) => [i.name, i.machineData])).toEqual([
            ["chrome-window.png", false],
            ["widget-agent-small.png", false],
            ["widget-toolchain-small.png", true],
            ["stash-tour.gif", false],
            ["stash-tour.mp4", false],
        ]);
    });
});

describe("publish", () => {
    function fixture() {
        const root = mkdtempSync(join(tmpdir(), "publish-"));
        const run = join(root, "run");
        const docs = join(root, "docs");
        const chrome = join(docs, "src", "assets", "screenshots", "chrome");
        const widgets = join(docs, "src", "assets", "screenshots", "widgets");
        mkdirSync(run);
        mkdirSync(chrome, { recursive: true });
        mkdirSync(widgets, { recursive: true });
        writeFileSync(join(run, "01-chrome-window.png"), "new window");
        writeFileSync(join(run, "02-chrome-top-bar.png"), "same bar");
        writeFileSync(join(run, "03-chrome-new.png"), "brand new");
        writeFileSync(join(run, "widget-toolchain-small.png"), "machine data");
        writeFileSync(join(chrome, "chrome-window.png"), "old window");
        writeFileSync(join(chrome, "chrome-top-bar.png"), "same bar");
        writeFileSync(join(widgets, "widget-toolchain-small.png"), "old toolchain");
        writeFileSync(
            join(run, "manifest.json"),
            JSON.stringify({
                shots: [
                    { id: "chrome-window", file: "01-chrome-window.png" },
                    { id: "chrome-top-bar", file: "02-chrome-top-bar.png" },
                    { id: "chrome-new", file: "03-chrome-new.png" },
                    { id: "widget-toolchain", size: "small", file: "widget-toolchain-small.png", containsWorkspaceData: true },
                ],
            })
        );
        return { run, docs, chrome, widgets };
    }

    it("replaces only images the docs already use, and never one that shows machine data", () => {
        const { run, docs, chrome, widgets } = fixture();
        const steps = publish({ docs, from: [run], log: () => {} });
        expect(steps.map((s) => [s.name, s.action])).toEqual([
            ["chrome-window.png", "replace"],
            ["chrome-top-bar.png", "replace"],
            ["chrome-new.png", "unused"],
            ["widget-toolchain-small.png", "skipped"],
        ]);
        expect(readFileSync(join(chrome, "chrome-window.png"), "utf8")).toBe("new window");
        expect(readFileSync(join(widgets, "widget-toolchain-small.png"), "utf8")).toBe("old toolchain");
    });

    it("changes nothing on a dry run", () => {
        const { run, docs, chrome } = fixture();
        publish({ docs, from: [run], dryRun: true, log: () => {} });
        expect(readFileSync(join(chrome, "chrome-window.png"), "utf8")).toBe("old window");
    });

    it("calls a name the docs use twice ambiguous instead of guessing", () => {
        const byName = new Map([["x.png", ["/a/x.png", "/b/x.png"]]]);
        const [step] = plan([{ source: __filename, name: "x.png", machineData: false }], byName);
        expect(step.action).toBe("ambiguous");
        expect(filesByName("/no/such/dir").size).toBe(0);
    });
});

describe("refresh", () => {
    it("runs the suites in the order that works, whatever order they're named in, then the clips", () => {
        const args = parseArgs(["--port", "9333", "--suites", "memory,agent", "--out", "/out", "--redact", "a=b"]);
        expect(args.suites).toEqual(["agent", "memory"]);
        const cmds = commands(args);
        expect(cmds.map(([label, script]) => `${label}:${script}`)).toEqual(["agent:capture.mjs", "memory:capture.mjs", "recordings:record.mjs"]);
        expect(cmds[0][2]).toEqual(expect.arrayContaining(["--suite", "agent", "--port", "9333", "--redact", "a=b"]));
        expect(parseArgs(["--no-recordings"]).recordings).toBe(false);
        expect(parseArgs([]).suites).toEqual(SUITE_ORDER);
        expect(() => parseArgs(["--suites", "nope"])).toThrow(/unknown suite/);
    });
});
