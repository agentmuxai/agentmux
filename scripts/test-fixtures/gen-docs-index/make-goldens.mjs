// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Regenerate the *.golden files from the ORIGINAL shell generator, so the
// tests compare the Node port against the implementation it replaced.
//
//   node scripts/test-fixtures/gen-docs-index/make-goldens.mjs [<git-rev>]
//
// <git-rev> defaults to the last commit whose scripts/gen-docs-index.sh was
// the full shell implementation. Needs bash >= 4 on PATH (Git Bash on
// Windows; Homebrew bash on macOS). Run from the repository root.
//
// The goldens were produced on Windows (Git Bash). For these fixtures that
// is equivalent to Linux: none contains CR, and every path is ASCII — the two
// inputs where the shell version's output depended on the platform
// (docs/specs/SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md §2.2–2.3).

import { execFileSync } from "node:child_process";
import { copyFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { GOLDEN_CASES, fixtureEnv, makeDir, removeDir } from "./fixtures.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REV = process.argv[2] ?? "3258b0575f4a8c";

const shell = execFileSync("git", ["show", `${REV}:scripts/gen-docs-index.sh`]);
const scratch = makeDir();
const oldScript = join(scratch, "gen-docs-index.old.sh");
writeFileSync(oldScript, shell);

try {
    for (const [name, build] of Object.entries(GOLDEN_CASES)) {
        const root = makeDir();
        try {
            build(root);
            execFileSync("bash", [oldScript], { cwd: root, env: fixtureEnv(root), stdio: ["ignore", "pipe", "pipe"] });
            copyFileSync(join(root, "docs/specs/INDEX.md"), join(HERE, `${name}.golden`));
            console.log(`wrote ${name}.golden`);
        } finally {
            removeDir(root);
        }
    }
} finally {
    removeDir(scratch);
}
