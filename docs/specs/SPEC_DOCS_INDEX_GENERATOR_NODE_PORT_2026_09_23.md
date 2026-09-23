# SPEC: Port the specs-index generator to Node

**Date:** 2026-09-23
**Status:** implemented — #3590 (port, wrapper, golden-fixture tests, the `specs index (<os>)` CI job, docs corrections). §9 follow-ups not started.
**Author:** Manoz
**Related:**
`docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` (Phase 3 introduced the generated index),
`docs/specs/PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01.md` (Batch D, #2914),
`docs/archive/README.md` (records the index as "not reproducible across platforms").
Prior fixes to the same script: #2920 (`LC_ALL=C`), #3056 (merge-safe index: no section counts),
#3059/#3069 (scope `--check` to branches that touch specs), #3073 (tracked files, not a glob).

---

## 1. Summary

`scripts/gen-docs-index.sh` regenerates the generated half of
`docs/specs/INDEX.md`. It is correct on CI's Linux runner but:

- **slow on Windows**: ~200 s per run for 956 specs on a Windows 10 dev
  machine, because it starts about ten processes per spec and Git Bash
  emulates `fork()`;
- **not portable**: it needs bash ≥ 4, which stock macOS doesn't ship, and
  its output depends on platform-specific tool behaviour;
- **silently incomplete** for any spec whose filename isn't plain ASCII.

This spec ports it to a single Node script, `scripts/gen-docs-index.mjs`,
with byte-identical output on today's tree, and adds a CI check that runs it
on Windows, macOS and Linux. `gen-docs-index.sh` becomes a one-line wrapper
so every existing invocation keeps working.

## 2. Problems, with evidence

### 2.1 Speed (measured 2026-09-23)

| Implementation | Time, 956 specs, Windows 10 + Git Bash |
|---|---|
| Current script | ~200 s per run (398 s for generate + `--check`) |
| One `awk` pass over all files (prototype) | 0.35 s |
| One Node process (prototype) | 0.21 s |

Both prototypes reproduced every row and every status group of the committed
`INDEX.md` exactly (0 differing lines out of 956).

Cause: per spec, the loop runs `basename`, `head`, `grep`, `sed`, `awk`,
`sed`, `grep`, `sed`, `tr` and several command substitutions — roughly 9,500
process starts per run. Windows has no copy-on-write `fork()`, so Cygwin/MSYS2
emulate it, and each start costs ~20 ms instead of < 1 ms. The cost also grows
with machine uptime (one report: 0.06 s per spawn after reboot, > 1 s after
hours).

### 2.2 Portability

- **bash 4 required.** `build()` uses `declare -A` (associative arrays,
  bash 4.0+). Stock macOS ships bash 3.2 and the script fails there unless a
  newer bash comes first on `PATH`.
- **Line endings differ by platform.** The GNU grep manual: "On MS-Windows
  when `grep` uses text I/O it reads a carriage return–newline pair as a
  newline"; elsewhere it passes bytes through. A spec with CRLF line endings
  therefore gets a title ending in `\r` on Linux and macOS and a clean title
  on Windows (the Windows half observed 2026-09-23). The repo's
  `.gitattributes` (`* text=auto eol=lf`) normally keeps CRLF out of the
  working tree, so this is latent, not active.
- **Nothing keeps the platforms in agreement.** CI runs the check only on
  `ubuntu-latest`. `docs/archive/README.md` and the lifecycle spec (both
  2026-09-18, #3366/#3367) record a Windows run producing ~21 extra status
  buckets and a different row order than CI. That was after `LC_ALL=C` landed
  (#2920, 2026-09-01), so the locale pin was not the cure. A run on this
  Windows machine on 2026-09-23 matched CI exactly; the cause of the earlier
  divergence was not identified.
- (Checked and *not* a problem: `sed 's/…/I'` works on macOS since Big Sur.)

### 2.3 Silent incompleteness

`git ls-files` quotes paths containing non-ASCII bytes
(`"docs/specs/Caf\303\251.md"`, with `core.quotePath` at its default). The
script's `grep -E '^docs/specs/[^/]+\.md$'` then rejects the quoted line, so
the spec gets no row. The completeness assertion cannot catch it because both
of its totals come from the same filtered list. Reproduced 2026-09-23: a
fixture with three specs, one named `Café.md`, produced
"2 specs under docs/specs/". No spec in the tree has such a name today.

## 3. Goals and non-goals

**Goals**

1. Byte-identical `INDEX.md` to the current script on today's tree.
2. Identical output on Windows, macOS and Linux, enforced by CI.
3. Under one second on the slowest dev machine.
4. Every existing behaviour preserved (§5), including every documented
   failure mode the current script guards against.
5. The three defects in §2 fixed.

**Non-goals**

- **Changing how a Status word is parsed.** Four scripts parse Status lines
  today (this one, `check-doc-status.sh`, `docs-stale-sweep.mjs`,
  `check-new-spec-status.mjs`) and they don't all agree: for
  `**Status:** active—Phase 0`, this script and `docs-stale-sweep.mjs` yield
  `activephase`, while `check-new-spec-status.mjs` yields `active`. Unifying
  them changes which bucket some specs land in, so it is a semantic change
  with its own review. Doing it here would also make byte-parity (goal 1)
  unverifiable. Follow-up, §9.
- Changing `INDEX.md`'s format, markers or wording.

## 4. Design

- `scripts/gen-docs-index.mjs`: a single Node ES module using only Node
  built-ins (`node:fs`, `node:path`, `node:child_process`, `node:os`). Runs on
  the Node already present on every CI runner and dev machine; no `npm ci`
  needed.
- **Reads bytes, not text.** Files are read as `latin1`, so each byte maps to
  one character and regexes behave as they do under `LC_ALL=C`. Output is
  written back as `latin1`, so UTF-8 in titles round-trips byte-for-byte.
- **Lowercasing is ASCII-only**, matching `awk tolower` under `LC_ALL=C`.
- **Sorting is by byte value** everywhere, never by locale.
- **git is called twice at most** (`rev-parse`/`ls-files`, and `diff` for the
  `--check` scope), with `-z` so paths arrive raw and unquoted (fixes §2.3).
- `scripts/gen-docs-index.sh` becomes a wrapper:
  `exec node "<dir>/gen-docs-index.mjs" "$@"`, with a clear error if `node`
  isn't on `PATH`. The wrapper uses only shell builtins (no `dirname`
  process).
- The rationale comments from the shell script move into the Node script with
  the code they explain; they record hard-won failure modes.
- The generated text still says "Generated by `scripts/gen-docs-index.sh`",
  and the BEGIN marker still names it. The wrapper is the documented entry
  point, and keeping the text unchanged keeps goal 1 and avoids needless
  conflicts with open PRs that touch `INDEX.md`.

## 5. Behaviour contract (preserved exactly)

| Area | Behaviour |
|---|---|
| Modes | no argument: rewrite; `--check`: assert, scoped; `--check-all`: assert on the whole tree. Only the first argument is read; anything else means "rewrite". |
| `--check` scope | asserts only if `git diff --name-status --find-renames <base>...HEAD` touches `docs/specs/**.md` or the generator (either side of a rename). Base is `origin/$GITHUB_BASE_REF`, else `$GITHUB_BASE_REF`, default `main`; an unresolvable base skips with a message and exit 0. |
| Spec files | `git ls-files -- docs/specs`, depth 1, `.md`, deduplicated keeping first occurrence (conflict stages), filtered to existing regular files; empty result or not a repo falls back to the directory listing, byte-sorted. `INDEX.md` and `README.md` are skipped. |
| Per file | first 40 lines only. Status: first line matching `^**Status:**` case-insensitively; strip that prefix and leading whitespace; first blank-separated word; ASCII-lowercase; delete every byte outside `a–z`; empty → `__none__`. Title: first line starting `# `; strip `#` and following spaces; delete `\|`; empty → the filename. An unreadable file still gets a row (`__none__`, filename). |
| Output | the same preamble; canonical buckets in the order implemented, active, proposed, draft, living, historical, superseded; then "no status line"; then "non-canonical status", grouped by word, byte-sorted. Rows keep file order. |
| Completeness | "N specs under docs/specs/ (excluding archive/)" to stderr on every run; if emitted rows ≠ N, "FATAL…" and exit 3, leaving `INDEX.md` untouched. |
| Splice | everything before the first line containing the BEGIN marker is kept verbatim; with no marker, the whole file plus `\n---\n\n`. |
| Write | to a temporary file in `docs/specs/`, then rename over `INDEX.md` (atomic on every OS). |
| `--check` result | exit 0 "INDEX.md is current." or exit 1 "INDEX.md is STALE." plus the first 40 lines of a line diff. |
| Missing index | "gen-docs-index: docs/specs/INDEX.md not found", exit 1. |
| Working directory | run from the repo root, as today. |

## 6. Deliberate behaviour changes

1. **Non-ASCII filenames get rows** (§2.3). Output for today's tree is
   unchanged; no spec is affected.
2. **A trailing `\r` is stripped from the Status line and the title** on every
   platform. This matches what Windows already produces and what an LF
   checkout produces, so one committed `INDEX.md` is correct everywhere.
3. **`scripts/gen-docs-index.mjs` and `.gitattributes` are added to the
   `--check` scope**, alongside the shell wrapper, so any change to the
   generator re-asserts the index. `.gitattributes` decides the bytes a
   checkout hands the generator (an `eol=crlf` rule on `docs/specs` would make
   the committed index fail on a fresh checkout), so an attributes-only change
   must re-assert it too (Codex review, #3590). The CI classifier's
   `docs_index` trigger includes it for the same reason.
4. **The STALE diff excerpt** is produced by the script itself (an LCS line
   diff in `diff`'s normal `<`/`>` format) instead of by calling `diff(1)`, which
   isn't guaranteed on Windows outside Git Bash. The pass/fail result is
   unchanged; only the text of the excerpt may differ.

## 7. Verification

1. **Real tree, byte-for-byte:** the current script and the port each
   regenerate `INDEX.md` from the same checkout; the files must be identical.
   `--check-all` must pass with the port against the `INDEX.md` the old script
   committed.
2. **Golden fixtures** (`scripts/gen-docs-index.test.mjs`, vitest): each
   builds a throwaway git repo and runs the port. Expected outputs were
   produced by the current shell script, except where §6 intentionally
   differs, and those cases say so. Cases:
   - every canonical status, no Status line, non-canonical words, mixed case,
     punctuation after the word (`active—Phase`, `Draft,`), Status beyond
     line 40, H1 beyond line 40, missing H1, H1 of only `#`, `|` in titles,
     UTF-8 titles;
   - `INDEX.md` and `README.md` skipped; `archive/` excluded; untracked specs
     excluded; tracked-but-deleted specs excluded; a staged (`git add`ed) spec
     included;
   - no BEGIN marker (append `---`); marker present (splice);
   - `--check` current (exit 0), stale (exit 1 plus diff), not in scope (exit 0),
     unresolvable base (exit 0), rename out of `docs/specs/` (in scope);
     `--check-all`;
   - not a git repo → directory fallback; a git repo that tracks none of these
     files → fallback;
   - the completeness assertion (forced through an injected mismatch) → exit 3,
     `INDEX.md` untouched;
   - a merge conflict leaving three index stages of one spec → one row;
   - §6 changes: CRLF spec, non-ASCII filename.
3. **Pure-function unit tests** for Status and title extraction, byte sort and
   ASCII lowercase, against the current script's rules.
4. **Cross-platform CI:** a new `ci-pr.yml` job, "specs index (os)", on
   `ubuntu-latest`, `windows-latest` and `macos-latest`. It runs the fixture
   tests and `node scripts/gen-docs-index.mjs --check`, and is added to the
   "CI required" aggregator. Because this PR changes the generator, `--check`
   is in scope and compares the port's output on all three OSes with the
   `INDEX.md` the old script produced.
5. **Timing** recorded on the Windows dev machine before and after.

## 8. Rollout

One PR: the port, the wrapper, tests, the CI job, the docs job switched to
call Node directly, and corrections to the two docs that say the index isn't
reproducible across platforms. Reverting the PR restores the shell script
exactly.

## 9. Follow-ups

- **One Status parser for all four doc gates** (§3 non-goals), as its own
  spec: choose the canonical rule, list the specs whose bucket or verdict
  changes, and land it with a regenerated index.
- Other doc gates still written in bash and run only on Linux
  (`check-doc-status.sh`, `check-spec-citations.sh`, and others) have the same
  per-file process cost on Windows. Measure them and port any that are slow or
  non-portable.

## 10. Sources

- Rufflewind, "Bash on Windows is really slow" — https://rufflewind.com/2014-08-23/windows-bash-slow
- claude-code #80670, Git Bash spawn cost grows with uptime — https://github.com/anthropics/claude-code/issues/80670
- MSYS2-packages #138, "All tools from shell are very slow" — https://github.com/msys2/MSYS2-packages/issues/138
- nitefood/asn #108, macOS bash 3.2 and `declare -A` — https://github.com/nitefood/asn/issues/108
- nixCraft, sed case-insensitive matching (macOS `I` flag since Big Sur) — https://www.cyberciti.biz/faq/unixlinux-sed-case-insensitive-search-replace-matching/
- GNU grep manual, `-U`/`--binary` (CR stripping on MS-Windows) — https://www.gnu.org/software/grep/manual/grep.html
