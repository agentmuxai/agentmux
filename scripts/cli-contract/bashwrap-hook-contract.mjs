#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The contract between AgentMux's Bash streaming (`agentmux-bashwrap`) and the
// real Claude Code CLI: run the CLI as AgentMux runs it, against a fake
// Messages API (fake-anthropic.mjs: no model, no credential), make scripted
// Bash calls, and check what the CLI actually did. bashwrap's own unit tests
// check what it sends; only this catches the CLI changing what it does with
// it, as it did once silently
// (docs/retro/RETRO_BASHWRAP_HOOK_DROPS_BASH_TOOL_FIELDS_2026_10_10.md).
//
// Two modes, both run:
// - prefix: bashwrap is the CLI's CLAUDE_CODE_SHELL_PREFIX and the hook only
//   records calls (docs/specs/SPEC_BASH_STREAMING_VIA_SHELL_PREFIX_2026_10_10.md);
// - rewrite: the hook rewrites the command (the fallback without the prefix).
//
// Usage: node scripts/cli-contract/bashwrap-hook-contract.mjs \
//            --claude <path to the claude CLI> --bashwrap <path to agentmux-bashwrap> \
//            [--mode prefix|rewrite] [--case <part of a case name>]
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

const forwardSlashes = (p) => p.split(path.sep).join("/");

/**
 * Run the CLI once, scripting `toolInput` (one Bash call, or a list made in
 * order) as the model's calls. `mode` is how bashwrap is installed: "prefix",
 * "rewrite", "prefix-nohook" (the prefix set but no hook, as for a `claude`
 * run by hand inside an agent), or "none" (the CLI alone). Returns the stream-json events, the
 * CLI's stderr, the fake API's request log and the work directory's files.
 */
async function runCli({
  claude,
  bashwrap,
  toolInput,
  mode = "prefix",
  settings = {},
  extraEnv = {},
  extraArgs = [],
  workBase = os.tmpdir(),
  permissionMode = "bypassPermissions",
}) {
  const api = await startFakeAnthropic(toolInput);
  const dir = fs.mkdtempSync(path.join(workBase, "bashwrap-contract-"));
  const work = path.join(dir, "work");
  const configDir = path.join(dir, "config");
  fs.mkdirSync(work);
  fs.mkdirSync(configDir);
  const hookCommand = `"${forwardSlashes(bashwrap)}" hook`;
  const allSettings =
    mode === "none" || mode === "prefix-nohook"
      ? settings
      : {
          ...settings,
          hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: hookCommand }] }] },
        };
  // Only what the CLI and the wrapper need: none of an AgentMux pane's
  // variables, the wrapper the hook names first on PATH, and bashwrap's own
  // state kept in this run's directory.
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
    AGENTMUX_BASHWRAP_CALLS_DIR: path.join(dir, "calls"),
    AGENTMUX_BASHWRAP_CWD_STATE_FILE: path.join(dir, "cwd-state"),
    ...(mode.startsWith("prefix") ? { CLAUDE_CODE_SHELL_PREFIX: bashwrap } : {}),
    ...extraEnv,
  });
  // AgentMux approves ordinary tool calls itself (its permission channel in
  // the default mode, or --dangerously-skip-permissions one-shot,
  // providers.rs) while the user's deny rules still apply; bypassPermissions
  // is that, without a permission channel to speak.
  const args = [
    "-p",
    "Run the command.",
    "--output-format",
    "stream-json",
    "--verbose",
    "--permission-mode",
    permissionMode,
    "--settings",
    JSON.stringify(allSettings),
    ...extraArgs,
  ];
  const child = spawn(claude, args, { cwd: work, env, stdio: ["ignore", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (d) => (stdout += d));
  child.stderr.on("data", (d) => (stderr += d));
  const killer = setTimeout(() => child.kill(), CASE_TIMEOUT_MS);
  const code = await new Promise((resolve) => child.on("close", resolve));
  clearTimeout(killer);
  await api.close();
  const files = fs.readdirSync(work);
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
  return { code, events, stderr, requests: api.requests, files };
}

/** The tool result for a scripted call, as text. */
function toolResult(events, id = "toolu_contract_1") {
  for (const e of events) {
    const content = e.type === "user" && Array.isArray(e.message?.content) ? e.message.content : [];
    for (const b of content) {
      if (b.type === "tool_result" && b.tool_use_id === id) {
        return typeof b.content === "string" ? b.content : JSON.stringify(b.content);
      }
    }
  }
  return null;
}

const systemEvent = (events, subtype) => events.find((e) => e.type === "system" && e.subtype === subtype);

/** bashwrap heads the output with how the command exited; outside AgentMux,
 *  with a line saying it can't stream. */
const WRAPPED = /^<exited 0 in [^>]*>/;

const CASES = [
  {
    name: "run_in_background reaches the CLI: the call goes to the background at once, labelled with its description",
    modes: ["prefix", "rewrite"],
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
    modes: ["prefix", "rewrite"],
    toolInput: { command: "sleep 5; echo contract-finished", description: "Contract test timeout", timeout: 1000 },
    check({ events }) {
      const result = toolResult(events) ?? "";
      return /contract-finished/.test(result)
        ? [`the command ran to the end, so the 1 s timeout was dropped: ${JSON.stringify(result.slice(0, 200))}`]
        : [];
    },
  },
  {
    name: "the command runs through AgentMux's wrapper, linked to its call",
    modes: ["prefix", "rewrite"],
    toolInput: { command: "echo contract-wrapped", description: "Contract test wrapper" },
    check({ events }) {
      // In prefix mode the header only appears when the run claimed the
      // call's record; an unlinked script runs as plain bash.
      const result = toolResult(events) ?? "";
      return WRAPPED.test(result) && /contract-wrapped/.test(result)
        ? []
        : [`the output didn't come through agentmux-bashwrap: ${JSON.stringify(result.slice(0, 200))}`];
    },
  },
  {
    name: "a quoted, multi-line command arrives intact",
    modes: ["prefix", "rewrite"],
    toolInput: {
      command: 'printf \'%s\\n\' "it\'s" \'a "b"\'\nfor x in 1 2; do echo "n$x"; done',
      description: "Contract test quoting",
    },
    check({ events }) {
      const result = toolResult(events) ?? "";
      return /it's\s+a "b"\s+n1\s+n2/.test(result)
        ? []
        : [`the command's output is wrong: ${JSON.stringify(result.slice(0, 200))}`];
    },
  },
  {
    name: "a cd carries over to the next call",
    modes: ["prefix", "rewrite"],
    toolInput: [
      { command: "mkdir -p sub && cd sub", description: "Contract test cd" },
      { command: "pwd", description: "Contract test pwd" },
    ],
    check({ events }) {
      const result = toolResult(events, "toolu_contract_2") ?? "";
      return /\/sub\s*$/.test(result.trim())
        ? []
        : [`the second call didn't start in sub: ${JSON.stringify(result.slice(0, 200))}`];
    },
  },
  {
    name: "a trailing redirect still writes its file, and its output still shows",
    modes: ["prefix", "rewrite"],
    toolInput: { command: "echo contract-tee > teed.txt", description: "Contract test tee" },
    check({ events, files }) {
      const result = toolResult(events) ?? "";
      const problems = [];
      if (!files.includes("teed.txt")) problems.push("the redirect's file wasn't written");
      if (!/contract-tee/.test(result))
        problems.push(`the redirected output wasn't teed into the result: ${JSON.stringify(result.slice(0, 200))}`);
      return problems;
    },
  },
  {
    name: "a run with no call to link (no hook) is plain bash",
    modes: ["prefix-nohook"],
    toolInput: { command: "echo contract-plain", description: "Contract test unlinked" },
    check({ events }) {
      const result = toolResult(events) ?? "";
      return result.trim() === "contract-plain"
        ? []
        : [`expected just the command's output: ${JSON.stringify(result.slice(0, 200))}`];
    },
  },
  {
    name: "the user's deny rule applies to the model's command",
    modes: ["prefix"],
    settings: { permissions: { deny: ["Bash(touch:*)"] } },
    toolInput: { command: "touch contract-denied.txt && echo contract-ran", description: "Contract test deny" },
    check({ events, files }) {
      const result = toolResult(events) ?? "";
      const problems = [];
      if (!/denied/i.test(result)) problems.push(`the call wasn't denied: ${JSON.stringify(result.slice(0, 200))}`);
      if (files.includes("contract-denied.txt")) problems.push("the denied command ran");
      return problems;
    },
  },
];

async function main() {
  const claude = arg("--claude");
  const bashwrap = arg("--bashwrap");
  if (!claude || !bashwrap) {
    console.error(
      "usage: bashwrap-hook-contract.mjs --claude <claude CLI> --bashwrap <agentmux-bashwrap> [--mode prefix|rewrite] [--case <name part>]"
    );
    process.exit(2);
  }
  const onlyCase = arg("--case");
  const onlyMode = arg("--mode");
  let failed = 0;
  for (const c of CASES) {
    if (onlyCase && !c.name.includes(onlyCase)) continue;
    for (const mode of c.modes) {
      if (onlyMode && mode !== onlyMode) continue;
      const run = await runCli({ claude, bashwrap, mode, toolInput: c.toolInput, settings: c.settings });
      const first = Array.isArray(c.toolInput) ? "toolu_contract_1" : undefined;
      const problems =
        toolResult(run.events, first) === null
          ? ["the CLI never made the scripted call (see the requests and stderr below)"]
          : c.check(run);
      const label = `[${mode}] ${c.name}`;
      if (problems.length === 0) {
        console.log(`ok   ${label}`);
        continue;
      }
      failed++;
      console.log(`FAIL ${label}`);
      for (const p of problems) console.log(`     ${p}`);
      console.log(`     exit ${run.code}; API requests: ${JSON.stringify(run.requests)}`);
      if (run.stderr.trim()) console.log(`     stderr: ${run.stderr.trim().slice(0, 800)}`);
      if (process.env.CONTRACT_DEBUG) console.log(JSON.stringify(run.events, null, 1).slice(0, 6000));
    }
  }
  process.exit(failed ? 1 : 0);
}

export { runCli, systemEvent, toolResult };

if (process.argv[1]?.endsWith("bashwrap-hook-contract.mjs")) await main();
