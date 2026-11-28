// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "pack-windows-symbols.sh");

let dir;
let release;
beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), "pack-symbols-test-"));
    release = path.join(dir, "target", "release");
    fs.mkdirSync(release, { recursive: true });
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

const ARCHIVE = "AgentMux_9.9.9_windows-symbols.tar.xz";

// The files the script says it packed, from its own report line.
const packed = (stdout) =>
    stdout
        .split(/\r?\n/)
        .find((l) => l.includes(" packed "))
        .replace(/^.* packed /, "")
        .replace(/ into .*$/, "")
        .split(" ")
        .sort();

const pack = () => spawnSync("bash", [script, "9.9.9", path.join(dir, "out"), path.join(dir, "target")], { encoding: "utf8" });

describe("pack-windows-symbols.sh", () => {
    it("packs srv's PDB under symbols/, away from the files a release uploads", () => {
        fs.writeFileSync(path.join(release, "agentmux_srv.pdb"), "pdb");
        const r = pack();
        expect(r.status, r.stderr).toBe(0);
        expect(fs.statSync(path.join(dir, "out", "symbols", ARCHIVE)).size).toBeGreaterThan(0);
        // Nothing beside the installers: the upload steps glob the out dir itself, never its symbols/ folder.
        expect(fs.readdirSync(path.join(dir, "out"))).toEqual(["symbols"]);
        expect(packed(r.stdout)).toEqual(["agentmux_srv.pdb"]);
    });

    it("adds the launcher's and the host's PDB when the build made them", () => {
        for (const n of ["agentmux_srv.pdb", "agentmux_launcher.pdb", "agentmux_cef.pdb"]) fs.writeFileSync(path.join(release, n), "pdb");
        const r = pack();
        expect(r.status, r.stderr).toBe(0);
        expect(packed(r.stdout)).toEqual(["agentmux_cef.pdb", "agentmux_launcher.pdb", "agentmux_srv.pdb"]);
    });

    it("fails when srv's PDB is missing or empty, so a release never goes out with no way to read a wedge", () => {
        const missing = pack();
        expect(missing.status).toBe(1);
        expect(missing.stderr).toContain("agentmux_srv.pdb");
        fs.writeFileSync(path.join(release, "agentmux_srv.pdb"), "");
        expect(pack().status).toBe(1);
        expect(fs.existsSync(path.join(dir, "out", "symbols"))).toBe(false);
    });
});
