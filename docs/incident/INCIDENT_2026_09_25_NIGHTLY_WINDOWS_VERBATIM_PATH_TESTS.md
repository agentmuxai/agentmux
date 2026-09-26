# INCIDENT 2026-09-25 — nightly Windows tests fail on `\\?\` verbatim paths

**Status:** implemented. The failing tests were fixed in PR #3749 (Agent3,
merged 2026-09-25 14:46 UTC), and every nightly since has passed. PR #3857,
which carries this document, removes the remaining copies of the same test
pattern (§6). One process gap is still open (§7).

**Severity:** Low. The problem was in tests only; no production code was
wrong and nothing shipped with it. `main` failed its Windows tests for about
7¾ hours, and one nightly run was red.

Times are UTC.

---

## 1. Summary

#3721 added a shared test helper, `claude_layout::tests::repo_with_worktree`,
and two tests built on it. The helper returned `canonicalize()`d paths. On
Windows, `canonicalize` returns a *verbatim* path (`\\?\C:\…\repo`), a form the
Claude CLI's process never sees as its working directory. Production code
already handles this: `physical_dir` doesn't canonicalize on Windows, and
`canonical_worktree_root` strips the prefix. So the tests compared a
prefixed expected value against an unprefixed real one, and failed.

PR CI runs no tests on Windows (only `cargo check --tests`), and on Linux
`canonicalize` has no prefix. So the first Windows run of these tests was the
nightly after the merge.

## 2. Timeline

| Time (UTC) | Event |
|---|---|
| 09-25 03:23 | #3721 opened. PR CI: `check --tests + test (windows-latest)` passes in 3m00s, compile-only; `(ubuntu-latest)` runs the tests and passes. |
| 09-25 07:01 | #3721 merged (`577c5f12a`). |
| 09-25 11:37 | Nightly **artifacts** run succeeds. It builds installers and runs no tests, so it isn't affected. |
| 09-25 12:35 | Nightly **cross-platform build + test** (run `36135782806`, on `605cca922`) starts. |
| 09-25 13:13 | Its `cargo build + test (windows-latest)` job fails with 4808 passed, 2 failed. Ubuntu and macOS pass. |
| 09-25 14:29 | Agent3 opens #3749. |
| 09-25 14:46 | #3749 merged (`bb4c28c22`). |
| 09-26 12:04 | Nightly run `36240775341` (on `59c2f9917`): every job passes, `windows-latest` included. |

About 6 hours from merge to detection (the next nightly), then 1½ hours from
detection to fix.

## 3. The failures

```
claude_layout::tests::memory_is_keyed_by_the_main_checkout_for_subdirectories_and_worktrees
  assertion `left == right` failed: a linked worktree shares the main checkout's memory
    left: "C:\\Users\\runneradmin\\AppData\\Local\\Temp\\.tmp4CpwJ8\\repo"
   right: "\\\\?\\C:\\Users\\runneradmin\\AppData\\Local\\Temp\\.tmp4CpwJ8\\repo"

memory_reconcile::tests::a_repository_root_and_its_worktree_leave_their_shared_folder_alone
  assertion `left == right` failed: the CLI's folder for both
    left: "…\\projects\\C--Users-runneradmin-AppData-Local-Temp--tmp24smhx-repo\\memory"
   right: "…\\projects\\----C--Users-runneradmin-AppData-Local-Temp--tmp24smhx-repo\\memory"
```

## 4. Root cause

`repo_with_worktree` returned `(repo.canonicalize(), wt.canonicalize())`,
which on Windows both start with `\\?\`.

- **Test 1.** `memory_project_root(&repo)` passed, because on Windows
  `physical_dir` returns its input unchanged: prefix in, prefix out.
  `memory_project_root(&wt)` follows the worktree's `.git` file to the main
  checkout, and `canonical_worktree_root` strips the prefix on the way, as
  it should, since the CLI names the folder after `C:\…\repo`. So the result
  was the unprefixed path, compared against the prefixed `repo`.
- **Test 2.** `memory_dir_for_cwd` names the folder with the CLI's rule
  (every non-alphanumeric character becomes `-`). For the prefixed repo
  root, the four characters of `\\?\` became `----`. For the worktree it
  resolved to the unprefixed root: `C--…`.

Production behaviour was right both times. The tests' inputs were paths a
real agent never has.

## 5. Why it wasn't caught before merge

`ci-pr.yml` runs `cargo test` on `ubuntu-latest` only. The Windows leg is
`cargo check --tests`, which compiles the tests without running them. Windows
test execution was dropped from PR CI on purpose, because it made the
required job take 14–15 minutes; the nightly covers it instead (see the
comments at `ci-pr.yml`'s `cargo test (CEF-free crates)` step). The cost of
that trade-off is exactly this: a Windows-only test failure reaches `main`
and is found up to a day later.

Linux can't catch this class of bug by accident: `canonicalize` there
returns an ordinary absolute path.

## 6. Contributing factor, and what the follow-up PR changes

After #3749, the rule "a path as the CLI sees it" existed in two copies:
`canonical_worktree_root`'s inner `real()` in production, and #3749's
`cli_path()` in the test helper. They already differed: production keeps a
verbatim UNC path (`\\?\UNC\server\…`), while the test copy stripped it to
`UNC\server\…`. Four more test sites still used a raw
`base.path().canonicalize()`:

- `claude_layout.rs`: `outside_a_repository_memory_is_keyed_by_the_physical_directory`
  and `a_git_file_that_is_not_a_linked_worktree_keeps_its_own_root`
- `memory_reconcile/tests.rs`: `a_subdirectory_agent_shares_the_repository_roots_folder`
  and the test after it

They pass on Windows only because both sides of each comparison carry the
same prefix. They are checking a path form that doesn't happen in practice,
the same latent bug that #3721's tests hit.

The follow-up PR:

1. Adds one function, `claude_layout::real_path`: symlinks resolved, verbatim
   prefix stripped, UNC kept. Production's `canonical_worktree_root` and all
   test sites use it. `without_verbatim_prefix` is the single copy of the
   prefix rule.
2. Unit-tests the prefix rule as string handling, so it runs on
   `ubuntu-latest` in every PR's CI and doesn't wait for the Windows nightly.
3. Fixes a doc comment that said the prefixed folder name is `---C--repo`.
   It is `----C--repo`: `\\?\` is four characters.

## 7. Open items

- **Windows test execution on PR CI (operator decision).** The structural gap
  in §5 remains. Options, cheapest first:
  1. Accept up to 24h detection lag for Windows-only failures, as now.
  2. Run `cargo test` on `windows-latest` only when the `changes` classifier
     in `ci-pr.yml` sees path-handling modules (`claude_layout.rs`,
     `memory_*`, `native_memory_*`, `persistent/`), filtered to those
     modules' tests. This costs runner time on those PRs only.
  3. Run Windows tests on every PR again, bringing back the 14–15 minute
     required job.
- **Not recommended now:** a clippy `disallowed-methods` ban on
  `Path::canonicalize`. There are 51 call sites in `agentmux-srv`, mostly in
  editor and file-watcher code, where the verbatim form is harmless.
