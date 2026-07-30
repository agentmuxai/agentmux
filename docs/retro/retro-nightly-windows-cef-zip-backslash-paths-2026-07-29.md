# Retro: nightly Windows build silently failing to extract codec-enabled CEF (backslash zip paths)

**Date found:** 2026-07-29
**Severity:** High — the Windows job of `ci-nightly-artifacts.yml` has failed
outright on every run since at least 2026-07-28 whenever the CEF-runtime
cache misses; no nightly Windows artifact (codec or otherwise) has been
produced in that window.

## TL;DR

`docs/cef-build/build-patched-cef-windows.md` §7 instructs creating the
codec-enabled CEF runtime release zip with PowerShell's `Compress-Archive`.
That cmdlet stores internal zip entries with backslash (`\`) path separators
for nested directories (notably `locales\*.pak`). Both `ci-nightly-artifacts.yml`
and `build-windows.yml` extracted that zip with bash's `unzip` (Info-ZIP),
which does not parse backslash-separated internal paths as nested
directories — it treats each entry as a single flat (and effectively
unusable) filename. The subsequent `find ... -name "libcef.dll"` lookup then
always came up empty, and the step failed with `ERROR: libcef.dll not found
in zip`.

## Why it wasn't obvious

The overall nightly workflow's pass/fail history looked mixed (some green,
some red) rather than consistently red, which read as ordinary flakiness.
The green runs were misleading: they hit a `rust-cache` restore where
`cef-dll-sys`'s build script didn't need to re-run at all, so the
CEF-runtime download/extract step was skipped entirely that day — not proof
the extraction path worked, just proof it wasn't exercised. The two days
this got caught (2026-07-28, 2026-07-29) both hit a genuine cache miss and
failed identically.

## Fix

Switch the Windows-side extraction from bash `unzip` to PowerShell's
`Expand-Archive` in both workflows — the same .NET zip implementation that
created the archive, so it round-trips regardless of internal path
separator style. No change needed to the zip-creation side
(`build-patched-cef-windows.md` §7) and no need to rebuild/re-release the
already-published CEF binary; this was purely a consumption-side bug.

## Prevention

When a CI step downloads an artifact produced by a *different* platform's
native tooling than the one doing the extracting (here: a Windows-native
`Compress-Archive` zip, consumed by a Linux-heritage `unzip` running under
Git Bash on the same Windows runner), don't assume standard zip format
compatibility — verify with an artifact actually produced by that exact tool,
not a hand-crafted test zip. The macOS/Linux equivalents of this pipeline use
`.tar.gz`, produced and consumed by the same POSIX toolchain end-to-end, so
they never had this class of bug — worth keeping platform-native tool
symmetry (create-with-X, extract-with-X) in mind for any future artifact
pipeline crossing this same boundary.

## Files

- `.github/workflows/ci-nightly-artifacts.yml` — Windows job's CEF-runtime
  download/extract step, switched to `pwsh`/`Expand-Archive`.
- `.github/workflows/build-windows.yml` — same fix, same pattern.
- `docs/cef-build/build-patched-cef-windows.md:208-228` — the
  `Compress-Archive` step that produces the backslash-path zip (unchanged;
  the fix is entirely on the consuming side).
- `Taskfile.yml`'s `bundle:windows` task — second, distinct bug found while
  live-verifying the fix above (see addendum below).

## Addendum (same day): a second, distinct bug surfaced once extraction was fixed

With the zip extraction fixed, a live verification run
(`gh workflow run ci-nightly-artifacts.yml --ref
fix/windows-cef-zip-extract-backslash-paths`) confirmed the codec-enabled CEF
runtime now resolves and version-matches correctly (`✓ CEF runtime OK:
libcef.dll ... matches linked cef crate major 148`) — but the build then
failed one step later, in `Taskfile.yml`'s `bundle:windows` task:

```
task: Failed to run task "bundle:windows": GetFileAttributesEx
D:\a\agentmux\agentmux/cef-runtime-windows/vulkan-1.dll: The system cannot
find the file specified.
```

`vulkan-1.dll` (part of the optional SwiftShader software-GL fallback trio)
happens to be absent from this particular codec-enabled CEF build. The
bundling script's copy line for it —
`cp -f "$cefDir/vulkan-1.dll" dist/cef/ 2>/dev/null || true` — was written to
tolerate exactly that (matching every other optional-file copy in the same
block). It didn't: Task's shell (`mvdan.cc/sh`) runs `cp` as a native Go
builtin rather than spawning a real `cp.exe`, and on Windows a missing
*source* file surfaces as a hard task-engine error (a raw
`GetFileAttributesEx` failure) that bypasses `2>/dev/null || true`
entirely — that shell-level suppression only catches errors the builtin
reports through normal stderr/exit-code channels, not this one.

Fixed by replacing every `cp ... 2>/dev/null || true` in `bundle:windows`
with an explicit `[ -f src ] && cp ...` existence check — the same pattern
the block already used for the locale-file copy, which is why that one line
never hit this bug. Scoped to `bundle:windows` only: `bundle:linux`'s
identical-looking `cp ... 2>/dev/null || true` lines run on a Linux runner,
where Task's builtin `cp` doesn't go through `GetFileAttributesEx` and the
Linux job in the same run completed successfully — no evidence that side is
affected, so left as-is rather than speculatively changed.
