# INCIDENT 2026-09-16 — Linux release/nightly builds fail at "Install fpm" (`Gem::FilePermissionError`); three releases never published

**Status:** root cause fixed on `main` (commit `7cd2702`, 2026-09-17 03:11 UTC); the
three release tags that failed during the window (v0.56.1, v0.56.2, v0.56.3) have
**not** been re-run or republished as of this writing — the latest asset users can
actually download is still **v0.56.0** (published 2026-09-15T19:35:43Z).

**Severity:** High for release publishing (3 releases silently failed to reach
users over ~14 hours); no impact on `main`'s health — `ci-pr.yml` doesn't touch
`fpm` at all.

**Trigger for this investigation:** a GitHub notification email about a failed
Linux build; requested to check `agentmuxai/agentmux`'s recent CI/release state.
Also checked `a5af/shared-infrastructure` per the same request — its 10 most
recent commits (2026-09-10 through 2026-09-15) are unrelated docs/analyst-script
fixes, nothing touching CI or this failure.

## 1. What failed

Every Linux packaging job (`Build — Linux AppImage / Linux x86_64 — AppImage`,
used by both `release.yml` and `ci-nightly-artifacts.yml`) died at the same step:

```
Run gem install --no-document fpm
ERROR:  While executing gem ... (Gem::FilePermissionError)
    You don't have write permissions for the /var/lib/gems/3.0.0 directory.
##[error]Process completed with exit code 1.
```

Unqualified `gem install` was writing into the GitHub-hosted runner's **system**
Ruby gem directory (root-owned). That had worked as recently as 2026-07-19 (this
job's prior run); nothing in the repo's own build scripts changed between then
and now — the runner-image's system Ruby gem-dir ownership is the most likely
external cause (per the fix commit's own investigation, not independently
re-verified here).

## 2. Runs affected

| Run | Trigger | Started (UTC) | Conclusion | Linux job |
|---|---|---|---|---|
| `35089604010` | `ci-nightly-artifacts.yml`, schedule | 2026-09-16 11:18:29 | **failure** | died at `Install fpm`, 1m07s |
| `35067243576` | `release.yml`, push `v0.56.1` (#3251) | 2026-09-16 07:11:04 | **failure** | same |
| `35129708327` | `release.yml`, push `v0.56.2` (#3273) | 2026-09-16 17:41:04 | **failure** | same |
| `35153731188` | `release.yml`, push `v0.56.3` (#3283) | 2026-09-16 21:41:05 | **failure** | died at `Install fpm`, 1m16s — this is almost certainly the run behind last night's email |
| `35215718221` | `ci-nightly-artifacts.yml`, schedule | 2026-09-17 11:26:47 | success | Linux job **passed**, 18m44s, produced `linux-appimage`/`linux-deb`/`linux-rpm`/`linux-tarball` — first green run after the fix |

Windows and macOS legs were unaffected throughout (both build via their own jobs
in the same workflow; neither uses `fpm`).

Each `release.yml` run's `Publish release` job never ran (`skip`ped — it depends
on all three platform builds succeeding), so **v0.56.1, v0.56.2, and v0.56.3 were
never published as GitHub releases.** Confirmed: `GET
/repos/agentmuxai/agentmux/releases/tags/v0.56.3` → 404;
`/repos/agentmuxai/agentmux/releases/latest` → still `v0.56.0`
(published 2026-09-15T19:35:43Z, assets present for all 5 platforms/formats).

## 3. Fix

Commit `7cd2702` (AgentA, 2026-09-17T03:11:05Z, pushed directly to `main` — no PR,
per this repo's documented CI-fix exception) changed `build-linux.yml`'s step to:

```yaml
- name: Install fpm
  run: sudo gem install --no-document fpm
```

`ci-nightly-artifacts.yml` reuses the same job definition (it has no `gem
install` line of its own), so the fix covers both the nightly and release paths
— confirmed by run `35215718221` above going green the same morning.

## 4. What's still open

1. **v0.56.1, v0.56.2, and v0.56.3 were never published.** Whatever those three
   version bumps were meant to ship is currently unavailable to users; `v0.56.0`
   is still the newest thing anyone can download. Re-running the existing failed
   `release.yml` runs may not pick up `main`'s fix depending on how those runs
   resolve workflow file versions (release runs check out the tag's own commit,
   not `main`) — this needs to be confirmed, not assumed, before relying on
   "just re-run it" as the fix. Not attempted here; flagging as the next action
   for whoever owns releases.
2. **No changesets/release-content review done here** — this investigation only
   confirms the build/publish mechanics failed, not what v0.56.1–v0.56.3
   contained or whether they should all still ship as-is once re-run.
3. No incident doc existed for this before now; this is the first.

## 5. Evidence

- `gh run list --repo agentmuxai/agentmux --workflow "<name>"` for `release.yml`,
  `ci-nightly-artifacts.yml`, `ci-nightly-build.yml`, `build-linux.yml`,
  `Nightly Release — Auto-Publish Pending Version Bump (manual fallback)`.
- `gh api repos/agentmuxai/agentmux/actions/jobs/104988120534/logs` (raw failure
  log, run `35153731188`'s Linux job).
- `gh api repos/agentmuxai/agentmux/commits/7cd2702` (fix commit + message).
- `gh api repos/agentmuxai/agentmux/contents/.github/workflows/build-linux.yml`
  (confirms `sudo` is live on `main`).
- `gh api repos/agentmuxai/agentmux/releases/tags/v0.56.3` → 404;
  `.../releases/latest` → `v0.56.0`.
- `gh api repos/a5af/shared-infrastructure/commits` (10 most recent, checked for
  relevance — none found).
