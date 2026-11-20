// Copyright 2026, AgentMux Corp
// SPDX-License-Identifier: Apache-2.0
//
// release-collect-files.sh takes each package file of a release from the artifact that makes it, and stops the release
// on a package file found in any other artifact (a build job runs the built commit's code and can upload an artifact of
// its own). This builds artifact trees like `actions/download-artifact` leaves them and runs the script on them.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "release-collect-files.sh");
const dirs = [];

function tree(files) {
    const root = mkdtempSync(join(tmpdir(), "collect-"));
    dirs.push(root);
    const src = join(root, "release-artifacts");
    for (const f of files) {
        mkdirSync(dirname(join(src, f)), { recursive: true });
        writeFileSync(join(src, f), f);
    }
    mkdirSync(src, { recursive: true });
    return { src, out: join(root, "dist-flat") };
}
function run({ src, out }) {
    const r = spawnSync("bash", [SCRIPT, src, out], { encoding: "utf8" });
    return { status: r.status, text: `${r.stdout}${r.stderr}`, files: existsSync(out) ? readdirSync(out).sort() : [] };
}
afterEach(() => {
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

const GOOD = [
    "windows-portable/agentmux-0.1.0-x64-portable.7z",
    "windows-installer/AgentMux-0.1.0-x64-setup.exe",
    "windows-msix/AgentMux_0.1.0_x64.msix",
    "windows-frontend-maps/AgentMux_0.1.0_windows-frontend-maps.tar.xz",
    "linux-appimage/AgentMux-0.1.0-x86_64.AppImage",
    "linux-deb/agentmux_0.1.0_amd64.deb",
    "linux-rpm/agentmux-0.1.0.x86_64.rpm",
    "linux-tarball/agentmux-0.1.0-linux.tar.xz",
    "server-package-linux-x64/agentmux-headless-0.1.0-linux-x64.tar.gz",
    "server-package-linux-arm64/agentmux-headless-0.1.0-linux-arm64.tar.gz",
    "server-package-macos-arm64/agentmux-headless-0.1.0-macos-arm64.tar.gz",
    "server-package-windows-x64/agentmux-headless-0.1.0-windows-x64.zip",
];
const others = ["macos-dmg/AgentMux_0.1.0_arm64.dmg", "release-manifest/manifest.json", "suite-resource-trace-1-1/resource-trace.log"];

describe("release-collect-files.sh", () => {
    it("takes every package file from its own artifact and leaves the rest (the DMG, the manifest, a trace)", () => {
        const r = run(tree([...GOOD, ...others]));
        expect(r.status, r.text).toBe(0);
        expect(r.files).toEqual(GOOD.map((f) => f.split("/")[1]).sort());
    });

    it("takes a file from an artifact with a folder inside it", () => {
        const r = run(tree(["windows-msix/msix/AgentMux_0.1.0_x64.msix"]));
        expect(r.status, r.text).toBe(0);
        expect(r.files).toEqual(["AgentMux_0.1.0_x64.msix"]);
    });

    it.each([
        ["a server package tarball in a build artifact", "windows-installer/agentmux-headless-9.9.9-linux-x64.tar.gz"],
        ["a tarball in a server package artifact of another platform's kind (a zip under a linux one)", "server-package-linux-x64/agentmux-headless-9.9.9-windows-x64.zip"],
        ["an installer in the portable's artifact", "windows-portable/Evil-setup.exe"],
        ["an AppImage in an artifact of its own making", "my-extra/Other.AppImage"],
        ["a deb in the msix artifact", "windows-msix/x.deb"],
        ["a package file in the release manifest", "release-manifest/agentmux-0.1.0.tar.xz"],
        ["a .7z in a server package artifact", "server-package-macos-arm64/x.7z"],
        ["a package file at the top of the artifacts", "stray.rpm"],
    ])("stops the release on %s, names it, and does not pass it on", (_what, bad) => {
        const r = run(tree([...GOOD, bad]));
        expect(r.status, r.text).toBe(1);
        expect(r.text).toContain(`::error::a release package file came in an artifact that does not make it: ${bad}`);
        expect(r.files).not.toContain(bad.split("/").at(-1));
    });

    it("names every bad file, not only the first", () => {
        const r = run(tree([...GOOD, "a/one.exe", "b/two.msix"]));
        expect(r.status).toBe(1);
        expect(r.text).toContain("a/one.exe");
        expect(r.text).toContain("b/two.msix");
    });

    it("does not take a file whose name only contains a package name", () => {
        const r = run(tree(["windows-installer/setup.exe.txt", "linux-deb/readme.deb.sig"]));
        expect(r.status, r.text).toBe(0);
        expect(r.files).toEqual([]);
    });

    it("refuses without arguments", () => {
        const r = spawnSync("bash", [SCRIPT], { encoding: "utf8" });
        expect(r.status).toBe(1);
        expect(r.stderr).toContain("usage:");
    });
});
