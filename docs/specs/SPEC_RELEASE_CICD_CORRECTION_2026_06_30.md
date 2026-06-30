# Release CI/CD Correction — remove the `dl.agentmux.ai` fabrication

**Date:** 2026-06-30
**Status:** Approved — implementing
**Supersedes:** §3, §4.2, §4.3 of `SPEC_UNIFIED_RELEASE_CICD_2026_06_29.md`
**Scope:** agentmuxai/agentmux (`release.yml`), agentmuxai/agentmux-landing (`landing-deploy.yml`)

## 0. Guiding principle — connect, don't rebuild

Every piece of this pipeline **already worked** before this effort; the only thing
missing was the wiring between them. The build jobs already produce artifacts (nightly
proves this daily). The landing page already knows how to fetch the latest release from
GitHub and publish it to `agentmux.ai` (`fetch-release.mjs`, see §1). So the release
workflow's *entire* job is:

1. Build artifacts on the `stable` channel.
2. `gh release create` — publish them to **GitHub Releases** (the one handoff point).
3. Fire a `repository_dispatch` so the landing page redeploys.

It must **not** mirror to S3, configure AWS, or invent a CDN — `fetch-release.mjs`
already does all asset mirroring into the landing's own bucket. The `dl.agentmux.ai`
tier I added was a from-scratch reimplementation of a thing that already existed.

---

## 1. Why this correction exists

The original unified-release spec (2026-06-29) and the `release.yml` it produced
(merged in PR #1846) assume a CDN bucket/domain **`dl.agentmux.ai`** as the
artifact distribution channel. That assumption was never verified and is **false**:

- There is **no `dl.agentmux.ai` S3 bucket** and **no CloudFront distribution** with
  that alias. Verified 2026-06-30:
  - `aws s3 ls s3://dl.agentmux.ai/` → `NoSuchBucket`
  - `aws cloudfront list-distributions` → aliases are `agentmux.ai`, `www.agentmux.ai`,
    `docs.agentmux.ai`, `muxbus.agentmux.ai` only. No `dl.*`.
- The only release-artifact bucket that exists is **`agentmux-releases`** (legacy,
  last populated 2026-03 with `agentmux-0.31.x` portables).

**Source of the error:** carried forward from OQ2 in the prior spec ("`dl.agentmux.ai`
S3 bucket — is it under shared-infrastructure CDK or separate?") as if it were a
settled fact rather than an open question. It then hard-coded itself into the merged
`release.yml` publish + winget steps.

**Ground truth (decided 2026-06-30):** **GitHub Releases is the single source of
truth** for distribution. The landing page already mirrors assets into its **own**
bucket — `fetch-release.mjs` reads the GitHub Release via `gh`, downloads each asset,
re-uploads to `agentmux-landing-prod` (prod) / `agentmux-landing-qa`, and writes
`public/release.json` with `agentmux.ai` URLs. No separate release CDN is needed.

---

## 2. Corrected architecture

```
git tag v0.50.0 → push
          │
          ▼
  release.yml (agentmuxai/agentmux)
    verify → build-windows / build-linux / build-macos
                          │
                          ▼
                  ┌─────────────────────────────┐
                  │ publish                     │
                  │  1. gh release create v<ver>│  ← GitHub Release = source of truth
                  │     with all artifacts      │
                  │  2. repository_dispatch →   │
                  │     agentmux-landing        │
                  └──────────────┬──────────────┘
            ┌────────────────────┼────────────────────┐
            ▼                    ▼                     ▼
       ┌─────────┐         ┌──────────┐         ┌──────────────┐
       │ winget  │         │ ms-store │         │ landing      │
       │ URL =   │         │ (MSIX    │         │ deploy       │
       │ GH      │         │ from GH  │         │ (dispatch)   │
       │ Release │         │ Release) │         │              │
       │ asset   │         │          │         │              │
       └─────────┘         └──────────┘         └──────┬───────┘
                                                       ▼
                              landing-deploy.yml (agentmuxai/agentmux-landing)
                                1. fetch-release.mjs → reads GH Release,
                                   mirrors assets to agentmux-landing-prod,
                                   writes public/release.json
                                2. vite build (build:prod calls fetch-release.mjs)
                                3. cdk deploy (agentmux-landing-prod)
                                4. CloudFront invalidation
```

Key change vs. prior spec: **the `publish` job no longer touches S3 at all.** Steps
3 and 4 of the old publish job (`Mirror artifacts to S3 versioned` + `latest/`) and
the `Configure AWS credentials` step are deleted. agentmux's release workflow needs
**zero AWS credentials**.

---

## 3. Changes to `release.yml` (agentmuxai/agentmux) — already merged, needs a fix PR

The merged `release.yml` (PR #1846) contains the fabricated dependency. Fix PR will:

| # | Current (buggy) | Corrected |
|---|---|---|
| R1 | `publish` job step `Configure AWS credentials` | **Delete** — no AWS in release flow |
| R2 | `publish` step `Mirror artifacts to S3 (versioned path)` | **Delete** |
| R3 | `publish` step `Mirror artifacts to S3 (latest/ …)` | **Delete** |
| R4 | `publish` `env: S3_RELEASES: s3://dl.agentmux.ai/releases` | **Delete** |
| R5 | `winget` step: `INSTALLER_URL="https://dl.agentmux.ai/releases/${VERSION}/${INSTALLER_NAME}"` | **Replace** with the GitHub Release asset download URL: `https://github.com/agentmuxai/agentmux/releases/download/v${VERSION}/${INSTALLER_NAME}` |
| R6 | `ms-store` already downloads MSIX from the GH Release (`gh release download`) | No change — confirm it does not reference S3 |

Secrets the corrected `release.yml` requires (agentmuxai/agentmux):
- `LANDING_DEPLOY_TOKEN` — PAT (repo scope) to fire `repository_dispatch` at agentmux-landing
- `WINGET_TOKEN` — fork/PR token for the winget-pkgs manifest PR
- `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`, `MSSTORE_CLIENT_SECRET`, `MSSTORE_SELLER_ID` — MS Store (job is `continue-on-error: true`, so absence only skips Store submission)
- `A5AF_PACKAGES_TOKEN`, Apple signing secrets, `CEF_RUNTIME_TOKEN` — already present (build jobs)
- **No `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`** — removed by R1.

---

## 4. Changes to `landing-deploy.yml` (agentmuxai/agentmux-landing)

The landing deploy **does** need AWS — it runs `cdk deploy` for `agentmux-landing-prod`,
uploads mirrored assets to that bucket via `fetch-release.mjs`, and invalidates
CloudFront. Per the "no new IAM roles" decision (2026-06-30), it uses **static access
keys**, not OIDC.

| # | Item | Action |
|---|---|---|
| L1 | OIDC change I pushed (`id-token: write` + `role-to-assume: ${{ secrets.AWS_ROLE_ARN }}`) | **Revert** to `aws-access-key-id` / `aws-secret-access-key` static keys |
| L2 | `permissions: contents: read` | Keep (drop `id-token: write` added for OIDC) |
| L3 | `.npmrc` deleted after `npm ci` | Keep (P1 fix already merged) |
| L4 | CloudFront invalidation `exit 1` on empty DIST_ID | Keep (already merged) |
| L5 | Redundant explicit `fetch-release.mjs` step removed | Keep (already merged) |

Secrets the corrected `landing-deploy.yml` requires (agentmuxai/agentmux-landing):
- `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` — see Open Question OQ-A
- `A5AF_PACKAGES_TOKEN` — for `@a5af` GitHub Packages during `npm ci`
- `GITHUB_TOKEN` (auto) — `fetch-release.mjs` uses `gh` to read the public release

---

## 5. Open Questions / blocked prerequisites

| # | Question | Status |
|---|---|---|
| OQ-A | **Which static AWS key does `landing-deploy.yml` use?** RESOLVED: the landing page deploy (`fetch-release.mjs` S3 mirror + `cdk deploy` + CloudFront invalidation) was already a **manual local process** before this workflow existed. The CI workflow reuses **the same pre-existing AWS credential** that was used to run those commands by hand — it is added to the repo as `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` secrets by the owner. No new IAM principal is minted. (The `ci-release` IAM user I had prematurely created was deleted.) | **RESOLVED** |
| OQ-B | Does `fetch-release.mjs`'s `UNVERSIONED_NAMES` map still list `.deb`? The Linux build only produces AppImage. Harmless (no `.deb` asset to match) but stale. | Low priority — leave as-is |
| OQ-C | Should landing also auto-deploy on `main` push (copy changes) in addition to release dispatch? | Deferred (prior OQ5) |

---

## 6. What is NOT changing

- The `verify` job (5-location version-consistency guard) — correct as merged.
- `extract-changelog.sh` — correct as merged.
- Build jobs (Windows/Linux/macOS) — unchanged by this correction (macOS CEF patch
  gate is a separate workstream).
- `shared-infrastructure` weekly-analysts fix — already merged (#375) and deployed;
  out of scope here.

---

## 7. Sequencing once approved

1. Open fix PR on agentmuxai/agentmux: apply R1–R5 to `release.yml`. Add changeset.
2. Open fix PR on agentmuxai/agentmux-landing: apply L1 (revert OIDC).
3. Resolve OQ-A, then add secrets to both repos.
4. Dry-run: `workflow_dispatch` a non-tag run, or tag a patch (`v0.49.9`) to exercise
   end-to-end with MS Store left non-blocking.
