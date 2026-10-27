// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { banner, chooseHome, shellLines, USE_REAL_HOME } from "./dev-home.mjs";
import { findInstancePort, pickHome, resolvePort } from "./dev-seed.mjs";

let tmp;
beforeEach(() => {
    tmp = mkdtempSync(join(tmpdir(), "dev-home-"));
});
afterEach(() => rmSync(tmp, { recursive: true, force: true }));

const real = () => join(tmp, "real-home", ".agentmux");

describe("chooseHome", () => {
    it("gives a dev build a home of its own under the clone, by Vite port", () => {
        const a = chooseHome({ env: {}, cloneRoot: join(tmp, "clone"), port: 5206, realHome: real() });
        const b = chooseHome({ env: {}, cloneRoot: join(tmp, "clone"), port: 5300, realHome: real() });
        expect(a).toEqual({ mode: "isolated", home: join(tmp, "clone", ".dev-home", "5206") });
        expect(b.home).not.toBe(a.home);
    });

    it("isolates a build started by an agent, whatever else is in its environment", () => {
        const env = { AGENTMUX_AGENT_ID: "clamk", AGENTMUX_DATA_HOME: join(real(), "channels", "stable", "versions", "1", "data") };
        expect(chooseHome({ env, cloneRoot: tmp, port: 5206, realHome: real() }).mode).toBe("isolated");
    });

    it("keeps a home the caller names", () => {
        const own = join(tmp, "scratch");
        mkdirSync(own);
        expect(chooseHome({ env: { AGENTMUX_HOME_OVERRIDE: own }, cloneRoot: tmp, port: 5206, realHome: real() })).toEqual({ mode: "override", home: own });
    });

    it("refuses an override that is the real home, unless the real home is asked for", () => {
        mkdirSync(real(), { recursive: true });
        const env = { AGENTMUX_HOME_OVERRIDE: real() };
        expect(() => chooseHome({ env, cloneRoot: tmp, port: 5206, realHome: real() })).toThrow(/own ~\/\.agentmux/);
        expect(chooseHome({ env: { ...env, [USE_REAL_HOME]: "1" }, cloneRoot: tmp, port: 5206, realHome: real() }).mode).toBe("real");
    });

    it("uses the real home only when asked for it, and only for the exact value 1", () => {
        const pick = (v) => chooseHome({ env: { [USE_REAL_HOME]: v }, cloneRoot: tmp, port: 5206, realHome: real() }).mode;
        expect(pick("1")).toBe("real");
        for (const v of ["", "0", "true", "yes"]) expect(pick(v)).toBe("isolated");
    });
});

describe("what the build is told", () => {
    it("says plainly when the home is the real one", () => {
        expect(banner({ mode: "real", home: "C:\\Users\\me\\.agentmux" })).toMatch(/\(REAL: /);
        expect(banner({ mode: "isolated", home: "/clone/.dev-home/5206" })).toBe("[dev] home: /clone/.dev-home/5206 (isolated)");
        expect(banner({ mode: "override", home: "/x" })).toContain("AGENTMUX_HOME_OVERRIDE");
    });

    it("exports the home and drops an inherited data home; the real home is no override at all", () => {
        expect(shellLines({ mode: "isolated", home: "C:\\a b\\it's" })).toEqual([
            "export AGENTMUX_DEV_HOME_MODE=isolated",
            "unset AGENTMUX_DATA_HOME",
            "export AGENTMUX_HOME_OVERRIDE='C:/a b/it'\\''s'",
        ]);
        expect(shellLines({ mode: "real", home: real() })).toContain("unset AGENTMUX_HOME_OVERRIDE");
    });
});

describe("the home dev-seed picks", () => {
    it("takes --home, then the environment, then the one dev home in the clone", () => {
        mkdirSync(join(tmp, ".dev-home", "5206"), { recursive: true });
        expect(pickHome({ home: join(tmp, "h"), env: { AGENTMUX_HOME_OVERRIDE: join(tmp, "e") }, cwd: tmp })).toBe(join(tmp, "h"));
        expect(pickHome({ env: { AGENTMUX_HOME_OVERRIDE: join(tmp, "e") }, cwd: tmp })).toBe(join(tmp, "e"));
        expect(pickHome({ env: {}, cwd: tmp })).toBe(join(tmp, ".dev-home", "5206"));
    });

    it("asks when there is none or more than one", () => {
        expect(() => pickHome({ env: {}, cwd: tmp })).toThrow(/start `task dev` first/);
        mkdirSync(join(tmp, ".dev-home", "5206"), { recursive: true });
        mkdirSync(join(tmp, ".dev-home", "5300"), { recursive: true });
        expect(() => pickHome({ env: {}, cwd: tmp })).toThrow(/more than one/);
    });
});

/** A build's authkey.dev under `home`, as the CEF host writes it. */
const writeBuild = (home, { branch = "main", clone = "abc", port, pid, at = "2026-10-27T10:00:00Z" }) => {
    const dir = join(home, "dev", branch, clone, "data");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "authkey.dev"), JSON.stringify({ debug_port: port, host_pid: pid, created_at: at }));
};
const alive = (...pids) => (pid) => pids.includes(pid);

describe("the window dev-seed drives", () => {
    it("is the build running on the home, not whatever holds the usual port", () => {
        const mine = join(tmp, "clone-b", ".dev-home", "5210");
        const other = join(tmp, "clone-a", ".dev-home", "5206");
        writeBuild(other, { port: 9223, pid: 11 });
        writeBuild(mine, { port: 51234, pid: 22 });
        expect(findInstancePort(mine, alive(11, 22))).toBe(51234);
        expect(findInstancePort(other, alive(11, 22))).toBe(9223);
    });

    it("ignores a build that is no longer running, and takes the newest of the running ones", () => {
        const home = join(tmp, "h");
        writeBuild(home, { branch: "old", port: 9301, pid: 1, at: "2026-10-27T09:00:00Z" });
        writeBuild(home, { branch: "a", port: 9302, pid: 2, at: "2026-10-27T10:00:00Z" });
        writeBuild(home, { branch: "b", port: 9303, pid: 3, at: "2026-10-27T11:00:00Z" });
        expect(findInstancePort(home, alive(2, 3))).toBe(9303);
        expect(findInstancePort(home, alive(2))).toBe(9302);
        expect(findInstancePort(home, alive())).toBeUndefined();
        expect(findInstancePort(join(tmp, "empty"), alive(1))).toBeUndefined();
    });

    it("refuses a home with no running build, and a port that belongs to another build", () => {
        const home = join(tmp, "h");
        expect(() => resolvePort(home, undefined, alive(1))).toThrow(/no running dev build is using/);
        writeBuild(home, { port: 51234, pid: 1 });
        expect(resolvePort(home, undefined, alive(1))).toBe(51234);
        expect(resolvePort(home, 51234, alive(1))).toBe(51234);
        expect(() => resolvePort(home, 9223, alive(1))).toThrow(/port 9223 is not the build running on .* \(that one is on 51234\)/);
    });
});

// The Taskfile steps that choose the home run in a shell, so their exit status is part of the
// behaviour: a refused home must stop the build there, not carry on with the refused override.
describe.skipIf(spawnSync("bash", ["-c", "true"]).status !== 0)("the Taskfile's home step", () => {
    const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
    const taskfile = readFileSync(join(repo, "Taskfile.yml"), "utf8").replace(/\r\n/g, "\n");
    const steps = [...taskfile.matchAll(/^[ \t]*(\w+)="\$\(node scripts\/dev-home\.mjs[^\n]*\|\| exit 1\n[ \t]*eval "\$\1"$/gm)].map((m) => m[0]);
    const run = (step, env) =>
        spawnSync("bash", ["-c", `${step}\necho "reached override=$AGENTMUX_HOME_OVERRIDE"`], {
            cwd: repo,
            encoding: "utf8",
            env: { PATH: process.env.PATH, HOME: tmp, USERPROFILE: tmp, AGENTMUX_VITE_PORT: "5173", ...env },
        });

    it("is in both dev tasks, and never evals the command substitution directly", () => {
        expect(steps).toHaveLength(2);
        expect(taskfile).not.toMatch(/eval "\$\(node scripts\/dev-home\.mjs/);
    });

    it("looks for a crashed host's stale Chromium lock in the selected home, not only the real one", () => {
        expect(taskfile).toContain('"${AGENTMUX_HOME_OVERRIDE:-$HOME/.agentmux}/dev/"*/*/cef-cache/SingletonLock');
        expect(taskfile).not.toContain('"$HOME/.agentmux/dev/"*/*/cef-cache');
    });

    it("stops when the home is refused, instead of running on the real one", () => {
        mkdirSync(join(tmp, ".agentmux"));
        for (const step of steps) {
            const r = run(step, { AGENTMUX_HOME_OVERRIDE: join(tmp, ".agentmux") });
            expect(r.status).toBe(1);
            expect(r.stdout).not.toContain("reached");
            expect(r.stderr).toMatch(/own ~\/\.agentmux/);
        }
    });

    it("carries on, on a home of its own, when nothing is refused", () => {
        for (const step of steps) {
            const r = run(step, {});
            expect(r.status).toBe(0);
            expect(r.stdout).toMatch(/reached override=.*\.dev-home\/5173/);
        }
    });
});
