# WinGet package bootstrap: `AgentMux.AI`

**Author:** Agent3
**Created:** 2026-09-10
**Status:** Active — implementing
**Related:** `Taskfile.yml` (`artifacts:winget:publish:*`), `.github/workflows/release.yml` (commented-out `winget` job), `docs/specs/SPEC_UNIFIED_RELEASE_CICD_2026_06_29.md`, `docs/specs/SPEC_RELEASE_CICD_CORRECTION_2026_06_30.md`, `docs/specs/SPEC_MSIX_PACKAGING_2026_05_30.md` (the **separate, already-published** MSIX/Store identity — see §2)

---

## 1. Problem

AgentMux has never actually been submitted to WinGet (`microsoft/winget-pkgs`).
Release automation for it exists but has been **disabled since 2026-08-12**
(commit `411056d`, PR #2549): the CI job called `wingetcreate update
AgentMux.AgentMux`, which only works against an **existing** manifest. No
manifest was ever bootstrapped via a first-time `wingetcreate new`
submission, so the job failed identically on every release back through at
least v0.54.14 (`repos/microsoft/winget-pkgs/.../AgentMux was not found`).
It was `continue-on-error: true` (never blocked a release) but provided zero
value, so it was turned off rather than left failing silently forever.

## 2. Naming — `AgentMux.AI`, and why it's safe to pick freely

WinGet's `PackageIdentifier` is its own namespace, independent of the MSIX
package identity already published to the Microsoft Store. **These are two
different things that happen to look similar:**

| Identity | Value | Status | Safe to rename? |
|---|---|---|---|
| MSIX `Identity/@Name` (Store) | `AgentMux.AgentMux` | **Already published** — Store ID `9P9QCXNNCRK3`, PFN `AgentMux.AgentMux_vqr1k32tkfk4y` (`packaging/msix/AppxManifest.xml.template`, `scripts/package-msix.ps1`) | **No** — changing it would break the update chain for existing Store installs. Out of scope here; untouched by this spec. |
| WinGet `PackageIdentifier` | previously planned as `AgentMux.AgentMux` (`Taskfile.yml`'s `WINGET_PACKAGE` var, the disabled CI job) | **Never submitted** — no manifest exists in `microsoft/winget-pkgs` | **Yes** — nothing is published under it yet, so there is no compatibility cost to choosing a different identifier now, before the one-time bootstrap. |

Decision (repo owner, 2026-09-10): the WinGet identifier is **`AgentMux.AI`**
— `Publisher = AgentMux`, `PackageName` segment `AI`. This does not touch the
MSIX/Store identity at all; it only renames the not-yet-existing WinGet
manifest's identifier before it's created for the first time.

## 3. What changes in this repo

- `Taskfile.yml`: `WINGET_PACKAGE: AgentMux.AgentMux` → `AgentMux.AI`.
- `.github/workflows/release.yml`: fix the identifier in the commented-out
  `winget` job's `wingetcreate update` call for when it's re-enabled. **Not
  re-enabled in this PR** — `wingetcreate update` still requires the manifest
  to exist first; flipping it on now would just resume the same failure mode
  under the new name. Re-enable as a follow-up once §4's bootstrap PR is
  merged into `microsoft/winget-pkgs`.
- This spec, as the record of the decision and the two-identity distinction
  in §2 (so nobody "fixes" the MSIX identity to match the WinGet one later).

## 4. Bootstrap procedure (one-time, manual — the actual "deploy" step)

Not automatable the first time: `wingetcreate new` walks/generates a fresh
manifest and opens a PR against `microsoft/winget-pkgs`, a repo this project
doesn't own.

1. Install `wingetcreate` (`winget install Microsoft.WingetCreate` or fetch
   the release binary directly).
2. Run against the latest real GitHub Release asset — installer is Inno Setup
   (`scripts/package-installer.ps1`), a native WinGet `InstallerType: inno`,
   which WinGet handles with standard silent-install switches automatically:
   ```
   wingetcreate new AgentMux.AI \
     --version <ver> \
     --urls "https://github.com/agentmuxai/agentmux/releases/download/v<ver>/AgentMux-<ver>-x64-setup.exe" \
     --submit \
     --token $WINGET_TOKEN
   ```
   Publisher: `AgentMux`. PackageName: `AgentMux`. License: `Apache-2.0`
   (`package.json`'s `license` field — matches the real repo license).
3. This opens a PR on `microsoft/winget-pkgs`. Automated validation there
   checks the URL is reachable, the installer runs silently, and the
   version/hash match. A human moderator merges on success — typically within
   a day or two, out of this repo's control.
4. Once merged upstream, re-enable the `winget` job in `release.yml` (switch
   `wingetcreate new` → `update` in the bootstrap's mental model — the CI job
   already uses `update`, it just needs uncommenting) so every future release
   auto-submits a version-bump PR.

## 5. Verification

- `Taskfile.yml`/`release.yml` reference the new identifier consistently
  (`grep -rn "AgentMux.AgentMux"` should only still match the MSIX-specific
  files listed in §2's table — `packaging/msix/`, `scripts/package-msix.ps1`,
  and historical/archive docs describing the Store identity).
- `WINGET_TOKEN` repo secret already exists (confirmed via
  `gh api repos/agentmuxai/agentmux/actions/secrets`) — not re-verified for
  validity/scope here; the bootstrap submission itself is the real test.
- Bootstrap success: the `microsoft/winget-pkgs` PR opens and its automated
  validation checks pass. Full success (merged, `winget show AgentMux.AI`
  resolves for any Windows user) depends on Microsoft's moderation queue and
  is outside this repo's control/timeline.
