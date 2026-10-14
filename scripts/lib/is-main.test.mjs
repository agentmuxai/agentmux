// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, describe, expect, it } from "vitest";
import { isMain } from "./is-main.mjs";

const dir = mkdtempSync(join(tmpdir(), "is-main-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

const LIB = new URL("./is-main.mjs", import.meta.url).href;

/** Write a script that prints isMain(import.meta.url) when it loads. */
function script(name) {
    const path = join(dir, name);
    writeFileSync(path, `import { isMain } from ${JSON.stringify(LIB)};\nconsole.log(${JSON.stringify(name)}, isMain(import.meta.url));\n`);
    return path;
}

const run = (path) => execFileSync(process.execPath, [path], { encoding: "utf8" }).trim().split("\n");

describe("isMain, run by node", () => {
    it("is true for the script node runs, and false for one it imports", () => {
        const imported = script("imported.mjs");
        const entry = join(dir, "entry.mjs");
        writeFileSync(entry, `import ${JSON.stringify(pathToFileURL(imported).href)};\nimport { isMain } from ${JSON.stringify(LIB)};\nconsole.log("entry.mjs", isMain(import.meta.url));\n`);
        expect(run(entry)).toEqual(["imported.mjs false", "entry.mjs true"]);
    });

    it("works for a path with a space in it", () => {
        mkdirSync(join(dir, "with space"), { recursive: true });
        expect(run(script("with space/spaced.mjs"))).toEqual(["with space/spaced.mjs true"]);
    });

    it("follows a symlink to the script", (ctx) => {
        const target = script("target.mjs");
        const link = join(dir, "link.mjs");
        try {
            symlinkSync(target, link);
        } catch {
            ctx.skip(); // Windows without the symlink privilege
        }
        expect(run(link)).toEqual(["target.mjs true"]);
    });
});

describe("isMain, called directly", () => {
    const path = script("direct.mjs");
    const url = pathToFileURL(path).href;

    it("compares the module's URL with the invoked path", () => {
        expect(isMain(url, path)).toBe(true);
        expect(isMain(url, script("other.mjs"))).toBe(false);
    });

    it("is false with no invoked script, a missing one, or a URL that isn't a file", () => {
        expect(isMain(url, undefined)).toBe(false);
        expect(isMain(url, join(dir, "missing.mjs"))).toBe(false);
        expect(isMain("data:text/javascript,1", path)).toBe(false);
    });

    it.runIf(process.platform === "win32")("ignores case on Windows", () => {
        expect(isMain(url, path.toUpperCase(), "win32")).toBe(true);
    });
});
