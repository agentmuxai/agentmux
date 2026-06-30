# Unified Release CI/CD — agentmux + MS Store + Landing Page
**Date:** 2026-06-29  
**Status:** Draft  
**Scope:** agentmuxai/agentmux, agentmuxai/agentmux-landing, a5af/shared-infrastructure

---

## 1. Problem Statement

Three delivery surfaces exist but are not connected into a single release flow:

| Surface | Current state |
|---|---|
| **agentmuxai/agentmux** builds | Nightly artifacts CI passes (Linux/macOS); Windows fixed by PR #1845; no release workflow |
| **Microsoft Store** | Spec'd in PR #1843 (not yet wired); requires Partner Center setup first |
| **agentmux.ai landing page** | CDK + `fetch-release.mjs` deployed manually; no CI/CD workflow |
| **gh-reporter nightly email** | Lambda healthy; `infrastructure-weekly-analysts` stack in `UPDATE_ROLLBACK_COMPLETE` |

Today a release requires manual execution of at least five independent steps across four repos/systems. Gaps: artifacts not published to GitHub Releases, landing page not auto-updated, MS Store submission manual.

---

## 2. Design Dimensions Considered

| Dimension | Option A | Option B | **Chosen** |
|---|---|---|---|
| Release trigger | Separate per-surface triggers | Manual dispatch only | **`v*` tag → single fan-out workflow** |
| Artifact source | Rebuild on release | Reuse nightly artifacts | **Rebuild: nightly uses local-label channel, release needs `stable` channel baked in** |
| Landing page wiring | Polling / cron | Repository dispatch from agentmux | **`repository_dispatch` event from release workflow** |
| MS Store timing | Same job as release | Separate, non-blocking | **Non-blocking parallel job: cert failure must not block the release** |
| Infrastructure fix | Leave rollback stack | Delete + redeploy | **Rename SSM docs (no delete) → clean update** |

**Rationale for single-tag-trigger fan-out:** a release is an atomic event — version bump, changelog, all artifacts, all channels should ship together or be clearly tracked as failing. Separate triggers (one per surface) create version skew risk where the landing page shows v0.49.8 while the Store still shows v0.48.x.

---

## 3. Target Architecture

```
git tag v0.50.0 → push
          │
          ▼
  ┌───────────────────────────────────────────────────────────┐
  │       release.yml  (agentmuxai/agentmux)                  │
  │                                                           │
  │  ┌──────────────┐                                         │
  │  │ verify       │  assert tag == VERSION_HISTORY ==       │
  │  │              │  package.json == Cargo.toml             │
  │  └──────┬───────┘                                         │
  │         │ (pass)                                          │
  │         ▼                                                 │
  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐    │
  │  │ build:win    │  │ build:linux  │  │ build:macos  │    │
  │  │ portable.zip │  │ AppImage     │  │ DMG arm64    │    │
  │  │ setup.exe    │  │ .deb         │  │              │    │
  │  │ .msix        │  │              │  │              │    │
  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘   │
  │         └──────────────────┴──────────────────┘          │
  │                            │                              │
  │                            ▼                              │
  │  ┌─────────────────────────────────────────────────────┐  │
  │  │ publish                                             │  │
  │  │  1. Create GitHub Release + changelog               │  │
  │  │  2. Upload artifacts to GH Release                  │  │
  │  │  3. Mirror to S3 dl.agentmux.ai/releases/<ver>/     │  │
  │  │  4. Mirror to S3 dl.agentmux.ai/releases/latest/    │  │
  │  └──────────────────────────┬──────────────────────────┘  │
  │                             │                              │
  │    ┌────────────────────────┼────────────────────────┐    │
  │    ▼                        ▼                        ▼    │
  │ ┌──────────┐         ┌───────────┐          ┌──────────┐  │
  │ │ winget   │         │ ms-store  │          │ landing  │  │
  │ │ PR via   │         │ msstore   │          │ dispatch │  │
  │ │ winget-  │         │ publish   │          │ event →  │  │
  │ │ create   │         │ (non-     │          │ landing  │  │
  │ │          │         │ blocking) │          │ deploy   │  │
  │ └──────────┘         └───────────┘          └──────────┘  │
  └───────────────────────────────────────────────────────────┘
                                                      │
                                                      ▼
                                    ┌─────────────────────────────┐
                                    │  landing-deploy.yml          │
                                    │  (agentmuxai/agentmux-landing│
                                    │                              │
                                    │  1. fetch-release.mjs        │
                                    │     → public/release.json    │
                                    │  2. vite build               │
                                    │  3. CDK deploy               │
                                    │  4. CloudFront invalidation  │
                                    └─────────────────────────────┘
```

---

## 4. Implementation Plan

### 4.1 Fix: `infrastructure-weekly-analysts` stuck stack

**Root cause:** CDK renamed two SSM Documents (`weekly-security-researcher`, `weekly-architecture-analyst`) with explicit `documentName` properties. CloudFormation cannot replace a custom-named resource in-place — it requires create-new + delete-old, which it won't do if the old name is still in use.

**Fix** (in `a5af/shared-infrastructure`):
1. Remove `documentName` props from both SSM Document constructs in `weekly-analysts` CDK stack (let CDK generate unique names).
2. `cdk deploy infrastructure-weekly-analysts` — without custom names, CloudFormation can replace by creating new names first.
3. Verify stack reaches `UPDATE_COMPLETE`.

No user-visible impact: these are internal SSM automation documents, not exposed externally.

---

### 4.2 New: `release.yml` (agentmuxai/agentmux)

**Trigger:**
```yaml
on:
  push:
    tags: ['v[0-9]+.[0-9]+.[0-9]+']
  workflow_dispatch:
    inputs:
      tag:
        description: 'Tag to release (e.g. v0.50.0)'
        required: true
```

**Job: `verify`** (ubuntu-latest, fast)
- Checkout at tag
- Assert `package.json` version == tag suffix == `VERSION_HISTORY.md` top entry == `Cargo.toml` workspace version
- Fail loudly if any mismatch (release-consistency invariant, see `scripts/release.sh`)

**Jobs: `build-windows`, `build-linux`, `build-macos`** (parallel, depend on `verify`)

_Windows_ (`windows-latest`):
- Same steps as `ci-nightly-artifacts.yml` windows job (Ninja, go-task, Inno Setup, rust-cache)
- `STRIP_MAPS=1 RELEASE_CHANNEL=stable bash scripts/package.sh`
- Package installer: `scripts/package-installer.ps1`
- Package MSIX: `scripts/package-msix.ps1` (unsigned — Store re-signs)
- Upload: portable.zip, setup.exe, AgentMux_<ver>_x64.msix

_Linux_ (`ubuntu-22.04`):
- Same steps as `ci-nightly-artifacts.yml` linux job (patched CEF download)
- Outputs: AppImage, .deb

_macOS_ (`macos-latest`):
- Same steps as `ci-nightly-artifacts.yml` macos job (Apple cert, notarize, patched CEF TBD)
- Output: AgentMux_<ver>_arm64.dmg
- **Note:** macOS CI currently uses stock CEF (no patched framework uploaded to `agentmuxai/cef` yet). This is a known gap — macOS DMG will crash on macOS 26 until the patched framework is published. Gate the macOS release job on `APPLE_CEF_AVAILABLE` secret or a required input.

**Job: `publish`** (ubuntu-latest, depends on all three build jobs)
```
needs: [build-windows, build-linux, build-macos]
```
Steps:
1. Download all build artifacts
2. Verify artifact filenames contain the expected version
3. `gh release create v<ver> --notes "$(scripts/extract-changelog.sh)"` with all artifacts attached
4. `aws s3 cp` each artifact → `s3://dl.agentmux.ai/releases/<ver>/`
5. `aws s3 cp` each artifact → `s3://dl.agentmux.ai/releases/latest/` (unversioned names per `UNVERSIONED_NAMES` map in `fetch-release.mjs`)
6. `repository_dispatch` to `agentmuxai/agentmux-landing` with `event_type: release` and `client_payload: { version: "<ver>" }`

**Job: `winget`** (ubuntu-latest, depends on `publish`)
- `wingetcreate update AgentMux.AgentMux --version <ver> --urls <installer-url> --token $WINGET_TOKEN`
- Non-blocking: `continue-on-error: true`

**Job: `ms-store`** (windows-latest, depends on `publish`)  
- `msstore publish` the MSIX from S3 or GitHub Release URL
- Non-blocking: `continue-on-error: true`
- Full details: see PR #1843 (`agentx/spec-msstore-automated-release`)
- **Prerequisite:** Partner Center app registered, service principal created, four `MSSTORE_*` secrets added to repo

---

### 4.3 New: `landing-deploy.yml` (agentmuxai/agentmux-landing)

**Trigger:**
```yaml
on:
  repository_dispatch:
    types: [release]          # fired by agentmux release.yml publish job
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to deploy (leave blank for latest)'
        required: false
```

**Job: `deploy-prod`** (ubuntu-latest)
Steps:
1. Checkout `agentmuxai/agentmux-landing` main
2. `npm ci`
3. `node scripts/fetch-release.mjs` (writes `public/release.json` with S3 URLs)
4. `npm run build` (vite build, `VITE_ENV=prod`)
5. `npx cdk deploy landing-prod --require-approval never`
6. CloudFront invalidation: `aws cloudfront create-invalidation --distribution-id $DIST_ID --paths "/*"`

**Secrets needed:**  
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` (existing deploy role), `GITHUB_TOKEN` (for `fetch-release.mjs` gh CLI calls)

---

### 4.4 Fix: `scripts/extract-changelog.sh` (new helper)

The `publish` job needs the changelog entry for the current version from `VERSION_HISTORY.md`. Add a small script that extracts the block between `## <ver>` and the next `##` heading — used for `gh release create --notes`.

---

## 5. Sequencing / Prerequisites

```
Week 1
  ├── Fix infrastructure-weekly-analysts CFN stack (a5af/shared-infrastructure)
  ├── Add landing-deploy.yml to agentmuxai/agentmux-landing
  └── Add extract-changelog.sh to agentmuxai/agentmux

Week 2
  ├── Add release.yml (verify + build + publish + winget jobs) to agentmuxai/agentmux
  ├── Wire repository_dispatch → landing-deploy.yml
  └── Test with a patch release tag (v0.49.9 dry-run dispatch)

Week 3 (after Partner Center setup)
  └── Add ms-store job to release.yml, add MSSTORE_* secrets
```

**MS Store prerequisites (manual, one-time):**
1. Reserve "AgentMux" name in Partner Center
2. Complete first manual submission (age ratings, category, screenshots)
3. Create Entra service principal with Store Publisher Manager role
4. Add four secrets: `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`, `MSSTORE_CLIENT_SECRET`, `MSSTORE_SELLER_ID`

---

## 6. Open Questions

| # | Question | Owner |
|---|---|---|
| OQ1 | macOS patched CEF: gate release on `PATCHED_CEF_AVAILABLE` secret, or always ship stock + warn? | Needs decision |
| OQ2 | `dl.agentmux.ai` S3 bucket — is it under `a5af/shared-infrastructure` CDK or separate? Confirm IAM permissions for release workflow | Needs audit |
| OQ3 | gh-reporter email delivery: zero bounces/rejects from SES, not in spam. Is there a Gmail filter? | Still open |
| OQ4 | Should `ci-nightly-artifacts.yml` stay separate from `release.yml`? Recommendation: yes — nightly is a health signal on HEAD, release is a publishing event on a tag. | Recommend: keep separate |
| OQ5 | Landing page QA deploy — should it auto-deploy on every `main` push too, not just release events? | Needs decision |

---

## 7. Files to Create / Modify

| File | Repo | Action |
|---|---|---|
| `.github/workflows/release.yml` | agentmuxai/agentmux | **Create** |
| `scripts/extract-changelog.sh` | agentmuxai/agentmux | **Create** |
| `.github/workflows/landing-deploy.yml` | agentmuxai/agentmux-landing | **Create** |
| `weekly-analysts/lib/weekly-analysts-stack.ts` | a5af/shared-infrastructure | **Edit** — remove custom `documentName` props |
| `gh-reporter/` | a5af/shared-infrastructure | No change needed (stack healthy) |

---

## 8. What Is NOT in Scope

- **Nightly build/test CI** (`ci-nightly-build.yml`, `ci-nightly-artifacts.yml`) — kept separate, not merged into release
- **Auto-versioning** — version bump continues to go through `task release` + changesets; this spec only wires the post-bump publication
- **macOS patched CEF CI** — tracked separately; the patched framework must be built locally and uploaded to `agentmuxai/cef` before CI can use it
- **gh-reporter email delivery investigation** — SES is healthy; root cause TBD
