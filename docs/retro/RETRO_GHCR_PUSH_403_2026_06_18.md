# Retro: GHCR Push 403 — container build never shipped

**Date:** 2026-06-18  
**Severity:** Medium (build pipeline broken since first run; no prod image ever published)  
**Affected versions:** v0.45.0, v0.46.0, v0.46.4 (all container builds since workflow was introduced in #1347)

---

## What happened

Every run of the `Container Agent Image` workflow has failed at the push step with:

```
ERROR: failed to push ghcr.io/agentmuxai/agent-claude:<tag>:
  unexpected status from HEAD request …: 403 Forbidden
```

The image builds successfully (multi-arch, amd64 + arm64). The login to GHCR also succeeds. Only the push fails. The package `agentmuxai/agent-claude` has never been created on GHCR.

## Root cause

GitHub Actions has a two-layer permission model:

1. **Repo default** (`Settings → Actions → General → Workflow permissions`): caps what any workflow token can do.
2. **Workflow-level `permissions:` block**: can only be *more restrictive* than the default — it cannot elevate above it.

The repo default is **`read`**:

```
GET repos/agentmuxai/agentmux/actions/permissions/workflow
→ { "default_workflow_permissions": "read", ... }
```

The workflow correctly declares `permissions: packages: write`, but GitHub silently clamps the issued `GITHUB_TOKEN` to the `read` ceiling. The token authenticates fine (read access) but cannot push a new package (requires write). Since the package has never been created, `ghcr.io` returns 403 on every attempt.

## Why it wasn't caught sooner

The workflow was merged in PR #1347 and the first build triggered on v0.45.0 (2026-06-14) — 4 days before this retro. There were only 3 builds total, all failures. The login step passes with a green checkmark, which masked the real issue during casual inspection of the run summary.

## Fix

**Options A and B are blocked** — the repo's "Workflow permissions" setting is grayed out, meaning the org has locked the default at the org level. Repo admins cannot override it.

**Option C — implemented** (`commit 15cad7e3`): The repo already had a PAT with `write:packages` scope stored as secret `A5AF_PACKAGES_TOKEN`. Updated the login step to use it:

```yaml
- name: Log in to GitHub Container Registry
  uses: docker/login-action@v3
  with:
    registry: ghcr.io
    username: ${{ github.actor }}
    password: ${{ secrets.A5AF_PACKAGES_TOKEN }}   # was: secrets.GITHUB_TOKEN
```

No secret creation needed — the PAT was already present. Fix was a one-line workflow change.

## Verification after fix

```bash
# Re-trigger the last failed build
gh run rerun 27759875421

# Or push a new tag
git tag v0.46.4-retry && git push origin v0.46.4-retry

# Confirm package appears
gh api orgs/agentmuxai/packages/container/agent-claude
```

## Timeline

| Date | Event |
|------|-------|
| 2026-06-13 | Workflow introduced in PR #1347 (merged) |
| 2026-06-14 | First build triggered (v0.45.0) → 403 on push |
| 2026-06-15 | v0.46.0 build → same failure |
| 2026-06-18 | v0.46.4 build → same failure; root cause identified |

## Lessons

1. **Test push on the first PR** — the CI workflow was merged without a real push to GHCR ever succeeding. A manual `workflow_dispatch` run during review would have caught this immediately.
2. **Login ≠ push access** — the green checkmark on "Log in to GitHub Container Registry" is misleading; login only proves read auth, not write. The actual push capability should be smoke-tested separately.
3. **Default-read is the safer GitHub default** — it's correct to default to `read`, but it means new push workflows need the repo setting explicitly flipped. Document this in the PR that introduces any new push workflow.
