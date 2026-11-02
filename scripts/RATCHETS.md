# Ratchet gates

A ratchet gate records today's count of something we want less of in a baseline file in the repo, fails a change that makes the count worse, and lets the baseline only go down. A gate never asks for the count to be zero before it can run, so it can be added to a codebase that already has the problem, and it stops the problem growing while people fix it as they touch the code.

The rules are the same for every gate:

- **Fixing something** makes the check fail until you lower the baseline, so the room you gave up can't be taken back. Run the gate's `--update` and commit the baseline file it wrote. `--update` only lowers numbers and drops fixed entries; it never adds an entry or raises a number.
- **Making it worse** fails the check. Fix the cause. If the growth can't be avoided, edit the baseline by hand and say why in the PR; a reviewer will read that line.
- A baseline file is plain text, one entry per line, sorted, so a change to it is a readable diff.

| Gate | What it holds | Baseline | Update |
|---|---|---|---|
| `scripts/check-file-sizes.mjs` | Source files over 1,500 lines may not grow; no other file may cross it | `scripts/check-file-sizes.baseline` | `node scripts/check-file-sizes.mjs --update` |
| `scripts/check-function-sizes.mjs` | Rust and TypeScript functions over 100 lines may not grow; no other function may cross it. Named by `<path>::<name>` (a method `<path>::<Type>::<name>`), not by line number; production Rust only | `scripts/function-sizes.baseline` | `node scripts/check-function-sizes.mjs --update` |
| `scripts/check-doc-references.mjs` | What the living docs say about the code is in the code: a backticked path exists, a name next to a path is in that file, no `file.rs:123` line citations, a file table lists its folder. One finding per line, `<doc>::<kind>::<text>` | `scripts/doc-references.baseline` | `node scripts/check-doc-references.mjs --update --baseline scripts/doc-references.baseline` |
| `scripts/check-import-ratchets.mjs`, `backend-server` | srv's `backend/` may not name `crate::server` (production code; count per file) | `scripts/import-ratchets/backend-server.baseline` | `node scripts/check-import-ratchets.mjs --update` |
| `scripts/check-import-ratchets.mjs`, `frontend-cycles` | A frontend file may not join an import cycle (imports that survive compilation: not `import type`, not `import()`) | `scripts/import-ratchets/frontend-cycles.baseline` | `node scripts/check-import-ratchets.mjs --update` |
| `scripts/check-import-ratchets.mjs`, `frontend-inverted` | A lower frontend layer may not import a higher one (`store` and `element` into `view`, `util` into `app`; the rules are `LAYER_RULES` in the script) | `scripts/import-ratchets/frontend-inverted.baseline` | `node scripts/check-import-ratchets.mjs --update` |

`--init` (on `check-import-ratchets.mjs` and `check-function-sizes.mjs`) records today's state as a new baseline, adding entries. Use it only to create the baseline files; a failing check is not a reason to run it.

`check-function-sizes.mjs` checks the files a change touches (since the merge base with origin/main), because main moves all day and a check of the whole tree against a baseline written the day before fails a change that did nothing; `--all` checks every file.

A listed function that is renamed, or moved to another file, shows up as one new function over the limit and one entry that is gone. `--update` drops the old line and never adds the new one, so give the new name the old line by hand in the PR and say so.

Each gate has a test next to it (`scripts/<name>.test.mjs`) that pins what it counts and how `--update` behaves. A gate runs in CI in the `doc status + grep gates` job, which also runs in the merge queue.

## The docs reference check outside this repo

`check-doc-references.mjs` reads Markdown in any folder against any checkout of the code, so a docs repo runs it against its own pages: `node <agentmux>/scripts/check-doc-references.mjs --docs src/content/docs --code <agentmux> --facts --any-symbol [--deny <file>] [--allow <file>]`. Without `--baseline` every finding fails.

- `--facts` also checks the counts, ranges and lists a page states that the code gives (the migrations range, the `HostCommand` variants, the MuxAPI tools, the host commands, the modal kinds, the pane env allow-list...). Each is a rule in `scripts/lib/doc-facts.mjs`; a rule is skipped when its page is not there.
- `--any-symbol` looks a backticked name up in the whole code when the line cites no file. This repo's own docs do not use it: they name Chromium's and CEF's identifiers too.
- `--deny <file>` holds `term => what to say instead`, one per line, for names the docs must not use any more; `--allow <file>` holds one term per line for names that are real but not in the code. Both live with the docs.
