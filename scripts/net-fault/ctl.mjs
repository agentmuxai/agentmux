#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Switches a running net-fault proxy's mode.
 *
 *   node scripts/net-fault/ctl.mjs refuse|blackhole|dns|pass|flap:30s/20s [--host <glob>]
 *   node scripts/net-fault/ctl.mjs status
 *   node scripts/net-fault/ctl.mjs clear          (drops every --host rule)
 *
 * Without `--host` it sets the default mode; with it, the rule for hosts matching
 * the glob. `--control <url>` (or `NET_FAULT_CONTROL`) names the control endpoint.
 */

import { parseArgs } from "node:util";
import { parseMode } from "./modes.mjs";
import { DEFAULT_CONTROL_PORT } from "./proxy.mjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    host: { type: "string" },
    control: {
      type: "string",
      default: process.env.NET_FAULT_CONTROL ?? `http://127.0.0.1:${DEFAULT_CONTROL_PORT}`,
    },
  },
});

const [command] = positionals;
if (!command) {
  console.error(
    "usage: ctl.mjs <pass|refuse|blackhole|dns|flap:<up>/<down>[:<downMode>]|status|clear> [--host <glob>]"
  );
  process.exit(2);
}

// A bad mode is a usage error, not a sign the proxy is down.
if (!["status", "clear"].includes(command)) {
  try {
    parseMode(command);
  } catch (err) {
    console.error(`ctl: ${err.message}`);
    process.exit(2);
  }
}
let res;
try {
  if (command === "status") res = await fetch(`${values.control}/status`);
  else if (command === "clear") res = await fetch(`${values.control}/clear`, { method: "POST" });
  else {
    res = await fetch(`${values.control}/mode`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ mode: command, host: values.host }),
    });
  }
} catch (err) {
  console.error(`ctl: ${err.cause?.code ?? err.message} (is the proxy running? control: ${values.control})`);
  process.exit(1);
}
process.stdout.write(await res.text());
process.exit(res.ok ? 0 : 1);
