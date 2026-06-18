# Scripts Folder Cleanup & Release System Analysis — 2026-06-18

## Summary

Two separate audits: (1) orphaned diagnostic scripts in `scripts/`, and (2) three bugs in the bump/release pipeline that caused repeated reagentx P0/P2 failures on PR #1548. Both are fixed in this PR.

---

## Part 1 — Orphaned scripts removed

### Removed (8 files, ~1335 lines)

| File | Last commit | Why removed |
|------|-------------|-------------|
| `cdp-sbflush-probe.js` | #1375 (terminal scrollbar) | Investigation closed — fix shipped |
| `cdp-gap-probe.js` | #1372 (terminal margin) | Investigation closed — fix shipped |
| `cdp-term-smoke.mjs` | #1370 (xterm-6 migration) | Investigation closed — xterm-5 retired |
| `bench-cdp.mjs` | #850 (AuthFlowController) | Ancient (~18 months), no callers |
| `capture-trace.cjs` | #850 (AuthFlowController) | Ancient, no callers |
| `smoke-test-portable.cjs` | #850 (AuthFlowController) | Superseded by CI |
| `verify-typing-fix.cjs` | #850 (AuthFlowController) | One-time typing bug verification |
| `redact-pii.mjs` | #990 (transcript recovery) | One-off PII redaction job |

### Kept — `gen-seed.js`

Initially flagged as orphaned (also from #850), but reagentx caught the error: `gen-seed.js` generates `agentmux-srv/agent-seed.json`, which is baked into the binary via `include_str!` at `agent_seed.rs:115`. Removing it would leave the seed with no regeneration path.

### All shell/PS1 scripts kept

All 22 `.sh` / 2 `.ps1` files are either wired into `Taskfile.yml` or are intentional standalone utilities (`import-agents.sh`, `wipe-old-data-dirs.sh`, `test-splash.sh`).

---

## Part 2 — Release pipeline bugs fixed

Three bugs surfaced during the v0.46.3 release (PR #1548) that caused reagentx P0/P2 rejections and repeated force-pushes.

### Bug 1 — `release.sh` didn't stage all version files (P0)

**Symptom:** After `bash scripts/release.sh`, `package.json`, `Cargo.toml`, and `Cargo.lock` were updated on disk but not staged. Running `git commit` produced a release commit with `VERSION_HISTORY.md` and `package-lock.json` bumped but the other three files still at the old version — reagentx caught the mismatch immediately.

**Root cause:** `scripts/release.sh` line 180 only staged `"$HISTORY" package-lock.json`.

**Fix:** Stage all five version-bearing files:
```bash
git add -- "$HISTORY" package.json Cargo.toml Cargo.lock package-lock.json
```

### Bug 2 — `bump-wrapper.sh` caused broad lockfile churn (P2)

**Symptom:** `package-lock.json` had hundreds of changed lines beyond the version bump — `"dev": true` entries reclassified as `"devOptional"` / `"peer"`, `react`/`loose-envify` pruned. These are npm metadata normalization artifacts from the machine's local npm version differing from whatever wrote the committed lockfile.

**Root cause:** `bump-wrapper.sh` ran `npm install --package-lock-only --ignore-scripts` to sync the lockfile. This triggers a full lockfile regeneration, which normalizes peer/dev metadata according to the current npm version — producing large noisy diffs when npm versions differ between machines.

**Fix:** Replace with a targeted Node.js edit that updates only the two version fields in `package-lock.json` without touching anything else:
```js
const lock = JSON.parse(fs.readFileSync('package-lock.json', 'utf8'));
lock.version = ver;
if (lock.packages && lock.packages['']) lock.packages[''].version = ver;
```

### Bug 3 — No `task release:patch` shorthand (agent UX)

**Symptom:** Multiple agents tried `task release --as patch`, which fails with `unknown flag: --as` because `task` parses `--as` as its own flag before forwarding `{{.CLI_ARGS}}`. The correct invocation (`bash scripts/release.sh --as patch`) is non-obvious and not in the task description.

**Fix:** Added `task release:patch` and `task release:minor` as explicit Taskfile shortcuts. Updated `task release` description to mention them.

---

## Correct release workflow (post-fix)

```bash
# 1. Create branch
git checkout -b <agent>/release-vX.Y.Z

# 2. Run release (auto-detects bump type from changesets):
task release

# 3. Or force a specific bump type:
task release:patch    # force patch regardless of feat/fix changesets
task release:minor    # force minor

# 4. Commit (ALL version files are now staged automatically by the script):
git commit -m "chore: release vX.Y.Z"
git push -u origin <branch>

# 5. Open PR — release PRs must contain ONLY:
#    - changeset deletions
#    - VERSION_HISTORY.md entry
#    - package.json, Cargo.toml, Cargo.lock, package-lock.json version bumps
#    Nothing else (no cleanup, no feature code, no analysis docs).
```

**Never use `scripts/bump-wrapper.sh` directly for a release** — it bumps versions but does not consume changesets or update `VERSION_HISTORY.md`. It is an internal helper called by `release.sh`.
