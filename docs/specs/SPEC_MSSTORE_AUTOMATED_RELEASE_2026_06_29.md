# SPEC: Automated Microsoft Store Release (msstore CLI + GitHub Actions)

**Date:** 2026-06-29
**Status:** Draft — open questions resolved
**Author:** AgentX
**Related:** `scripts/package-msix.ps1`, `docs/specs/SPEC_MSIX_PACKAGING_2026_05_30.md`, `Taskfile.yml` (`release`, `package:release`, `package:msix`, winget update), `.github/workflows/`

---

## 1. Goal

Publish each release's MSIX to the Microsoft Store automatically, as one more
channel alongside the existing S3 (`dl.agentmux.ai/releases`) + winget paths —
**without** blocking those faster channels on Microsoft's certification.

Non-goals: changing how the MSIX is built (`package:msix` already produces the
correct artifact), reserving the app, or anything Microsoft gates manually.

## 2. Decision: which mechanism

**`msstore` (Microsoft Store Developer CLI) driven from a GitHub Actions
workflow.** AgentMux ships a **packaged MSIX** (`AgentMux_<semver>_x64.msix` from
`scripts/package-msix.ps1:156`), so:

- The `microsoft/store-submission` Action is **out** — it is the *unpackaged
  MSI/EXE* flow; wrong artifact class.
- The raw Store Submission REST API is **out** — it is token-handling we'd be
  reimplementing under the CLI for no benefit.
- `msstore` is the first-class MSIX path, is the only one with **gradual
  rollout** (set % / halt / finalize), and mirrors the existing winget step's
  "CLI + GitHub secret" shape (`Taskfile.yml:319`).

## 3. One-time manual prerequisites (cannot be automated)

These must be done **once, by a human**, before the workflow can succeed. They are
out of scope for the code change but in scope for the rollout checklist:

1. **Reserve the app name** in Partner Center (the API cannot create the app).
2. **Complete one full manual MSIX submission** — including the age-ratings
   (IARC) questionnaire — so a baseline submission exists.
3. **Entra association + service principal:** associate the Partner Center account
   with the org's Entra directory; create an app registration; assign it the
   **Manager** role in Partner Center. Capture `tenantId`, `sellerId`, `clientId`,
   `clientSecret`.
4. **Confirm identity match:** the reserved Store product's **Package/Identity
   Name** and **Publisher** must equal what `package-msix.ps1` bakes — it already
   asserts `PublisherDisplayName = "AgentMux"` and a fixed Publisher→hash invariant
   (`package-msix.ps1:37-42`, see `retro-msix-publisherdisplayname-regression-2026-05-30.md`).
   Record the Partner Center values in the MSIX spec so the two never drift.

## 4. Resolved open questions

### OQ1 — Build in CI, or reuse the released artifact?
**Reuse the already-built MSIX.** The release pipeline builds the portable +
MSIX once; rebuilding CEF in the Store workflow would cost 30+ min and risk
shipping *different bytes* than the S3/winget channels. **Resolution:** the
release process uploads `AgentMux_<ver>_x64.msix` to
`https://dl.agentmux.ai/releases/` (add it next to the existing `.msi`); the Store
workflow **downloads that exact file** and submits it. Identical bytes across all
channels, by construction.

### OQ2 — What triggers the workflow?
**A pushed release tag `v*`, with a `workflow_dispatch` manual fallback.** The
existing release flow produces a `chore: release vX.Y.Z` commit; tag that commit
`vX.Y.Z` (the release script/PR already implies this) and the Store workflow keys
off the tag. `workflow_dispatch` (with a `version` input) covers re-runs and the
first few supervised releases. **Not** triggered on every main push — Store
submission is deliberately decoupled from the fast channels.

### OQ3 — Signing?
**Submit the UNSIGNED MSIX.** The Store re-signs packages with the Store
certificate for distribution; you must not submit a self-signed one. The
`-Sign` flag in `package-msix.ps1` exists only for local `Add-AppxPackage`
testing and **must not** be used for the Store artifact. **Resolution:** the
uploaded/submitted artifact is the plain `makeappx`-packed, unsigned msix.

### OQ4 — Version source of truth?
`package.json.version` → MSIX `X.Y.Z.0` (already how `package-msix.ps1:62-64`
derives it). The **release-consistency invariant** (`.github/workflows/release-consistency.yml`)
already guarantees `package.json` ≡ `VERSION_HISTORY` ≡ `Cargo.toml`, so the tag,
the artifact filename, and the manifest version are the same value. The workflow
derives the version from the tag and **asserts** it matches the artifact filename
before submitting (fail fast on drift).

### OQ5 — Rollout policy?
**Expose a `rollout_percentage` input (default `100`).** For a normal release,
publish at 100%. For risky releases, submit at e.g. `10`, monitor, then finalize
with a follow-up manual run (`msstore` rollout update/finalize). Document the
halt/finalize commands; do not build auto-finalization in v1.

### OQ6 — Certification gate / failure handling?
**Submit + (optionally) poll, but never block the release.** The S3/winget
channels have already shipped by the time this runs, so a Store cert rejection is
a *notification*, not a release blocker. **Resolution:** the workflow submits,
polls submission status with a bounded timeout (e.g. 20 min), and on
timeout/failure exits with a **non-blocking** annotation (warning, not a required
check). Cert can take hours/days — the poll is best-effort visibility only.

### OQ7 — Idempotency / double-submit?
**Guard on "submission already exists for this version."** `msstore` refuses a
second in-flight submission; the workflow treats "a submission for X.Y.Z is
already pending/published" as success (idempotent re-run), not an error.

### OQ8 — Mixing API and UI edits?
**Hard rule, documented in the workflow header and the rollout checklist:** once a
submission is created via `msstore`, **never** edit it in the Partner Center UI —
doing so permanently locks the API out of that submission and can wedge it into an
error state. UI is read-only after automation is live.

### OQ9 — Where do the secrets live?
GitHub **repository (or org) secrets**: `MSSTORE_TENANT_ID`, `MSSTORE_SELLER_ID`,
`MSSTORE_CLIENT_ID`, `MSSTORE_CLIENT_SECRET`, `MSSTORE_APP_ID`. Same secret-store pattern the winget
step already uses for its token. The client secret rotates per Entra policy;
document the rotation owner.

### OQ10 — Coexistence with S3 + winget?
Additive. Order per release: S3 upload (source of truth) → winget update → **Store
submit**. Store is last and independent; its failure does not roll back the
others.

## 5. Proposed pipeline shape

```
task release            # changesets → version bump (+ VERSION_HISTORY), no commit
  → release PR merged   # chore: release vX.Y.Z, tag vX.Y.Z
  → release build job   # task package:release → task package:msix
        → upload AgentMux_<ver>_x64.msix to dl.agentmux.ai/releases/   (OQ1/OQ3)
  → msstore-release.yml (on: push tags v* / workflow_dispatch)
        1. derive version from tag; download AgentMux_<ver>_x64.msix
        2. assert filename version == tag version                       (OQ4)
        3. winget install "Microsoft Store Developer CLI"
        4. msstore reconfigure --tenantId … --sellerId … --clientId … --clientSecret …  (OQ9)
        5. msstore publish --… <msix>  (rollout deferred to manual post-publish step, see OQ5)
        6. poll submission status (bounded); annotate, never block      (OQ6/OQ7)
```

### 5.1 Proposed workflow (`.github/workflows/msstore-release.yml`)

> Authored here for review; **not committed live** until §3 prerequisites + §4.9
> secrets exist (it would fail otherwise). Exact `msstore` subcommand flags to be
> confirmed against the installed CLI version during implementation.

```yaml
name: Microsoft Store release
on:
  push:
    tags: ["v*"]
  workflow_dispatch:
    inputs:
      version:
        description: "Version to publish (e.g. 0.49.8)"
        required: true
      # rollout_percentage removed: msstore CLI does not accept a rollout flag on
      # publish; use `msstore submission rollout update/finalize` manually (OQ5).

jobs:
  publish:
    runs-on: windows-latest
    # [P1] Restrict job to the minimum required permissions (no repo write needed).
    permissions:
      contents: read
    steps:
      - name: Resolve version
        id: ver
        shell: pwsh
        # [P0] Do NOT interpolate github context expressions directly into run:
        # blocks — that enables script injection. Pass values through env: instead.
        env:
          INPUT_VERSION: ${{ github.event.inputs.version }}
          REF_NAME: ${{ github.ref_name }}
        run: |
          $v = $env:INPUT_VERSION
          if (-not $v) { $v = $env:REF_NAME.TrimStart('v') }
          "version=$v" >> $env:GITHUB_OUTPUT

      - name: Download MSIX artifact
        shell: pwsh
        # [P1] Do NOT interpolate step outputs directly into run: blocks.
        # Pass through env: to prevent injection.
        env:
          STEP_VERSION: ${{ steps.ver.outputs.version }}
        run: |
          $v = $env:STEP_VERSION
          $url = "https://dl.agentmux.ai/releases/AgentMux_${v}_x64.msix"
          Invoke-WebRequest $url -OutFile "AgentMux.msix"
          if (-not (Test-Path "AgentMux.msix")) { throw "MSIX not found at $url" }
          # [P1] OQ4: assert the MSIX manifest version matches the tag version,
          # not just that the file exists.
          $manifestVersion = (Get-AppxPackageManifest -Path (Resolve-Path "AgentMux.msix")).Package.Identity.Version
          # Manifest uses X.Y.Z.0 format; tag uses X.Y.Z
          if ($manifestVersion -ne "${v}.0") {
            throw "Manifest version '$manifestVersion' does not match expected '${v}.0'"
          }

      - name: Install msstore CLI
        # [P2] Pin the CLI version to avoid silent breaking changes from upstream updates.
        # Update this pin deliberately when testing a new CLI release.
        run: winget install --id "9P53PC5S0PHJ" --version 1.0.8.0 --accept-package-agreements --accept-source-agreements

      - name: Configure credentials
        # [P1] Secrets must not appear in CLI args (visible in process list).
        # Pass all secret values via env: and read them as $env:* to keep them
        # out of the process list (tenant/seller/client IDs can also leak paths).
        env:
          MSSTORE_TENANT_ID: ${{ secrets.MSSTORE_TENANT_ID }}
          MSSTORE_SELLER_ID: ${{ secrets.MSSTORE_SELLER_ID }}
          MSSTORE_CLIENT_ID: ${{ secrets.MSSTORE_CLIENT_ID }}
          MSSTORE_CLIENT_SECRET: ${{ secrets.MSSTORE_CLIENT_SECRET }}
        shell: pwsh
        run: |
          msstore reconfigure `
            --tenantId $env:MSSTORE_TENANT_ID `
            --sellerId $env:MSSTORE_SELLER_ID `
            --clientId $env:MSSTORE_CLIENT_ID `
            --clientSecret $env:MSSTORE_CLIENT_SECRET

      - name: Publish to Store
        env:
          MSSTORE_APP_ID: ${{ secrets.MSSTORE_APP_ID }}
        run: msstore publish "AgentMux.msix" --appId $env:MSSTORE_APP_ID --no-commit=false
        # OQ5: gradual rollout is managed post-publish via:
        #   msstore submission rollout update --rollout <pct>
        #   msstore submission rollout finalize
        # Document those commands in the runbook; do not wire them here in v1.

      - name: Report submission status (non-blocking)
        continue-on-error: true
        run: msstore submission status
```

## 6. Implementation steps (when greenlit)

1. **Release build:** extend the release artifact upload to also push
   `AgentMux_<ver>_x64.msix` to `dl.agentmux.ai/releases/` (the only change to
   existing tasks — one upload line, mirroring the `.msi`).
2. **Secrets:** add the five `MSSTORE_*` repo secrets (after §3).
3. **Workflow:** land `.github/workflows/msstore-release.yml`, initially
   `workflow_dispatch`-only; flip on the `v*` tag trigger once a supervised manual
   run succeeds end-to-end.
4. **Docs:** cross-link from `SPEC_MSIX_PACKAGING_2026_05_30.md` and record the
   Partner Center identity values next to the script's invariants (OQ-identity).

## 7. Risks

- **Cert latency/rejection** — async, Microsoft-controlled; mitigated by making
  the Store channel non-blocking (OQ6).
- **Identity drift** — manifest Publisher/Identity vs Partner Center reservation;
  mitigated by the existing script asserts + recording the Store values (§3.4).
- **Secret expiry** — Entra client secret rotation will silently break the
  workflow; assign a rotation owner and prefer the longest allowed lifetime or a
  federated-credential (OIDC) upgrade later.
- **UI/API lockout** — human edits a submission in Partner Center → API wedged;
  mitigated by the documented hard rule (OQ8).
- **CLI flag drift** — `msstore` subcommand surface evolves; pin the CLI version
  in the workflow and verify flags at implementation time.
