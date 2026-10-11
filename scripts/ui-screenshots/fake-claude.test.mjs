// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The scripted stand-in for Claude Code (fake-claude/cli.mjs) and its
// installer. The stand-in runs as a real process here, as srv runs it.

import { spawn, spawnSync } from "node:child_process";
import { writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { PICKER_AGENTS, SHOT_AGENT, setup, shots as agentShots } from "./agent-shots.mjs";
import { homeProblem, pinnedClaudeVersion, posixShim, windowsShim } from "./install-fake-claude.mjs";
import conversation from "./fake-claude/conversation.json" with { type: "json" };

const CLI = join(__dirname, "fake-claude", "cli.mjs");

/** Runs the stand-in with `script`, sends `messages` one per line, and
 *  resolves with every line it wrote once `results` turns have ended. */
function run(script, messages, results = messages.length) {
    const dir = mkdtempSync(join(tmpdir(), "fake-claude-"));
    const path = join(dir, "conversation.json");
    writeFileSync(path, JSON.stringify({ msPerChunk: 0, ...script }));
    return new Promise((resolve, reject) => {
        const child = spawn(process.execPath, [CLI, "--input-format", "stream-json", "--output-format", "stream-json"], {
            env: { ...process.env, FAKE_CLAUDE_SCRIPT: path },
        });
        const lines = [];
        let buf = "";
        let done = 0;
        const timer = setTimeout(() => {
            child.kill();
            reject(new Error(`timed out; got ${lines.length} lines`));
        }, 10000);
        child.stdout.on("data", (d) => {
            buf += d;
            let i;
            while ((i = buf.indexOf("\n")) >= 0) {
                const obj = JSON.parse(buf.slice(0, i));
                buf = buf.slice(i + 1);
                lines.push(obj);
                if (obj.type === "result" && ++done === results) {
                    clearTimeout(timer);
                    child.stdin.end();
                    resolve(lines);
                }
            }
        });
        for (const m of messages) child.stdin.write(JSON.stringify({ type: "user", message: { role: "user", content: m } }) + "\n");
    });
}

describe("fake Claude Code", () => {
    it("reports the pinned version and a signed-in demo account", () => {
        const v = spawnSync(process.execPath, [CLI, "--version"], { encoding: "utf8" });
        expect(v.stdout.trim()).toBe(`${pinnedClaudeVersion()} (Claude Code)`);
        const a = spawnSync(process.execPath, [CLI, "auth", "status", "--json"], { encoding: "utf8" });
        expect(JSON.parse(a.stdout)).toMatchObject({ loggedIn: true, emailAddress: "dev@acme.example" });
    });

    it("plays the matching turn: init, streamed text, a tool call and its result, then the result line", async () => {
        const lines = await run(
            {
                turns: [
                    { match: "other", steps: [{ text: "not this one" }] },
                    {
                        match: "badge",
                        steps: [
                            { text: "Reading it." },
                            { tool: "Read", input: { file_path: "a.ts" }, result: "1\tx", runMs: 0, extra: { file: { startLine: 1, numLines: 1 } } },
                        ],
                        usage: { input_tokens: 5, output_tokens: 7 },
                    },
                ],
            },
            ["fix the badge"]
        );
        expect(lines[0]).toMatchObject({ type: "system", subtype: "init" });
        const text = lines
            .filter((l) => l.event?.delta?.type === "text_delta")
            .map((l) => l.event.delta.text)
            .join("");
        expect(text).toBe("Reading it.");
        const toolUse = lines.find((l) => l.type === "assistant" && l.message.content[0].type === "tool_use");
        expect(toolUse.message.content[0]).toMatchObject({ name: "Read", input: { file_path: "a.ts" } });
        const result = lines.find((l) => l.type === "user");
        expect(result.message.content[0]).toMatchObject({ type: "tool_result", tool_use_id: toolUse.message.content[0].id, content: "1\tx" });
        expect(result.tool_use_result).toEqual({ file: { startLine: 1, numLines: 1 } });
        expect(lines.at(-1)).toMatchObject({ type: "result", subtype: "success", usage: { input_tokens: 5, output_tokens: 7 } });
    });

    it("answers AgentMux's context message with an empty turn, without using up a scripted one", async () => {
        const lines = await run({ turns: [{ steps: [{ text: "hello" }] }] }, ["# Session Context\n...", "hi"]);
        const results = lines.filter((l) => l.type === "result");
        expect(results).toHaveLength(2);
        const firstTurn = lines.slice(0, lines.indexOf(results[0]));
        expect(firstTurn.some((l) => l.type === "assistant")).toBe(false);
        expect(lines.some((l) => l.event?.delta?.text === "hello")).toBe(true);
    });
});

describe("install-fake-claude", () => {
    it("reads Claude's pinned version from srv's provider table", () => {
        const src = 'npm_package: "@other/cli",\n pinned_version: "9.9.9",\n npm_package: "@anthropic-ai/claude-code",\n // note\n pinned_version: "2.1.288",';
        expect(pinnedClaudeVersion(src)).toBe("2.1.288");
        expect(() => pinnedClaudeVersion("nothing here")).toThrow(/pinned_version/);
    });

    it("writes a shim in the shape srv's .cmd parser accepts", () => {
        // crates/common/src/cli.rs: a line with %dp0%\ and %* naming a .mjs.
        const line = windowsShim()
            .split("\r\n")
            .find((l) => l.includes("%dp0%\\") && l.includes("%*"));
        expect(line).toMatch(/"%dp0%\\\.\.\\agentmux-fake-claude\\cli\.mjs" %\*$/);
        expect(posixShim()).toMatch(/^#!\/bin\/sh\nexec node .*cli\.mjs" "\$@"/);
    });

    it("refuses the machine's own data folder", () => {
        expect(homeProblem("/home/alice/.agentmux", "/home/alice/.agentmux")).toMatch(/own/);
        expect(homeProblem("/home/alice/.agentmux/", "/home/alice/.agentmux")).toMatch(/own/);
        expect(homeProblem("/tmp/capture-home", "/home/alice/.agentmux")).toBeNull();
    });
});

describe("agent suite", () => {
    it("has unique ids, a verify and a cleanup on every shot, and a setup", () => {
        const ids = agentShots.map((s) => s.id);
        expect(new Set(ids).size).toBe(ids.length);
        for (const s of agentShots) {
            expect(s.verify, s.id).toBeTruthy();
            expect(typeof s.cleanup, s.id).toBe("function");
        }
        expect(typeof setup).toBe("function");
        expect(PICKER_AGENTS).not.toContain(SHOT_AGENT);
    });

    it("scripts turns whose matches don't overlap", () => {
        // A message plays the first unplayed turn whose match it contains, so
        // no match may contain another.
        const matches = conversation.turns.map((t) => t.match?.toLowerCase()).filter(Boolean);
        for (const a of matches) for (const b of matches) if (a !== b) expect(a.includes(b), `${a} / ${b}`).toBe(false);
    });
});
