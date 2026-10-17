// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// scripts/extract-changelog.sh writes a release's notes. With
// --include-unpublished it also lists the older versions that never got a
// GitHub release: v0.59.29's build failed, so it was never published, and its
// 30 entries appeared on no release page although they shipped in v0.59.30.
// A fake `gh` on PATH says which releases exist.

import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "extract-changelog.sh");

const HISTORY = `# AgentMux Version History

## 0.9.4 — 2026-10-17

- four

## 0.9.3 — 2026-10-16

- three a

- three b

## 0.9.2 — 2026-10-15

- two

## 0.9.1 — 2026-10-14

- one
`;

let dirs = [];
afterEach(() => {
    for (const d of dirs) fs.rmSync(d, { recursive: true, force: true });
    dirs = [];
});

/** Run the script with `published` as the releases gh knows, or `ghError` as gh's failure. */
function run(args, { published = [], ghError = null } = {}) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "extract-changelog-"));
    dirs.push(dir);
    fs.writeFileSync(path.join(dir, "VERSION_HISTORY.md"), HISTORY);
    const fail = ghError ? `echo "${ghError}" >&2; exit 1` : `echo "release not found" >&2; exit 1`;
    const fake = `#!/usr/bin/env bash\ncase " ${published.join(" ")} " in *" $3 "*) exit 0;; esac\n${fail}\n`;
    fs.writeFileSync(path.join(dir, "gh"), fake, { mode: 0o755 });
    const r = spawnSync("bash", [script, ...args], {
        encoding: "utf8",
        env: { ...process.env, PATH: `${dir}${path.delimiter}${process.env.PATH}`, VERSION_HISTORY_FILE: path.join(dir, "VERSION_HISTORY.md") },
    });
    return { code: r.status, out: r.stdout.trimEnd(), err: r.stderr };
}

describe("extract-changelog.sh", () => {
    it("prints only the version's own entries by default", () => {
        expect(run(["0.9.4"]).out).toBe("- four");
    });

    it("adds each older unpublished version until a published one", () => {
        const { code, out } = run(["--include-unpublished", "0.9.4"], { published: ["v0.9.2", "v0.9.1"] });
        expect(code).toBe(0);
        expect(out).toBe(["- four", "", "### Also in this release: v0.9.3, which was never published", "", "- three a", "- three b"].join("\n"));
    });

    it("adds nothing when the previous version is published", () => {
        expect(run(["--include-unpublished", "0.9.4"], { published: ["v0.9.3"] }).out).toBe("- four");
    });

    it("stops, with a warning, when gh fails for another reason", () => {
        const { code, out, err } = run(["--include-unpublished", "0.9.4"], { ghError: "HTTP 401: Bad credentials" });
        expect(code).toBe(0);
        expect(out).toBe("- four");
        expect(err).toContain("couldn't check v0.9.3");
    });

    it("fails on a version with no entry", () => {
        expect(run(["--include-unpublished", "0.9.9"]).code).toBe(1);
    });
});
