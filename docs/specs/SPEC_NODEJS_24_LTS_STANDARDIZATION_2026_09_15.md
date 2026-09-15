# Node.js 24 LTS standardization across agentmuxai repos

**Author:** Vmer
**Date:** 2026-09-15
**Status:** in progress — `agentmux`'s own gaps implemented (this PR); `agentmux-docs`
and `muxcode` fixes are open PRs, not yet merged. See §0 for current state.

---

## 0. TL;DR

Audited every `agentmuxai` repo cloned in this workspace (`agentmux`, `agentmux-docs`,
`muxcode`, `agentmux-mobile`) for their Node.js version pins. **`agentmux` itself is
already fully standardized on Node 24 LTS** — this was fixed as part of
`docs/reports/REPORT_FRESH_PC_ONBOARDING_AUDIT_2026_09_02.md`'s finding 2a (that report
recorded README.md/BUILD.md still saying "22 LTS" against a `.nvmrc`/CI baseline of 24;
both docs now correctly say 24). This PR closes the two remaining gaps inside
`agentmux` itself; two sibling repos have their own open PRs, not yet merged:

| Repo | Current state | Action needed |
|---|---|---|
| `agentmux` (main app + CI) | **24 everywhere** (`.nvmrc`, `package.json` engines, 8 CI workflow files, `check-toolchain.sh`, README.md, BUILD.md) | None — compliant |
| `agentmux` `docker/Dockerfile.agent-agentmux` | ✅ **Done, this PR** — bumped to `node:24-slim` | None — could not be rebuilt/tested locally (no Docker in the environment this was prepared in); first real verification is this PR's own `container-image.yml` build-and-push job |
| `agentmux` `.github/workflows/container-image.yml` | ✅ **Done, this PR** — added an explicit `actions/setup-node@v4` (`node-version: '24'`) step | None |
| `agentmux-docs` | CI pinned to **Node 22** (3 workflows); `package.json` has **no `engines` field** | Open PR (agentmux-docs#123): bumps CI to 24, adds `engines`. Not yet merged. |
| `muxcode` | `package.json` engines: **`>=20.0.0`**; no `.nvmrc`; **no CI at all** | Open PR (muxcode#2): bumps engines floor to 24, adds `.nvmrc`. Not yet merged. CI itself remains a separate open question. |
| `agentmux-mobile` | Flutter/Dart app — **no Node.js anywhere** in its toolchain or CI | Out of scope, documented so a future drift-checker doesn't flag it as a silent gap |

---

## 1. Method

Every claim below is from reading the actual file in the cloned repo, not inferred from
a changelog or another doc — greped for `"engines"`/`"node"` in every `package.json`,
`node-version`/`setup-node` in every CI workflow, `.nvmrc`/`.node-version` files, and
`FROM node` in every Dockerfile, across all four repos. `agentmux-mobile`'s CI workflows
were also greped for any Node reference at all (none found), confirming it's genuinely
out of scope rather than an oversight.

---

## 2. Findings, per repo

### 2.1 `agentmux` — compliant, with two edge cases

Confirmed consistent at Node 24:

```
.nvmrc                          -> 24.11.0
package.json "engines"          -> {"node": ">=24.11.0", "npm": ">=11"}
package.json "packageManager"   -> npm@11.6.2
8 CI workflow files              -> actions/setup-node@v4, node-version: 24 / '24'
                                    (release.yml x2, release-consistency.yml,
                                     docs-stale-sweep.yml, ci-pr.yml,
                                     ci-nightly-build.yml, build-macos.yml,
                                     build-linux.yml, build-windows.yml)
scripts/check-toolchain.sh       -> hard-fails below 24.11.0, with an .nvmrc pointer
README.md line 73                -> | **Node.js** | 24 LTS | Frontend build |
BUILD.md line 15                 -> | **Node.js** | v24 LTS | Frontend build (SolidJS/Vite) |
```

Two things outside that list were **not** on 24 — both fixed as part of this same PR:

- **`docker/Dockerfile.agent-agentmux`** (`FROM node:22-slim`) is not this repo's own
  build toolchain — it's the container image AgentMux ships for *running agent CLIs
  inside a sandboxed container* (the "container pane" feature). Its only Node-dependent
  step is `npm install -g @anthropic-ai/claude-code`. **✅ Bumped to `node:24-slim`.**
  It's a shipped, published image (`ghcr.io/agentmuxai/agent-claude`) rather than a dev
  dependency, so the real compatibility check (does the pinned
  `@anthropic-ai/claude-code@2.1.263` install and run cleanly under Node 24-slim/glibc)
  could not be done locally — no Docker was available in the environment this was
  prepared in. This PR's own `container-image.yml` build-and-push job is the first
  actual verification.
- **`.github/workflows/container-image.yml`** never called `actions/setup-node` — its
  one Node-touching step (`npm view @anthropic-ai/claude-code version`) ran on whatever
  Node shipped baked into the `ubuntu-latest` runner image, unpinned and drifting on
  GitHub's own schedule. **✅ Added an explicit `actions/setup-node@v4`
  (`node-version: '24'`) step**, matching the other 8 workflows.

### 2.2 `agentmux-docs` — CI on 22, no engines guard

```
.github/workflows/deploy.yml       -> node-version: 22
.github/workflows/pr-check.yml     -> node-version: 22
.github/workflows/deploy-prod.yml  -> node-version: '22'
package.json                       -> no "engines" field at all
```

This is the clearest real gap: an actively-deployed sister repo (auto-deploys to
production on every merge to `main`, per its own `CLAUDE.md`) is pinned two majors
behind the standard the main repo already enforces. `package.json`'s dependencies
(`astro ^7.1.4`, `typedoc ^0.28.19`, `sharp ^0.35.1`) don't pin a Node ceiling in a way
that would obviously break on 24, but this hasn't been verified by an actual build —
see §4.2.

### 2.3 `muxcode` — floor of 20, nothing enforced

```
package.json "engines" -> {"node": ">=20.0.0"}
.nvmrc                  -> none
CI                      -> none — no .github/workflows directory exists at all
```

`muxcode` is the most permissive of the four: its stated floor is Node 20 (four majors
behind 24), nothing pins a ceiling or a preferred version, and there is no CI to catch
drift even if one were added informally. Whoever builds/publishes it today is doing so
on whatever Node they happen to have locally.

### 2.4 `agentmux-mobile` — out of scope, confirmed not silently missed

Flutter/Dart app (`pubspec.yaml`-driven); its three CI workflows (`ci.yml`,
`release-android.yml`, `release-ios.yml`) were greped for any Node/`node-version`
reference and found none. Recorded here explicitly so this audit reads as "checked and
exempt," not "forgot to look."

---

## 3. What "Node 24 LTS" means here

`agentmux` already settled this: the working baseline is **`>=24.11.0`**, the exact
version pinned in its own `.nvmrc` and enforced by `check-toolchain.sh`. This spec
proposes reusing that same floor everywhere else, rather than inventing a second
standard — one version string to keep in sync across repos, not two.

---

## 4. Work breakdown

### 4.1 `agentmux` (two edge cases only — everything else already compliant) — ✅ DONE

1. ✅ Bumped `docker/Dockerfile.agent-agentmux`'s `FROM node:22-slim` to
   `FROM node:24-slim`. Could not rebuild/confirm
   `npm install -g @anthropic-ai/claude-code@2.1.263` locally — no Docker available
   in the environment this was prepared in. `container-image.yml`'s own
   build-and-push job is the first real verification; watch it on this PR before
   merging.
2. ✅ Added an explicit `actions/setup-node@v4` (`node-version: '24'`) step to
   `.github/workflows/container-image.yml`, matching the other 8 workflows, so CI's
   Node version is a checked-in fact rather than whatever `ubuntu-latest` happens to
   ship.

### 4.2 `agentmux-docs` — open PR, not yet merged: [agentmux-docs#123](https://github.com/agentmuxai/agentmux-docs/pull/123)

1. ✅ Bumped `node-version` from `22`/`'22'` to `'24'` in all three workflows
   (`deploy.yml`, `pr-check.yml`, `deploy-prod.yml`).
2. ✅ Added an `engines` field to `package.json` (`{"node": ">=24.11.0"}`), matching
   `agentmux`'s convention, so a local `npm install` on the wrong Node fails loudly
   instead of silently.
3. ✅ Verified locally under Node 24.12.0: `npm install` + `npm run build` clean, 58
   pages built. `npm run build:full` (typedoc + rustdoc) was not exercised — needs the
   `src/agentmux` submodule and a cargo toolchain, which CI already covers.
4. Still to do before merge: since `deploy.yml` deploys to production on every merge to
   `main` with no separate manual gate, verify the CI run and the post-deploy CSS-hash
   check in that repo's `CLAUDE.md` before considering it done.

### 4.3 `muxcode` — open PR, not yet merged: [muxcode#2](https://github.com/agentmuxai/muxcode/pull/2)

1. ✅ Bumped `package.json`'s `engines.node` floor from `>=20.0.0` to `>=24.11.0`.
2. ✅ Added a `.nvmrc` (`24.11.0`) for parity with `agentmux`.
3. ✅ Verified locally under Node 24.12.0: `npm install` and `npm run build` (`tsc`)
   both clean. `npm test` fails, but pre-existing and unrelated — the `test` script
   invokes `jest`, which was never added to `devDependencies`, so it was never
   actually installed. Not fixed as part of this change.
4. **Open question, not decided by this spec:** should `muxcode` get a CI workflow at
   all as part of this? Today nothing enforces its `engines` field even after bumping
   it — see §5.

### 4.4 `agentmux-mobile`

No action. Recorded as reviewed and exempt (§2.4).

---

## 5. Open questions for the repo owner

1. **Is `muxcode`'s Node 20 floor intentional** (e.g. meant to stay maximally portable
   as a CLI tool people install directly), or just never revisited? Bumping the
   `engines` floor changes who can `npm install` it.
2. **Should this spec's scope include adding CI to `muxcode`** (currently has none), or
   is that a separate, bigger piece of work than "fix the version pin"?
3. **Is the container image (`docker/Dockerfile.agent-agentmux`) meant to track the same
   Node version as the dev toolchain**, or is it intentionally decoupled since it only
   needs to run the Claude Code CLI, not build AgentMux itself?
4. Any other internal/private repos under `agentmuxai` not cloned into this workspace
   that should be included in this audit?
