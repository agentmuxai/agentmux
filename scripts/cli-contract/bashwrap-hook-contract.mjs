#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The contract between AgentMux's Bash hook (`agentmux-bashwrap hook`) and the
// real Claude Code CLI: run the CLI with only the hook configured, against a
// fake Messages API (fake-anthropic.mjs: no model, no credential), make one
// scripted Bash call per case, and check what the CLI actually did with the
// hook's rewrite. The hook's own unit tests check what it sends; only this
// catches the CLI changing what it does with it, as it did once silently.
// docs/retro/RETRO_BASHWRAP_HOOK_DROPS_BASH_TOOL_FIELDS_2026_10_10.md.
//
// Usage: node scripts/cli-contract/bashwrap-hook-contract.mjs \
//            --claude <path to the claude CLI> --bashwrap <path to agentmux-bashwrap>
// Exit code 0 when every case holds.

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startFakeAnthropic } from "./fake-anthropic.mjs";

const CASE_TIMEOUT_MS = 90_000;

function arg(name) {
  const i = process.argv.indexOf(name);
  return i > 0 ? process.argv[i + 1] : undefined;
}

/**
 * Run the CLI once with the hook and `settings`, scripting `toolInput` as the
 * model's one Bash call. Returns the stream-json events and the fake API's
 * request log.
 */
async function runCli({ claude, bashwrap, toolInput, settings = {}, hook = true, extraEnv = {} }) {
  const api = await startFakeAnthropic(toolInput);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bashwrap-contract-"));
  const configDir = path.join(dir, "config");
  fs.mkdirSync(configDir);
  const hookCommand = `"${bashwrap.replace(/\\/g, "/")}" hook`;
  const allSettings = hook
    ? { ...settings, hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: hookCommand }] }] } }
    : settings;
  // Only what the CLI and the wrapper need: none of an AgentMux pane's
  // variables, and the wrapper the hook names first on PATH.
  const env = {};
  for (const key of [
    "PATH",
    "Path",
    "SystemRoot",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ComSpec",
    "PATHEXT",
  ]) {
    if (process.env[key] !== undefined) env[key] = process.env[key];
  }
  const pathKey = env.Path !== undefined && env.PATH === undefined ? "Path" : "PATH";
  env[pathKey] = `${path.dirname(bashwrap)}${path.delimiter}${env[pathKey] ?? ""}`;
  Object.assign(env, {
    ANTHROPIC_BASE_URL: api.url,
    ANTHROPIC_API_KEY: "contract-test-not-a-key",
    CLAUDE_CONFIG_DIR: configDir,
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_AUTOUPDATER: "1",
    ...extraEnv,
  });
  const args = [
    "-p",
    "Run the command.",
    "--output-format",
    "stream-json",
    "--verbose",
    "--settings",
    JSON.stringify(allSettings),
  ];
  const child = spawn(claude, args, { cwd: dir, env, stdio: ["ignore", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (d) => (stdout += d));
  child.stderr.on("data", (d) => (stderr += d));
  const killer = setTimeout(() => child.kill(), CASE_TIMEOUT_MS);
  const code = await new Promise((resolve) => child.on("close", resolve));
  clearTimeout(killer);
  await api.close();
  try {
    // On Windows a process the CLI started may still hold the directory.
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
  } catch {
    // A temp directory left behind doesn't change the result.
  }
  const events = stdout
    .split(/\r?\n/)
    .filter(Boolean)
    .map((l) => {
      try {
        return JSON.parse(l);
      } catch {
        return { unparsed: l };
      }
    });
  return { code, events, stderr, requests: api.requests };
}

/** The tool result for the scripted call, as text. */
function toolResult(events) {
  for (const e of events) {
    const content = e.type === "user" && Array.isArray(e.message?.content) ? e.message.content : [];
    for (const b of content) {
      if (b.type === "tool_result" && b.tool_use_id === "toolu_contract_1") {
        return typeof b.content === "string" ? b.content : JSON.stringify(b.content);
      }
    }
  }
  return null;
}

const systemEvent = (events, subtype) => events.find((e) => e.type === "system" && e.subtype === subtype);

const CASES = [
  {
    name: "run_in_background reaches the CLI: the call goes to the background at once, labelled with its description",
    toolInput: {
      command: "echo contract-background",
      description: "Contract test background call",
      run_in_background: true,
    },
    check({ events }) {
      const result = toolResult(events) ?? "";
      const started = systemEvent(events, "task_started");
      const done = systemEvent(events, "task_notification");
      const problems = [];
      if (!/running in background/i.test(result))
        problems.push(`the call didn't go to the background: tool result ${JSON.stringify(result.slice(0, 200))}`);
      if (started?.description !== "Contract test background call")
        problems.push(`task label ${JSON.stringify(started?.description)}, not the description`);
      if (done && /agentmux-bashwrap/.test(done.summary ?? ""))
        problems.push(`task summary quotes the wrapper: ${JSON.stringify(done.summary)}`);
      return problems;
    },
  },
  {
    name: "timeout reaches the CLI: a 1 s timeout stops a 5 s command",
    toolInput: { command: "sleep 5; echo contract-finished", description: "Contract test timeout", timeout: 1000 },
    check({ events }) {
      const result = toolResult(events) ?? "";
      return /contract-finished/.test(result)
        ? [`the command ran to the end, so the 1 s timeout was dropped: ${JSON.stringify(result.slice(0, 200))}`]
        : [];
    },
  },
  {
    name: "the command still runs through AgentMux's wrapper",
    toolInput: { command: "echo contract-wrapped", description: "Contract test wrapper" },
    check({ events }) {
      const result = toolResult(events) ?? "";
      // The wrapper heads its output with how the command exited (and,
      // outside AgentMux, a line saying it can't stream).
      return /^<exited 0 in [^>]*>[\s\S]*contract-wrapped/.test(result)
        ? []
        : [`the output didn't come through agentmux-bashwrap: ${JSON.stringify(result.slice(0, 200))}`];
    },
  },
];

async function main() {
  const claude = arg("--claude");
  const bashwrap = arg("--bashwrap");
  if (!claude || !bashwrap) {
    console.error("usage: bashwrap-hook-contract.mjs --claude <claude CLI> --bashwrap <agentmux-bashwrap>");
    process.exit(2);
  }
  const only = arg("--case");
  let failed = 0;
  for (const c of CASES) {
    if (only && !c.name.includes(only)) continue;
    const run = await runCli({ claude, bashwrap, toolInput: c.toolInput, settings: c.settings });
    const problems =
      toolResult(run.events) === null
        ? ["the CLI never made the scripted call (see the requests and stderr below)"]
        : c.check(run);
    if (problems.length === 0) {
      console.log(`ok   ${c.name}`);
      continue;
    }
    failed++;
    console.log(`FAIL ${c.name}`);
    for (const p of problems) console.log(`     ${p}`);
    console.log(`     exit ${run.code}; API requests: ${JSON.stringify(run.requests)}`);
    if (run.stderr.trim()) console.log(`     stderr: ${run.stderr.trim().slice(0, 800)}`);
    if (process.env.CONTRACT_DEBUG) console.log(JSON.stringify(run.events, null, 1).slice(0, 6000));
  }
  process.exit(failed ? 1 : 0);
}

export { runCli, systemEvent, toolResult };

if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith("bashwrap-hook-contract.mjs"))
  await main();
