#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-workdir-writes.mjs — every write AgentMux makes into an agent's
// working directory goes through `backend::workdir_fs::Workdir`
// (docs/specs/SPEC_WORKDIR_SAFE_WRITES_2026_10_01.md). It refuses a path
// outside the workdir, a folder linking out of it, and a symlinked file, and
// replaces files atomically. Each write site used to guard itself, and LC3's
// review (#4131) found the gaps one at a time.
//
// Fails on a raw filesystem write in the launch-config writers, outside
// `#[cfg(test)]` modules. A line that must stay raw (creating the workdir
// itself) carries a same-line `// workdir-fs: <reason>` comment.
//
//   node scripts/check-workdir-writes.mjs

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// fileURLToPath, not `.pathname`: on Windows the latter is "/C:/…".
const ROOT = fileURLToPath(new URL("..", import.meta.url));

// `region`: only the lines from `start` (directly after a line matching
// `after`) up to the next `end` are checked
// (editor_handlers.rs also holds the editor pane's own saves, which write
// wherever the user points them, not into a workdir).
const FILES = [
    { path: "crates/srv/src/backend/agent_config.rs" },
    { path: "crates/srv/src/server/app_api/agent_open.rs" },
    {
        path: "crates/srv/src/server/editor_handlers.rs",
        // The handler's registration, not the `use` list naming the constant.
        region: { start: /^\s+COMMAND_WRITE_AGENT_CONFIG,\s*$/, after: /engine\.register_typed\(/, end: /engine\.register_typed\(/ },
    },
];

// Any `fs::` call that creates, changes or removes something, and the
// `OpenOptions`/`File::create` openers.
const RAW =
    /\b(?:fs::(?:write|rename|copy|create_dir\w*|remove_\w+|set_permissions|hard_link|symlink\w*)|File::create\w*|OpenOptions)\b/;
const ESCAPE = /\/\/\s*workdir-fs:\s*\S/;

const problems = [];
for (const { path, region } of FILES) {
    const lines = readFileSync(join(ROOT, path), "utf8").split("\n");
    let inRegion = !region;
    let testDepth = -1; // brace depth at which a #[cfg(test)] module started
    let depth = 0;
    let pendingTest = false;
    lines.forEach((line, i) => {
        const code = line.replace(/\/\/.*$/, "");
        if (region) {
            if (!inRegion && region.start.test(line) && i > 0 && region.after.test(lines[i - 1])) inRegion = true;
            else if (inRegion && region.end.test(line)) inRegion = false;
        }
        if (/#\[cfg\(test\)\]/.test(line)) pendingTest = true;
        const opens = (code.match(/\{/g) || []).length;
        const closes = (code.match(/\}/g) || []).length;
        if (pendingTest && opens > 0) {
            if (testDepth < 0) testDepth = depth;
            pendingTest = false;
        }
        const inTest = testDepth >= 0;
        depth += opens - closes;
        if (inTest && depth <= testDepth) testDepth = -1;
        if (inTest || !inRegion) return;
        if (RAW.test(code) && !ESCAPE.test(line)) {
            problems.push(`${path}:${i + 1}: ${line.trim()}`);
        }
    });
}

if (problems.length) {
    console.error("Raw filesystem writes into an agent's working directory:\n");
    for (const p of problems) console.error(`  ${p}`);
    console.error(
        "\nWrite through crate::backend::workdir_fs::Workdir instead (write / create_new / append /" +
            " remove_file / remove_dir / open_lock_file). See docs/specs/SPEC_WORKDIR_SAFE_WRITES_2026_10_01.md." +
            "\nIf a line truly must stay raw (it creates the workdir itself), add `// workdir-fs: <reason>` on it.",
    );
    process.exit(1);
}
console.log("check-workdir-writes: ok");
