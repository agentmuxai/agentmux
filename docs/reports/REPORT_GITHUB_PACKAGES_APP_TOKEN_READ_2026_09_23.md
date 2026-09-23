# Report: agents cannot read `@a5af/*` from GitHub Packages

**Date:** 2026-09-23
**Status:** analysis
**Trigger:** `task release:patch` fails — `ERROR: bump-cli not installed` — and
`npm install -g @a5af/bump-cli` returns 403 for every agent identity available.
**Scope:** why the 403 happens, what it is *not*, and the options.

---

## 1. Summary

An earlier message from this agent asked a human to grant **`Packages:
Read-only`** to the `genericagentx-workflow` App. **That request was wrong.**
The permission was already granted, and granting it again changes nothing.

What the evidence supports instead: **GitHub Packages' npm registry does not
authorize GitHub App *installation tokens* for reads.** It authenticates them
happily — `whoami` succeeds — and then refuses the read with an error naming
the token type.

§2 is measured. §3 is inference, labelled as such. §4 is options.

## 2. Evidence

Every command below was run against live infrastructure on 2026-09-23. Tokens
are minted by `scripts/github-app-token.py <agent> <org>`.

### 2.1 The permission is already granted

```
GET /app/installations/162853501      (App 4994463, account: a5af)
→ "permissions": {
     "packages":      "write",     ← already present
     "contents":      "write",
     "pull_requests": "write",
     "issues":        "write",
     "actions":       "write",
     "workflows":     "write",
     "statuses":      "write",
     "metadata":      "read"
   }
```

### 2.2 The repository is in scope

```
GET /installation/repositories        → total_count: 127
                                      → includes a5af/dev-tools
```

Checked across both pages — an earlier single-page check missed it and would
have produced a second wrong conclusion.

### 2.3 The registry accepts the token, then refuses the read

```
GET https://npm.pkg.github.com/-/whoami
  Authorization: Bearer <app installation token>
→ 200  {"username":"genericagentx-workflow[bot]"}

GET https://npm.pkg.github.com/@a5af%2fbump-cli
  Authorization: Bearer <same token>
→ 403  {"error":"Permission installation not allowed to Read organization package"}
```

This pair is the core of the report: **authentication succeeds, authorization
fails**, and the error names *"installation"* explicitly rather than describing
a missing scope.

### 2.4 Same result on every identity available

| Token scoped to | Registry read |
|---|---|
| `a5af` (installation 162853501) | 403 |
| `a5afsys` (162964759) | 403 |
| `agentmuxai` (162853535) | 403 |
| anonymous | 401 |

The `agentmuxai`-scoped token gives a *different* message — *"the requested
installation does not exist"* — which is the ordinary wrong-org error and
should not be confused with the 403 above.

### 2.5 The REST packages API behaves differently again

```
GET /users/a5af/packages?package_type=npm        → 200  []        ← authorized, empty
GET /users/a5af/packages/npm/bump-cli            → 404  "Package not found."
GET /orgs/a5afsys/packages/npm/bump-cli          → 404
GET /orgs/agentmuxai/packages/npm/bump-cli       → 404
```

The list endpoint returning **200 with an empty array** matters: the App is
authorized to ask, and is shown nothing. That is consistent with package
visibility being withheld from this token type rather than with a missing
permission (which would be 403) or a wrong account (which would be 404 on the
list too).

### 2.6 How the package is published

`a5af/dev-tools/.github/workflows/publish.yml`:

```yaml
registry-url: 'https://npm.pkg.github.com'      # :114
NODE_AUTH_TOKEN: ${{ secrets.GITHUB_TOKEN }}    # :157
npm publish --access restricted                 # :180
```

Publishing uses the **Actions `GITHUB_TOKEN`** — an installation token bound to
the publishing repository — not a PAT. So writes work from inside Actions for
the repo that owns the package. Nothing in that arrangement grants read access
to a *different* App's installation token from outside Actions.

## 3. Inference — labelled, not proven

**Hypothesis:** GitHub Packages' npm registry accepts exactly two credential
kinds for reading a restricted package — the Actions `GITHUB_TOKEN` of a
workflow in the linked repository, and a classic PAT with `read:packages`. A
GitHub App installation token is authenticated but never authorized.

**Supporting:** §2.3's authenticate-then-refuse pair; the error naming
*"installation"*; §2.5's authorized-but-empty listing; the result being
identical across three installations with the permission granted and the repo
in scope (§2.1, §2.2, §2.4).

**Not proven:** no working PAT was available to demonstrate the positive case.
Producing one read with a classic PAT would settle it in a single command, and
until that is done this remains the best explanation rather than a fact.

**What it is not** — each ruled out by measurement:

- Not a missing App permission (§2.1)
- Not repository scoping (§2.2)
- Not a wrong organization (§2.4 — that error looks different)
- Not an unpublished package (§2.6 publishes it; and an absent package would
  not produce a permission error)
- Not an expired or malformed token (§2.3 `whoami` succeeds)

## 4. Options

Ranked by robustness, not by effort.

**A. Classic PAT with `read:packages`, stored in `services/infra`.**
Mirrors `gh-agent.sh`'s existing tier-3 PAT fallback, so the shape is already
familiar here. Cost: a long-lived credential, which is exactly what
`SPEC_GITHUB_APP_IDENTITY_MIGRATION_2026_09_18.md` moved away from — worth
scoping to `read:packages` only and nothing else.

**B. Make the consuming repo's own Actions token sufficient.**
If `bump-cli` is only needed in CI, a workflow in `agentmuxai/agentmux` cannot
read an `a5af`-owned package with its own `GITHUB_TOKEN` either. Linking the
package to a repo both orgs can reach, or publishing to a registry with
cross-org reads, would address this properly.

**C. Vendor the tool.** `bump-cli` is a small, config-driven bump utility;
`agentmux` already carries `scripts/bump-wrapper.sh` and
`scripts/sync-lockfile-version.mjs` around it. Vendoring removes a
cross-organization credential dependency from the release path entirely — the
release then depends on nothing outside this repo.

**D. Build from source at need.** What was done to unblock v0.56.12: clone
`a5af/dev-tools`, build `packages/bump-cli`, put it on `PATH`. Works, but see
§5 — the obvious path is blocked by a separate npm bug, so this is not a
one-liner and should not be the documented route.

## 5. Adjacent blocker: npm cannot install `bump-cli`'s dependency set

Independent of permissions, and worth its own fix.

```
npm install                 (npm 10.9.9, node 22.22.1)
→ npm error Cannot read properties of null (reading 'edgesOut')
```

Bisected:

| Install | Result |
|---|---|
| `chalk` alone | ok |
| `commander` alone | ok |
| `typescript` alone | ok |
| `@types/node` alone | ok |
| `vitest` alone | ok |
| **all five together** | **fails** |
| all five **added one at a time** | ok |

So it is an arborist resolution bug on the combined graph, not a bad
dependency, not workspace-related, and not stale state — it reproduces in an
empty directory from the manifest alone. Installing incrementally is a reliable
workaround and is how v0.56.12's `bump-cli` was built.

## 6. Recommended next step

Run one command with a classic PAT that has `read:packages`:

```bash
npm --userconfig <(printf '@a5af:registry=https://npm.pkg.github.com\n//npm.pkg.github.com/:_authToken=<PAT>\n') \
    view @a5af/bump-cli version
```

- **Returns a version** → §3's hypothesis is confirmed; adopt option A or C.
- **Still 403** → the hypothesis is wrong and the cause is elsewhere; this
  report should be reopened rather than built upon.

Either outcome is worth more than further reasoning from the same evidence.

## 7. Note on how this report came to exist

The first diagnosis here was wrong: a human was asked to grant a permission
that was already granted, on the strength of an error message that *sounded*
like a missing permission. It was corrected only by reading the installation's
actual granted permissions rather than inferring them.

That is the same failure recorded in
`retro-agent-identity-redesign-and-release-toolchain-2026-09-23.md` §2 —
confident claims about systems nobody had queried. §3 above is therefore
labelled as inference and given a single falsifying test, rather than being
stated as a conclusion.
