# SPEC — Windows builds refuse a CEF runtime that isn't the pinned tracer-off build

**Status:** implemented (#3615) — `scripts/cef-build/verify-cef-runtime-windows.sh`, wired into `bundle:windows` (Taskfile.yml), pin in `scripts/cef-build/windows-runtime-pin.sh`, tests in `scripts/cef-build/verify-cef-runtime-windows.test.sh` (run by ci-pr).
**Date:** 2026-09-23
**Author:** agentx
**Related:**
- `docs/incident/INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md` (§3a: `InstanceTracer` mutex deadlock; the lock at `libcef+0x11656b38`)
- #3561 (pinned the tracer-off runtime `cef-windows-x86_64-152.0.7977.83-r2`)
- #3592 (`fetch-patched-cef-windows.sh` re-fetches a stale cached runtime)
- `docs/specs/SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03.md` (the sibling version guard this sits next to)

---

## 1. Problem

Every Windows `libcef.dll` built with `enable_backup_ref_ptr_instance_tracer=true`
can deadlock any process that loads it: the browser/host UI thread or a renderer.
The tracer is on in upstream CEF's Windows release builds and in every runtime
AgentMux shipped before r2. On 2026-09-23 a locally packaged 0.56.13 froze twice
in 40 minutes, once in the host and once in a renderer. Both times it was that
lock, because the local package bundled the pre-r2 runtime cached on 09-15.

#3592 made `fetch-patched-cef-windows.sh` replace a stale cached runtime. That
closes the common case, but `bundle:windows` still **builds with a tracer-on
runtime without stopping** in four situations:

| Situation | What happens today |
|---|---|
| The fetch fails: `gh` not logged in, no `GH_TOKEN`, or offline | The script keeps the stale cached runtime (by design: the stock fallback is no safer) and exits 1. The Taskfile ignores the exit code (`\|\| true`) and bundles the stale runtime. On the 09-23 machine the default `gh` login was broken, so this is what would have happened. |
| No cache and no fetch possible | Tier 3 bundles the **stock** cef-dll-sys runtime. It's also tracer-on (upstream `gn_args.py` enables the tracer for every non-debug Windows build). The only signal is the "no codecs" advisory, which doesn't mention the hang. |
| `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS` set | Nothing is checked beyond file existence and the CEF major version. |
| A local Chromium compile predating `args-windows.gn`'s fix | The fetch script deliberately never touches it. Nothing checks its args. |

The failure is silent and delayed. The build succeeds, the app works, and it
hangs at some random later moment. The more pane activity, the sooner.

## 2. Goal

`task package` / `task dev` on Windows **stop with a clear, actionable error**
unless the bundled runtime is known to be tracer-off. An explicit escape hatch
exists for a deliberate exception.

Non-goals: macOS/Linux (their args files never set the tracer, and #3561 guards
it off there), and changing *which* runtime is fetched.

## 3. Design

### 3.1 One pin, by content

`scripts/cef-build/windows-runtime-pin.sh` (sourced, not executed) holds:

- `CEF_WINDOWS_RELEASE_TAG`, the agentmuxai/cef release;
- `CEF_WINDOWS_ASSET`, its zip;
- `CEF_WINDOWS_LIBCEF_SHA256`, the SHA-256 of that release's `libcef.dll`
  (r2: `cb652dd7ae5c1724ea7f2f1efcd4d72630685684aa0381c0570020a436881085`,
  byte-identical to the official v0.56.13 and v0.56.14 portables' `runtime/libcef.dll`).

`fetch-patched-cef-windows.sh` reads its tag and asset from here instead of its
own copies, so the fetch and the check can't disagree.

The check is by **content hash**, not by the `.agentmux-cef-release` stamp
#3592 writes. The stamp only says what the fetch script installed. The hash says
what is actually on disk, whichever tier or override put it there.

### 3.2 The guard: `scripts/cef-build/verify-cef-runtime-windows.sh <dir>`

Accepts the runtime and exits 0 when either:

1. `sha256(<dir>/libcef.dll) == CEF_WINDOWS_LIBCEF_SHA256`, or
2. `<dir>` is a local Chromium build tree whose `args.gn` sets
   `enable_backup_ref_ptr_instance_tracer=false` **exactly once, to exactly
   `false`**, which **has a `build.ninja`** (it's a real `gn gen` tree), and
   whose `libcef.dll` is **newer than `args.gn` and `build.ninja`**. `args.gn`
   is configuration, not proof of what the DLL contains (Codex and ReAgent on
   #3615):
   - `args.gn` alone, next to a DLL copied in afterwards, ties nothing to that
     config. Without `build.ninja`, only the pinned hash is accepted;
   - a tree reconfigured with the fixed args but not yet, or not
     successfully, rebuilt still holds the old tracer-on DLL. `gn gen`
     rewrites `build.ninja` on every args change, so an older DLL is refused;
   - `falsey_nonsense`, a second assignment, or a conditional reassignment
     elsewhere in the file is refused.

Otherwise it exits 1 and prints: what was found (hash and tier), why it matters
(the incident, one line), and how to fix it. The fixes are `gh auth login` or
`GH_TOKEN=… task package` so the fetch can install the pinned runtime; point
`AGENTMUX_CEF_RUNTIME_DIR_WINDOWS` at the pinned runtime; or rebuild CEF with
`scripts/cef-build/args-windows.gn`.

**Escape hatch:** `AGENTMUX_ALLOW_UNVERIFIED_CEF=1` turns the failure into a
loud warning and exit 0, for a deliberate experiment with another runtime. It is
never set by CI.

### 3.3 Wiring

`bundle:windows` runs the guard on the final `cefDir` after
`verify-cef-version.sh` and `verify-angle-libs.sh`, for **every tier**:
override, cef-build default, and the stock cargo cache. `task dev` reaches
the same step (its `dev:serve` requires `dist/cef/libcef.dll` produced by the
bundle), so both commands are covered.

### 3.4 CI

`build-windows.yml` (releases and nightly artifacts) sets
`AGENTMUX_CEF_RUNTIME_DIR_WINDOWS` to a runtime downloaded by tag. Releases pin
r2 (`release.yml` `WIN_TAG`), and the nightly takes the newest
`cef-windows-x86_64-*` release, currently r2. Both pass unchanged. The guard
also turns a future pin drift into a failure. If someone publishes a new Windows
runtime or bumps `WIN_TAG` without updating `windows-runtime-pin.sh`, CI fails
at bundle time instead of shipping an unverified runtime.

**Bumping the runtime** is therefore one change, in one PR: `windows-runtime-pin.sh`
(tag, asset, libcef hash) plus `release.yml`'s `WIN_TAG`. The libcef hash comes
from the release zip's `libcef.dll`, not from the zip's own `SHA256SUMS.txt`
entry, which covers the archive.

## 4. Behavior change

A Windows machine that can't fetch the pinned runtime can no longer
`task package` / `task dev` until it can, or until the developer opts out with
the escape hatch. That's intended: the alternative is a build that hangs later
with nothing pointing at the cause. The error names the one-line fix.

## 5. Tests (`scripts/cef-build/verify-cef-runtime-windows.test.sh`)

Synthetic runtime dirs:

- pinned hash → pass; any other content → fail with the hash and the fix in the message;
- missing `libcef.dll` → fail;
- local build tree with `args.gn` tracer `=false` → pass (spacing, trailing comment, CRLF);
- `args.gn` with the tracer `=true`, not mentioning it, commented out,
  `=falsey_nonsense`, or assigned twice (one conditional) → fail;
- `libcef.dll` older than `args.gn` or `build.ninja` (reconfigured, not rebuilt) → fail;
  newer than both → pass; `args.gn` without `build.ninja` → fail;
- `AGENTMUX_ALLOW_UNVERIFIED_CEF=1` on a bad runtime → exit 0 with a warning;
- the fetch script and the guard read the same pin;
- `bundle:windows` calls the guard after the version check.

The pin constant itself is checked against the real r2 `libcef.dll` by hand in the
PR description. No CI test downloads a 190 MB runtime.
