// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { banner, chooseHome, shellLines, USE_REAL_HOME } from "./dev-home.mjs";
import { pickHome } from "./dev-seed.mjs";

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
