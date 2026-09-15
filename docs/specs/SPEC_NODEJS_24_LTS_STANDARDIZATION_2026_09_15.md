# Node.js 24 LTS standardization across agentmuxai repos

**Author:** Vmer
**Date:** 2026-09-15
**Status:** draft — audit complete, no code changed yet

---

## 0. TL;DR

Audited every `agentmuxai` repo cloned in this workspace (`agentmux`, `agentmux-docs`,
`muxcode`, `agentmux-mobile`) for their Node.js version pins. **`agentmux` itself is
already fully standardized on Node 24 LTS** — this was fixed as part of
`docs/reports/REPORT_FRESH_PC_ONBOARDING_AUDIT_2026_09_02.md`'s finding 2a (that report
recorded README.md/BUILD.md still saying "22 LTS" against a `.nvmrc`/CI baseline of 24;
both docs now correctly say 24). Two gaps remain even inside `agentmux`, and two sibling
repos are not on 24 at all:

| Repo | Current state | Action needed |
|---|---|---|
| `agentmux` (main app + CI) | **24 everywhere** (`.nvmrc`, `package.json` engines, 8 CI workflow files, `check-toolchain.sh`, README.md, BUILD.md) | None — compliant |
| `agentmux` `docker/Dockerfile.agent-agentmux` | `FROM node:22-slim` | Bump to `node:24-slim`, or explicitly decide it's exempt (see §2.1) |
| `agentmux` `.github/workflows/container-image.yml` | No `actions/setup-node` step at all — runs `npm view` on whatever Node ships on `ubuntu-latest` | Low priority; pin explicitly for reproducibility (see §2.1) |
| `agentmux-docs` | CI pinned to **Node 22** (3 workflows); `package.json` has **no `engines` field** | Bump CI to 24, add `engines` |
| `muxcode` | `package.json` engines: **`>=20.0.0`**; no `.nvmrc`; **no CI at all** | Bump engines floor to 24, add `.nvmrc`; CI is a separate open question |
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

Two things outside that list are **not** on 24, and are arguably different in kind from
the rest — worth a deliberate decision rather than a blind bump:

- **`docker/Dockerfile.agent-agentmux`** (`FROM node:22-slim`) is not this repo's own
  build toolchain — it's the container image AgentMux ships for *running agent CLIs
  inside a sandboxed container* (the "container pane" feature). Its only Node-dependent
  step is `npm install -g @anthropic-ai/claude-code`. Bumping it to `node:24-slim` is
  probably right for consistency and to stay ahead of Node 22's EOL clock, but it's a
  shipped, published image (`ghcr.io/agentmuxai/agent-claude`) rather than a dev
  dependency, so it deserves its own compatibility check (does the pinned
  `@anthropic-ai/claude-code@2.1.263` install and run cleanly under Node 24-slim/glibc)
  rather than a blind find-replace.
- **`.github/workflows/container-image.yml`** never calls `actions/setup-node` — its one
  Node-touching step (`npm view @anthropic-ai/claude-code version`) runs on whatever
  Node ships baked into the `ubuntu-latest` runner image, which is unpinned and drifts
  on GitHub's own schedule. Low-risk today (it only queries the npm registry, doesn't
  build anything), but it's the one place in this repo where "what Node version is CI
  actually using" isn't a checked-in fact. Worth an explicit `setup-node` step for the
  same reproducibility reason the other 8 workflows already have one.

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

### 4.1 `agentmux` (two edge cases only — everything else already compliant)

1. Decide, and if yes, bump `docker/Dockerfile.agent-agentmux`'s `FROM node:22-slim` to
   `FROM node:24-slim`; rebuild the image locally and confirm
   `npm install -g @anthropic-ai/claude-code@2.1.263` still succeeds and the CLI still
   runs under the new base before publishing.
2. Add an explicit `actions/setup-node@v4` (`node-version: '24'`) step to
   `.github/workflows/container-image.yml`, matching the other 8 workflows, so CI's
   Node version is a checked-in fact rather than whatever `ubuntu-latest` happens to
   ship.

### 4.2 `agentmux-docs`

1. Bump `node-version` from `22`/`'22'` to `'24'` in all three workflows
   (`deploy.yml`, `pr-check.yml`, `deploy-prod.yml`).
2. Add an `engines` field to `package.json` (`{"node": ">=24.11.0"}`), matching
   `agentmux`'s convention, so a local `npm install` on the wrong Node fails loudly
   instead of silently.
3. Run `npm run build:full` locally under Node 24 before merging — this repo's own
   `CLAUDE.md` flags `build:full` (typedoc + rust-docs + astro build) as the real
   production build; a plain `astro dev` working is not sufficient evidence.
4. Since `deploy.yml` deploys to production on every merge to `main` with no separate
   manual gate, land this on a branch and verify the CI run (`gh run list --repo
   agentmuxai/agentmux-docs --workflow deploy.yml`) and the post-deploy CSS-hash check
   in that repo's `CLAUDE.md` before considering it done.

### 4.3 `muxcode`

1. Bump `package.json`'s `engines.node` floor from `>=20.0.0` to `>=24.11.0`.
2. Add a `.nvmrc` (`24.11.0`) for parity with `agentmux`.
3. Run `npm test` (Jest, per `package.json`) and `npm run build` (`tsc`) locally under
   Node 24 to confirm nothing in the dependency set (`@anthropic-ai/sdk`,
   `@modelcontextprotocol/sdk`, `commander`, `openai`) breaks.
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
