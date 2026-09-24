# SPEC — The Windows CEF runtime guard rejects the correct runtime whenever its path contains a backslash

**Status:** implemented — PR #3639; fixes a regression in #3615 (`SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md`).
**Date:** 2026-09-24
**Author:** Agent1
**Severity:** High.
- Every local `task dev` / `task package` on Windows fails at `bundle:windows`,
  unless the developer knows the workaround.
- Every CI Windows build is expected to fail at the same step: nightly artifacts
  and releases (`build-windows.yml`). The first nightly after #3615 is expected
  to be 2026-09-24's; see §2.3.
- The guard it breaks exists to prevent a deadlocking runtime from shipping. The
  obvious workaround, `AGENTMUX_ALLOW_UNVERIFIED_CEF=1`, turns that protection
  off, so the bug pushes people toward exactly what #3615 was built to stop.

**Related:**
- #3615 / `docs/specs/SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md`: introduced the guard. Its §3.4 says CI "passes unchanged"; this spec corrects that.
- `docs/incident/INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md`: why the guard must stay on.
- `docs/retro/retro-nightly-windows-cef-zip-backslash-paths-2026-07-29.md`: an earlier backslash-path break in the same pipeline.

---

## 1. Problem

`scripts/cef-build/verify-cef-runtime-windows.sh:50`:

```bash
actual="$(sha256sum "$libcef" | cut -d' ' -f1)"
```

GNU coreutils escapes a file name that contains `\` or a newline in `*sum`
output. When it does, it **prefixes the whole output line with `\`**. `cut`
then returns `\<hash>`, which never equals `CEF_WINDOWS_LIBCEF_SHA256`, so the
guard refuses the runtime that *is* the pinned build.

This is coreutils behavior, not Windows behavior. Observed on 2026-09-24:

| Environment | `sha256sum "<path with \>"` | `sha256sum < "<same path>"` |
|---|---|---|
| Git for Windows, coreutils 8.32 | `\2d71…4881 *C:\\Users\\…\\h.txt` | `2d71…4881 *-` |
| Alpine (Docker), coreutils 9.11 | `\2d71…4881  /tmp/a\\b/libcef.dll` | `2d71…4881  -` |

On Linux and macOS a backslash in a path is rare. On Windows it is the normal
case, and both real callers produce one (§2).

### 1.1 End-to-end reproduction

The real script was run in Docker (Alpine, bash, coreutils 9.11) against a
runtime dir named `/tmp/rt\x`, with a pin file naming that dir's `libcef.dll`
hash:

| Variant | Exit | Output |
|---|---|---|
| `main` as of `9cc79a6` | 1 | `❌ … libcef.dll is not the pinned tracer-off build (sha256 \770e607624d…)` |
| line 50 reading stdin (`sha256sum < "$libcef"`) | 0 | `CEF runtime verified: t (tracer off)` |

Locally on Windows, the 2026-09-23 `task dev` failed with
`(sha256 \cb652dd7ae5…)`. The runtime's actual hash is
`cb652dd7ae5c1724ea7f2f1efcd4d72630685684aa0381c0570020a436881085`, which is
exactly `CEF_WINDOWS_LIBCEF_SHA256`.

## 2. Who hits it

The guard runs on `bundle:windows`'s final `cefDir` for every tier
(`Taskfile.yml`, `bash scripts/cef-build/verify-cef-runtime-windows.sh "$cefDir"`).

### 2.1 Tier 2: local default (`~/cef-build/...`), every Windows developer

`Taskfile.yml:676`:

```
cefBuildDefault="$HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64"
```

Task expands `$HOME` with its own shell interpreter, not Git Bash, and on
Windows that yields the Windows form `C:\Users\<user>`. The guard receives
`C:\Users\<user>/cef-build/...`: mixed separators, with backslashes. Observed on
2026-09-23 in a local `task dev`, whose error printed
`Runtime: C:\Users\asafe/cef-build/chromium_git/chromium/src/out/Release_GN_x64`.

By contrast, `$HOME` expanded *inside* a script run with `bash` is the POSIX form
`/c/Users/<user>`. Scripts that build paths internally are therefore not
affected unless given a Windows-form override (§2.4).

### 2.2 Tier 1: `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS`

Any value the user sets in Windows form (`C:\cef\r2`) fails. A forward-slash
value (`C:/cef/r2`, `/c/cef/r2`) passes. Setting a forward-slash override is
today's only workaround that keeps the guard on (§6).

### 2.3 CI: `build-windows.yml` (nightly artifacts and releases), expected

`build-windows.yml:65`:

```yaml
CEF_RUNTIME_DIR_WINDOWS: ${{ github.workspace }}/cef-runtime-windows
```

It is exported unchanged as `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS`
(`build-windows.yml:181-183`). On a Windows runner `github.workspace` is
`D:\a\agentmux\agentmux`, so the guard receives
`D:\a\agentmux\agentmux/cef-runtime-windows` and should refuse the correct r2
runtime:
- `scripts/package.sh` → `task bundle` → `bundle:windows` → exit 1.

**Not yet observed.** Every Windows CI build since the guard landed
(2026-09-24T05:14Z) has either not run or not reached `bundle:windows`:
- The last `ci-nightly-artifacts.yml` run was 2026-09-23T11:18Z, before #3615.
- Scheduled nightlies have been starting around 11:00–12:30Z.
- No `release.yml` run since.

`ci-pr` never runs `bundle:windows`: its Windows job is `check --tests + test`.
The guard's unit test runs on Ubuntu, with paths from `mktemp` (§4), so PR CI
could not have caught this.

**Expected outcome:**
- Today's Windows nightly fails.
- The next release fails at the Windows build.

Both fail loudly, not silently, and nothing wrong ships. But releases stay
blocked until this is fixed.

### 2.4 Latent: the same pattern elsewhere

| Site | Path source | Affected today? |
|---|---|---|
| `scripts/inject-exe-icon.sh:30,33` (rcedit pin check) | `${AGENTMUX_BUILD_TOOLS:-$HOME/.agentmux/build-tools}`, with `$HOME` expanded in Git Bash, so POSIX | No, unless `AGENTMUX_BUILD_TOOLS` is Windows-form. Then every build re-downloads rcedit, and the post-download check fails with `exit 1`, **breaking packaging**. |
| `scripts/package-portable.sh:219,232` (tool cache) | `$HOME/.agentmux/tool-build-cache`, POSIX | No. Same latent shape. |
| `scripts/cef-build/verify-cef-runtime-windows.test.sh:27,62` | `mktemp -d` | No, but that is why the test never saw it (§4). |

## 3. Fix

### 3.1 Hash from stdin, everywhere a hash is compared

With no file name there is nothing to escape, so the output is always
`<hash>  -`:

```bash
actual="$(sha256sum < "$libcef" | cut -d' ' -f1)"
```

Apply this at every `sha256sum "<file>" | cut` site in §2.4. It is the same
one-token change at each: `sha256sum "$x"` → `sha256sum < "$x"`. The guard's
error message then shows the real hash prefix (`cb652dd7ae5…`) instead of the
misleading `\cb652dd7ae5…`.

Alternatives considered and rejected:
- **Stripping a leading `\` from the output.** It works, but it encodes a
  coreutils quirk that the next reader has to rediscover.
- **Normalizing `cefDir` with `cygpath -u`.** It fixes this caller only, leaves
  every other `sha256sum` caller exposed, and adds a Git-Bash-only dependency to
  a script whose test runs on Linux.

**No shared helper.** A sourced `sha256_file()` was considered. It is not
worth a new `scripts/lib/` for one idiom that §3.2's gate now enforces at every
site.

### 3.2 Gate: forbid file-argument hash parsing

The `doc status + grep gates` job already runs grep gates such as
`check-menu-positioning.sh` and `tools/lint/check-input-handler-layout-reads.sh`.
Add one more: `scripts/check-hash-from-stdin.sh`.
- It fails on any `sha(1|224|256|384|512)sum|md5sum|b2sum|shasum` given a file
  argument whose output is piped on the same line. It scans the git-tracked
  `scripts/**/*.sh`, `tools/**/*.sh`, `Taskfile*.yml` and
  `.github/workflows/*.yml`, and ignores comment lines.
- A file argument may not start with a digit. That way `shasum -a 1 | cut`
  reads `1` as the option's value rather than as a file name.
- It names this spec in its error.
- Existing stdin uses (`shasum < "$f"`, `printf … | shasum`) already pass.

### 3.3 Correct the 09-23 spec

Add a dated note to `SPEC_WINDOWS_CEF_RUNTIME_VERIFY_OR_FAIL_2026_09_23.md` §3.4
("Both pass unchanged"), pointing here. Leave its Status as is: the guard is
implemented, and this is a defect in it.

## 4. Tests

In `scripts/cef-build/verify-cef-runtime-windows.test.sh`, which runs on Ubuntu
in `ci-pr`. Linux allows `\` in a directory name, and coreutils escapes it the
same way there (§1), so the Windows failure can be reproduced on Linux:

1. **Pinned `libcef.dll` in a directory whose name contains `\` passes.**
   Example: `$TMP/rt\win-style`. This case fails on `main` today (§1.1).
2. **A non-pinned DLL in a backslash directory still fails.**
3. **The failure message shows the real hash prefix, without a leading `\`.**
   Also change the test's own `GOOD_SHA` (line 27) and the "shows the hash it
   found" assertion (line 62) to the stdin form, so the test can't share the
   script's bug.

Plus `scripts/check-hash-from-stdin.test.sh`, run by `ci-pr` together with the
gate:
- 5 violating shapes, including the exact line 50 from #3615;
- 8 clean shapes: stdin, piped input, `shasum -a N < f`, `-c`, a hash printed
  with no pipe, a comment, and a lookalike name;
- the repo itself is clean.

**Manual, on Windows, before merging:**
- [ ] `task dev` with no `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS` set: tier 2
      (`C:\Users\…/cef-build/…`) prints `CEF runtime verified: …-r2 (tracer off)`.
- [ ] Same with `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS='C:\…\cef-windows-…-r2'`
      (backslash form).
- [ ] A non-pinned `libcef.dll` is still refused, with its real hash shown.

**CI, after merging:** run `ci-nightly-artifacts.yml` with `workflow_dispatch`
and confirm the Windows job passes `bundle:windows`. Don't wait for the next
scheduled run.

## 5. Rollout and urgency

- **Merge before the next release.** Until then, a release fails at the Windows
  build (§2.3).
- **If today's scheduled nightly has already failed at `bundle:windows`,** record
  the run URL in this spec as the confirmation of §2.3, then re-run it with
  `workflow_dispatch` after the merge.
- **No change to `windows-runtime-pin.sh`, `release.yml` `WIN_TAG`, or any
  runtime.** The pin (r2, `cb652dd7…`) is correct, and so is the r2 runtime at
  `~/cef-build`.

## 6. Workarounds until merged

- **Keeps the guard on (preferred):** point the build at the runtime through a
  forward-slash path:
  - `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS="$HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64" task dev`,
    run from Git Bash so that `$HOME` is `/c/Users/<user>`; or
  - an equivalent `C:/…` path.
- **Not recommended:** `AGENTMUX_ALLOW_UNVERIFIED_CEF=1` bypasses the check. It
  is safe *only* after independently confirming that the runtime's `libcef.dll`
  hash is `cb652dd7…`, and it stops protecting against a stale tracer-on runtime
  for as long as it is set.

## 7. Out of scope

- The guard's policy: hash pinning, no exception for local builds, the opt-out.
  Unchanged.
- `inject-exe-icon.sh`'s separate "rcedit busy / Unable to commit changes" retry
  failure, seen in the same 2026-09-23 dev log. It is a Defender/indexer handle
  race and already degrades to a warning.
