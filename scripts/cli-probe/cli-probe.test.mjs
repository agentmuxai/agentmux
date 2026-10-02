// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The observed behaviour of the pinned Claude CLI that the Runtime menu relies
 * on (docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §7.3).
 *
 * Opt-in: it spawns the real CLI (no account, no keychain, see probe.mjs), so it
 * only runs with AGENTMUX_CLI_PROBE=1 and finds the CLI under
 * ~/.agentmux/shared/cli/claude (or AGENTMUX_CLI_PROBE_BIN). When a pin moves,
 * run it: a failure here means an assumption in the menu or in srv is stale.
 *
 *   AGENTMUX_CLI_PROBE=1 npx vitest run scripts/cli-probe/cli-probe.test.mjs
 */

import { existsSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runProbe } from "./probe.mjs";

/** The newest installed claude binary, or the one named by AGENTMUX_CLI_PROBE_BIN. */
function findCli() {
    if (process.env.AGENTMUX_CLI_PROBE_BIN) return process.env.AGENTMUX_CLI_PROBE_BIN;
    const root = join(homedir(), ".agentmux", "shared", "cli", "claude");
    if (!existsSync(root)) return null;
    const versions = readdirSync(root)
        .filter((v) => /^\d+\.\d+\.\d+$/.test(v))
        .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))
        .reverse();
    for (const v of versions) {
        const scoped = join(root, v, "node_modules", "@anthropic-ai");
        if (!existsSync(scoped)) continue;
        for (const pkg of readdirSync(scoped)) {
            const bin = join(scoped, pkg, process.platform === "win32" ? "claude.exe" : "claude");
            if (existsSync(bin)) return bin;
        }
    }
    return null;
}

const cli = process.env.AGENTMUX_CLI_PROBE === "1" && process.platform !== "win32" ? findCli() : null;
const run = (o) => runProbe({ cli, ...o });
const apiModels = (r) => r.requests.filter((q) => q.path === "/v1/messages").map((q) => q.model);
const settings = (r) => r.events.find((e) => e.type === "control_response" && e.response?.response?.applied)?.response.response.applied;

describe.skipIf(!cli)("pinned Claude CLI", () => {
    it("never touches the real keychain or an account", async () => {
        const r = await run({ args: ["--model", "haiku"] });
        expect(r.securityCalls.every((c) => c.startsWith("find-generic-password"))).toBe(true);
        expect(r.timedOut).toBe(false);
    }, 90000);

    it("resolves aliases, and the LAST of two --model flags wins", async () => {
        expect(apiModels(await run({ args: ["--model", "sonnet"] }))[0]).toBe("claude-sonnet-5-5");
        expect(apiModels(await run({ args: ["--model", "opus", "--model", "haiku"] }))[0]).toBe("claude-haiku-4-5-20251001");
        expect(apiModels(await run({ args: ["--model", "haiku", "--model", "opus"] }))[0]).toBe("claude-opus-5-5");
    }, 180000);

    it("with no --model runs Opus (the incident the menu's seeded launch args prevent)", async () => {
        expect(apiModels(await run({}))[0]).toBe("claude-opus-5-5");
    }, 90000);

    it("sends --effort on Sonnet and drops it on Haiku", async () => {
        const sonnet = await run({ args: ["--model", "sonnet", "--effort", "high"] });
        expect(sonnet.requests.find((q) => q.path === "/v1/messages").body.output_config?.effort).toBe("high");
        const haiku = await run({ args: ["--model", "haiku", "--effort", "high"] });
        expect(haiku.requests.find((q) => q.path === "/v1/messages").body.output_config).toBeUndefined();
    }, 180000);

    it("answers get_settings straight after spawn, with the resolved model", async () => {
        const r = await run({ args: ["--model", "sonnet", "--effort", "high"], messages: [], controlRequests: [{ subtype: "get_settings" }] });
        expect(settings(r)).toMatchObject({ model: "claude-sonnet-5-5", effort: "high" });
        const h = await run({ args: ["--model", "haiku"], messages: [], controlRequests: [{ subtype: "get_settings" }] });
        expect(settings(h)?.model).toBe("claude-haiku-4-5-20251001");
        expect(settings(h)?.effort ?? null).toBeNull();
    }, 90000);

    it("set_model takes effect on the running process", async () => {
        const r = await run({
            args: ["--model", "sonnet"],
            messages: [],
            controlRequests: [{ subtype: "set_model", model: "haiku" }, { subtype: "get_settings" }],
        });
        expect(settings(r)?.model).toBe("claude-haiku-4-5-20251001");
    }, 90000);

    describe("permission routing with a permission prompt tool (a persistent agent)", () => {
        // A file INSIDE the working directory: outside it, even acceptEdits asks.
        const writeCall = (n, _body, { cwd }) =>
            n === 0 ? { toolUse: { id: "toolu_probe_w", name: "Write", input: { file_path: join(cwd, "probe-write-target.txt"), content: "x" } } } : { text: "done" };
        const ask = (mode) =>
            run({
                args: ["--permission-prompt-tool", "stdio", "--permission-mode", mode],
                messages: ["write a file"],
                script: writeCall,
                // A prompted Write is denied. acceptEdits never prompts for one inside the
                // working directory, so there it runs - in the throwaway home, deleted after.
                decide: () => "deny",
            });

        it("default asks before a write", async () => {
            expect((await ask("default")).permissionRequests.map((p) => p.tool)).toContain("Write");
        }, 90000);

        it("plan ASKS before a write rather than refusing it, so allow-all lets it through", async () => {
            expect((await ask("plan")).permissionRequests.map((p) => p.tool)).toContain("Write");
        }, 90000);

        it("acceptEdits does not ask before a write", async () => {
            expect((await ask("acceptEdits")).permissionRequests.map((p) => p.tool)).not.toContain("Write");
        }, 90000);
    });
});
