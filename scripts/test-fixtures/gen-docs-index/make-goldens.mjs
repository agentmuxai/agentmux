// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Regenerate the *.golden files from the generator as it is now, over the
// fixture trees in fixtures.mjs.
//
//   node scripts/test-fixtures/gen-docs-index/make-goldens.mjs
//
// The goldens used to come from the ORIGINAL shell generator, so the Node port
// could be compared with what it replaced. They stopped matching it on
// 2026-11-14, when the index began grouping by area with a Status column
// (task #47664), so they are now this generator's own output. Run this only
// when the output is meant to change, and read the diff of the goldens before
// committing it: they are what the tests hold the output to.
//
// Run from the repository root.

import { copyFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { main } from "../../gen-docs-index.mjs";
import { GOLDEN_CASES, fixtureEnv, makeDir, removeDir } from "./fixtures.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

for (const [name, build] of Object.entries(GOLDEN_CASES)) {
    const root = makeDir();
    try {
        build(root);
        const code = main([], { root, env: fixtureEnv(root), out: () => {}, err: () => {} });
        if (code !== 0) throw new Error(`${name}: the generator exited ${code}`);
        copyFileSync(join(root, "docs/specs/INDEX.md"), join(HERE, `${name}.golden`));
        console.log(`wrote ${name}.golden`);
    } finally {
        removeDir(root);
    }
}
