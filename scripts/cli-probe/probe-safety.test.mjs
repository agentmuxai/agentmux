// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The probe's safety properties. These do NOT run the CLI: they pin the
 * guarantees that make running it safe - the keychain shim answers "not found"
 * and logs, it wins over the real `security` on PATH, and the environment is
 * built from scratch.
 */

import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { makeSecurityShim, probeEnv, SECURITY_ITEM_NOT_FOUND } from "./probe.mjs";

const posix = process.platform !== "win32";
const dirs = [];
afterEach(() => {
    while (dirs.length) rmSync(dirs.pop(), { recursive: true, force: true });
});
const tmp = () => {
    const d = mkdtempSync(join(tmpdir(), "amx-probe-test-"));
    dirs.push(d);
    return d;
};

describe.skipIf(!posix)("the security shim", () => {
    it("answers 'item not found' and logs what was asked, however it is called", () => {
        const shim = makeSecurityShim(join(tmp(), "shim"));
        const r = spawnSync("security", ["find-generic-password", "-a", "someone", "-w", "-s", "Claude Code-credentials"], {
            env: { PATH: shim.dir },
        });
        expect(r.status).toBe(SECURITY_ITEM_NOT_FOUND);
        expect(r.stdout.toString()).toBe(""); // never a secret
        expect(readFileSync(shim.log, "utf8")).toBe('find-generic-password -a someone -w -s Claude Code-credentials\n');
    });

    it("wins over a real `security` later on PATH, which is what keeps the real keychain untouched", () => {
        const shim = makeSecurityShim(join(tmp(), "shim"));
        const env = probeEnv({ baseUrl: "http://x", home: "/h", configDir: "/c", shimDir: shim.dir, path: "/usr/bin:/bin" });
        const found = execFileSync("sh", ["-c", "command -v security"], { env }).toString().trim();
        expect(found).toBe(join(shim.dir, "security"));
    });

    it("also catches the shell-string form the CLI uses (`security find-generic-password -a ... -w -s ...`)", () => {
        const shim = makeSecurityShim(join(tmp(), "shim"));
        const r = spawnSync("/bin/sh", ["-c", 'security find-generic-password -a "u" -w -s "svc"'], { env: { PATH: shim.dir } });
        expect(r.status).toBe(SECURITY_ITEM_NOT_FOUND);
        expect(readFileSync(shim.log, "utf8")).toContain('-s svc');
    });
});

describe("the probe environment", () => {
    const env = probeEnv({ baseUrl: "http://127.0.0.1:9", home: "/h", configDir: "/c", shimDir: "/shim", path: "/usr/bin" });

    it("is built from scratch: nothing is inherited from the caller's environment", () => {
        process.env.ANTHROPIC_MODEL = "must-not-leak";
        process.env.AGENTMUX_AGENT_ID = "must-not-leak";
        process.env.CLAUDE_CODE_OAUTH_TOKEN = "must-not-leak";
        try {
            const e = probeEnv({ baseUrl: "http://x", home: "/h", configDir: "/c", shimDir: "/s" });
            expect(Object.keys(e).sort()).toEqual(Object.keys(env).sort());
            expect(JSON.stringify(e)).not.toContain("must-not-leak");
        } finally {
            delete process.env.ANTHROPIC_MODEL;
            delete process.env.AGENTMUX_AGENT_ID;
            delete process.env.CLAUDE_CODE_OAUTH_TOKEN;
        }
    });

    it("points the CLI at the fake API with a dummy key and at throwaway state", () => {
        expect(env.ANTHROPIC_BASE_URL).toBe("http://127.0.0.1:9");
        expect(env.ANTHROPIC_API_KEY).toMatch(/probe-not-a-real-key/);
        expect(env.HOME).toBe("/h");
        expect(env.CLAUDE_CONFIG_DIR).toBe("/c");
        expect(env.PATH.startsWith("/shim:")).toBe(true);
    });
});
