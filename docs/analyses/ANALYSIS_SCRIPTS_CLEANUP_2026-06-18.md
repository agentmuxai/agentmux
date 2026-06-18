# Scripts Folder Cleanup Analysis — 2026-06-18

## Summary

The `scripts/` directory contains 23 shell scripts, 2 PowerShell scripts, and 8 JS/CJS/MJS files. Several JS files are one-off diagnostic tools from closed investigations with no Taskfile entry and no references outside their own docs. These are safe to remove. All shell/PS1 scripts are either wired into `Taskfile.yml` or are clearly operational utilities (import, wipe, splash test).

---

## Verdict by file

### Keep — wired into Taskfile.yml (build pipeline)

| Script | Task |
|--------|------|
| `bump-wrapper.sh` | called by `release.sh` |
| `changeset.sh` | `task changeset` |
| `release.sh` | `task release` |
| `dev-local.sh` | `task dev:local` |
| `vite-build.sh` | `task build:*` (dev + prod) |
| `package.sh` | `task package`, `task package:release` |
| `package-portable.sh` | called by `package.sh` |
| `package-macos.sh` | `task package:macos` |
| `package-msix.ps1` | `task package:msix` |
| `package-installer.ps1` | `task package:installer` |
| `build-appimage-linux.sh` | `task package:appimage` |
| `linux-apprun.sh` | called by Linux build task |
| `install-linux-desktop.sh` | called by Linux dev + package tasks |
| `resolve-cef-runtime.sh` | called by Windows/Linux build task |
| `resolve-cef-runtime-darwin.sh` | called by macOS build task |
| `repair-cef-extract.sh` | fallback in cargo build task |
| `verify-cef-patch.sh` | called by Linux build task |
| `verify-cef-version.sh` | called by dev task |
| `verify-package.sh` | `task verify:package` |
| `verify-release-consistency.sh` | `task verify:release` |
| `check-menu-positioning.sh` | `task check:menu-positioning` |
| `check-scrollbar-cursor.sh` | `task check:scrollbar-cursor` |

### Keep — operational utilities (no Taskfile entry, but intentional standalone)

| Script | Purpose |
|--------|---------|
| `import-agents.sh` | Migrate user agent definitions between version DBs — used at upgrades |
| `wipe-old-data-dirs.sh` | Dev-machine cleanup of legacy data dirs — destructive, intentionally manual |
| `test-splash.sh` | Manual pre-release splash screen visual check |

### Remove — one-off diagnostic JS files, investigations closed

| File | Last commit | Closed in |
|------|------------|-----------|
| `cdp-sbflush-probe.js` | #1375 (terminal scrollbar flush) | Closed — fix shipped |
| `cdp-gap-probe.js` | #1372 (terminal margin) | Closed — fix shipped |
| `cdp-term-smoke.mjs` | #1370 (xterm-6 migration) | Closed — xterm-5 retired |
| `bench-cdp.mjs` | #850 (AuthFlowController) | Closed — ancient, PR ~18 months old |
| `capture-trace.cjs` | #850 (AuthFlowController) | Closed — no current users |
| `gen-seed.js` | #850 (AuthFlowController) | Closed — no current users |
| `smoke-test-portable.cjs` | #850 (AuthFlowController) | Closed — superseded by CI |
| `verify-typing-fix.cjs` | #850 (AuthFlowController) | Closed — one-time typing bug verification |
| `redact-pii.mjs` | #990 (transcript recovery) | Closed — one-off PII redaction job |

Also in `scripts/` but not a script: `benchmarks/` subdir and `cef-build/` subdir, `dev-tools/` symlink — not touched here.

---

## Release system clarity

The bump/release flow tripped up multiple agents this session. The correct invocation is:

```bash
# Standard release (auto-detects bump type from changesets):
task release

# Force a specific bump type (e.g. patch even when feat changesets exist):
bash scripts/release.sh --as patch

# Then commit and push:
git commit -m "chore: release vX.Y.Z"
git push -u origin <branch>
```

**Never use `scripts/bump-wrapper.sh` directly for a release** — it bumps versions but does not consume changesets or update VERSION_HISTORY. It is an internal detail called by `release.sh`.

**`task release --as patch` does NOT work** — `task` does not forward `--as` to the script. Use `bash scripts/release.sh --as patch` directly.

---

## Action taken

Removed the 9 one-off JS/CJS/MJS diagnostic files listed above. No shell scripts or PowerShell scripts removed. No Taskfile entries affected.
