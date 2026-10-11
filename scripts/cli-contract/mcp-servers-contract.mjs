#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The contract behind Connectors' list of what each agent runs
// (`crates/srv/src/backend/mcp_observed.rs`): the real Claude Code CLI's
// `system/init` frame lists every MCP server it loaded, with its status and
// source, and names each server's tools `mcp__<server>__<tool>`. Run the
// CLI against a fake Messages API (fake-anthropic.mjs: no model, no
// credential) with servers from three places and check what init says.
// srv's parser is tested against a copy of such a frame; only this catches
// the CLI changing it.
//
// Usage: node scripts/cli-contract/mcp-servers-contract.mjs --claude <path to the claude CLI>
// Exit code 0 when every check holds.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { startFakeAnthropic } from "./fake-anthropic.mjs";

const TIMEOUT_MS = 90_000;
const claude = process.argv[process.argv.indexOf("--claude") + 1];
if (!claude || process.argv.indexOf("--claude") < 0) {
  console.error("usage: mcp-servers-contract.mjs --claude <path>");
  process.exit(2);
}
const slash = (p) => p.split(path.sep).join("/");

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mcp-contract-"));
const work = path.join(dir, "work");
const configDir = path.join(dir, "config");
fs.mkdirSync(work);
fs.mkdirSync(configDir);

// A stdio MCP server with two tools.
const stub = path.join(dir, "stub.mjs");
fs.writeFileSync(
  stub,
  `import readline from "node:readline";
const send = (m) => process.stdout.write(JSON.stringify(m) + "\\n");
readline.createInterface({ input: process.stdin }).on("line", (line) => {
  const m = JSON.parse(line);
  if (m.method === "initialize")
    send({ jsonrpc: "2.0", id: m.id, result: { protocolVersion: m.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "stub", version: "1" } } });
  else if (m.method === "tools/list")
    send({ jsonrpc: "2.0", id: m.id, result: { tools: ["alpha", "beta"].map((name) => ({ name, description: name, inputSchema: { type: "object", properties: {} } })) } });
  else if (m.id !== undefined) send({ jsonrpc: "2.0", id: m.id, result: {} });
});
`,
);
const node = slash(process.execPath);
const server = { type: "stdio", command: node, args: [slash(stub)] };

// Where AgentMux writes servers (the workdir's .mcp.json), where an
// account's own servers live (CLAUDE_CONFIG_DIR/.claude.json), and the flag.
fs.writeFileSync(
  path.join(work, ".mcp.json"),
  JSON.stringify({
    mcpServers: {
      stub: server,
      "my server.v2": server,
      broken: { type: "stdio", command: node, args: ["-e", "process.exit(3)"] },
    },
  }),
);
fs.writeFileSync(path.join(configDir, ".claude.json"), JSON.stringify({ mcpServers: { userwide: server } }));
const flagConfig = path.join(dir, "flag.json");
fs.writeFileSync(flagConfig, JSON.stringify({ mcpServers: { viaflag: server } }));

const api = await startFakeAnthropic({ command: "echo contract", description: "contract" });
const env = {};
for (const key of ["PATH", "Path", "SystemRoot", "SYSTEMROOT", "TEMP", "TMP", "HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "ComSpec", "PATHEXT"]) {
  if (process.env[key] !== undefined) env[key] = process.env[key];
}
Object.assign(env, {
  ANTHROPIC_BASE_URL: api.url,
  ANTHROPIC_API_KEY: "contract-test-not-a-key",
  CLAUDE_CONFIG_DIR: configDir,
  CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
  DISABLE_AUTOUPDATER: "1",
});
// Headless, as AgentMux runs it: the project's .mcp.json loads without a prompt.
const args = ["-p", "hi", "--output-format", "stream-json", "--verbose", "--permission-mode", "bypassPermissions", "--mcp-config", flagConfig];
const child = spawn(claude, args, { cwd: work, env, stdio: ["ignore", "pipe", "pipe"] });
let stdout = "";
let stderr = "";
child.stdout.on("data", (d) => (stdout += d));
child.stderr.on("data", (d) => (stderr += d));
const killer = setTimeout(() => child.kill(), TIMEOUT_MS);
await new Promise((resolve) => child.on("close", resolve));
clearTimeout(killer);
await api.close();
try {
  fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
} catch {
  // A temp directory left behind doesn't change the result.
}

const init = stdout
  .split(/\r?\n/)
  .filter(Boolean)
  .map((l) => {
    try {
      return JSON.parse(l);
    } catch {
      return {};
    }
  })
  .find((e) => e.type === "system" && e.subtype === "init");

const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};
check(Boolean(init), "the CLI sends a system/init frame");
const servers = new Map((init?.mcp_servers ?? []).map((s) => [s.name, s]));
const tools = (init?.tools ?? []).filter((t) => t.startsWith("mcp__"));
const expect = (name, status, source) => {
  const s = servers.get(name);
  check(s?.status === status && s?.source === source, `${name}: ${status}, source ${source} (got ${JSON.stringify(s)})`);
};
expect("stub", "connected", "project");
expect("my server.v2", "connected", "project");
expect("broken", "failed", "project");
expect("userwide", "connected", "user");
expect("viaflag", "connected", "dynamic");
check(
  ["mcp__stub__alpha", "mcp__stub__beta", "mcp__my_server_v2__alpha", "mcp__my_server_v2__beta"].every((t) => tools.includes(t)),
  `tools are named mcp__<server>__<tool>, the name's other characters made _ (got ${JSON.stringify(tools)})`,
);
if (failures.length) {
  if (!init) console.error(stderr.slice(-2000));
  process.exit(1);
}
