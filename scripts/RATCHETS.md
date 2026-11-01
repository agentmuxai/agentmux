# Ratchet gates

A ratchet gate records today's count of something we want less of in a baseline file in the repo, fails a change that makes the count worse, and lets the baseline only go down. A gate never asks for the count to be zero before it can run, so it can be added to a codebase that already has the problem, and it stops the problem growing while people fix it as they touch the code.

The rules are the same for every gate:

- **Fixing something** makes the check fail until you lower the baseline, so the room you gave up can't be taken back. Run the gate's `--update` and commit the baseline file it wrote. `--update` only lowers numbers and drops fixed entries; it never adds an entry or raises a number.
- **Making it worse** fails the check. Fix the cause. If the growth can't be avoided, edit the baseline by hand and say why in the PR; a reviewer will read that line.
- A baseline file is plain text, one entry per line, sorted, so a change to it is a readable diff.

| Gate | What it holds | Baseline | Update |
|---|---|---|---|
| `scripts/check-file-sizes.mjs` | Source files over 1,500 lines may not grow; no other file may cross it | `scripts/check-file-sizes.baseline` | `node scripts/check-file-sizes.mjs --update` |
| `scripts/check-import-ratchets.mjs`, `backend-server` | srv's `backend/` may not name `crate::server` (production code; count per file) | `scripts/import-ratchets/backend-server.baseline` | `node scripts/check-import-ratchets.mjs --update` |
| `scripts/check-import-ratchets.mjs`, `frontend-cycles` | A frontend file may not join an import cycle (imports that survive compilation: not `import type`, not `import()`) | `scripts/import-ratchets/frontend-cycles.baseline` | `node scripts/check-import-ratchets.mjs --update` |
| `scripts/check-import-ratchets.mjs`, `frontend-inverted` | A lower frontend layer may not import a higher one (`store` and `element` into `view`, `util` into `app`; the rules are `LAYER_RULES` in the script) | `scripts/import-ratchets/frontend-inverted.baseline` | `node scripts/check-import-ratchets.mjs --update` |

`node scripts/check-import-ratchets.mjs --init` records today's state as a new baseline, adding entries. Use it only to create the baseline files; a failing check is not a reason to run it.

Each gate has a test next to it (`scripts/<name>.test.mjs`) that pins what it counts and how `--update` behaves. A gate runs in CI in the `doc status + grep gates` job, which also runs in the merge queue.
