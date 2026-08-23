# Nightly Release Automation — Auto-Publish the Latest Pending Version Bump

**Date:** 2026-08-23 (revised twice same-day — see §0 and §0.1)
**Status:** Draft — spec only, no implementation yet
**Scope:** agentmuxai/agentmux (new `.github/workflows/nightly-release.yml`,
tag-and-push only — no email code lives here). Landing page
(`agentmuxai/agentmux-landing`) is now **in scope as a consequence**, not as
new work — see §6. Notification is delivered by extending the existing
`gh-reporter` Lambda in **a5af/shared-infrastructure** (cross-repo,
CDK-deployed) — see §5.5. No new email secret/provider anywhere.

---

## 0. Revision note — the model changed after first draft

The first draft of this spec (see git history of this file, or §9 for a
summary) proposed a classic "nightly build channel": tag `main`'s raw HEAD
every night as `nightly-YYYYMMDD`, publish it as a GitHub prerelease, and
explicitly **do not** touch the landing page, on the theory that HEAD is
unvetted and shouldn't reach the public download page.

The repo owner corrected the framing: **this repo's release process already
produces a fully reviewed, CI-gated, merged-to-`main` version-bump commit
(`chore: release vX.Y.Z`) ahead of actually being tagged** — see this exact
session's own history, where `task release -- --as patch` was run, the
resulting commit was opened as PR #2767, reviewed, CI-gated, and merged, and
only *then*, as a separate manual step, tagged and pushed. Nightly's actual
job, as scoped by the repo owner, is **not** "snapshot whatever's on `main`
tonight" — it's **"notice that a real, already-approved release commit is
sitting on `main` untagged, and do the tag-and-push step a human currently
does by hand."**

That reframing changes almost everything below:

- There is no new versioning scheme — nightly publishes the *real* `vX.Y.Z`
  tag the pending bump already computed, not a synthetic date-based one.
- There is no unvetted-content risk — the commit being tagged already
  passed PR review + CI as a precondition of being on `main` at all, same
  bar as any manually-tagged release.
- The landing-page dispatch is therefore **correct and wanted**, not a
  risk to guard against — it's the same `repository_dispatch` `release.yml`
  already fires for a manual tag push, now just triggered by automation
  instead of a human running `git tag && git push`.
- Most of the "channel separation" machinery industry nightly builds use
  (§2) doesn't apply, because agentmux isn't shipping a second, lower-trust
  artifact type — it's automating one manual step (tagging) in its existing
  single-trust-tier release pipeline.

§2's survey and its channel-separation lessons are kept below as useful
background (and because a couple of its findings — e.g. §5.2's collision
caution — remain implicitly true and worth understanding), but the design
in §5 onward reflects the revised, repo-owner-confirmed model, not the
original one.

## 0.1 Second revision — one email, not a new one

The §0 revision still proposed **new** notification infrastructure: a
self-contained sender in `nightly-release.yml` via Resend, gated behind new
`NIGHTLY_EMAIL_API_KEY`/`NIGHTLY_EMAIL_TO`. The repo owner corrected this
too: **fold the results into the existing "GitHub Nightly Report" email**
(the one `gh-reporter` already sends nightly for CI health) — "that way
it's in one email" — rather than standing up a second sender/provider for
a second nightly email arriving in the same inbox.

This is a materially better fit than it first appears, not just a
preference: `gh-reporter` (confirmed by direct code inspection,
`a5af/shared-infrastructure`) already has (a) standing GitHub App
credentials scoped to `agentmuxai/agentmux` capable of querying *any* new
workflow or the releases API with no new auth to wire up, (b) proven SES
sending with a known-good sender identity, and (c) a section-based email
template specifically designed for exactly this kind of addition (see
§5.5). The result is that agentmux's own `nightly-release.yml` gets
*simpler* than the §0 design, not more complex — it no longer needs an
email step at all; its entire job shrinks to detect-tag-push (§4.3), full
stop. `gh-reporter` discovers the outcome on its own next poll, entirely
from data it can already fetch itself.

The two clarifications that came with this request are already correctly
captured in the design and are restated here for clarity, not as new
decisions:

- **No pending bump → no publish that night**, confirmed unchanged: §4.3's
  skip condition already handles this — if the latest `chore: release`
  commit is already tagged, nightly does nothing.
- **Two or more bumps landing the same day → only the most recent gets
  tagged and released**, confirmed unchanged: §4.3 already only ever looks
  at the single most recent `chore: release` commit; an earlier, superseded
  bump commit is never independently tagged, exactly like the manual
  process today (nobody would go back and tag an old bump after a newer
  one already merged).

---

## 1. Problem statement

### 1.1 What actually happens today (confirmed this session)

1. Changesets accumulate in `.changesets/*.md` as PRs land on `main`.
2. Periodically, a human or agent runs `task release -- --as <bump>`, which
   consumes all pending changesets, computes the version bump, updates the
   five version-locations (`package.json`, `Cargo.toml`,
   `Cargo.lock`, `package-lock.json`, `VERSION_HISTORY.md`), and stages —
   but does not commit — the result.
3. That gets committed as `chore: release vX.Y.Z`, pushed as a PR, reviewed,
   CI-gated, and squash-merged to `main` — e.g. PR #2767 earlier this
   session.
4. **Only then**, as a distinct, easy-to-forget manual step, does someone
   run `git tag vX.Y.Z <sha> && git push origin vX.Y.Z` — which is the
   *only* thing that actually triggers `release.yml` (builds, GitHub
   Release, landing-page dispatch) and `container-image.yml` (Docker image).

Step 4 is the gap. A `chore: release vX.Y.Z` commit can sit on `main`,
fully merged and reviewed, for an arbitrary amount of time before anyone
remembers to tag it — nothing in the repo currently notices or reminds
anyone. Nightly automation closes exactly that gap.

### 1.2 Goals

- Every night (see §4 for cadence), check whether the most recent
  `chore: release vX.Y.Z` commit on `main` has been tagged yet. If not,
  tag it and push — letting the **existing, unmodified** `release.yml` /
  `container-image.yml` pipeline do everything it already does for a
  manual tag push, landing-page deploy included.
- Skip cleanly, silently, on a night where the last pending bump is already
  tagged (nothing new to publish) — this will be the common case on quiet
  nights or immediately after a manual tag.
- Still gate on nightly CI health (§5.6) before auto-tagging — don't
  publish off the back of a red nightly run, even though the commit itself
  already passed its own PR-level CI.
- A notification recapping what just got auto-published — version,
  changelog (sourced from the `VERSION_HISTORY.md` entry `task release`
  already wrote), contributors, and links to the GitHub Release and the
  now-updated landing page — delivered as a **new section inside the
  existing "GitHub Nightly Report" email** (`gh-reporter`,
  `a5af/shared-infrastructure`), not a second email (§5.5, per §0.1).
- Reuse 100% of the existing `release.yml` → `container-image.yml` →
  landing-page pipeline, *and* 100% of the existing nightly-email
  infrastructure. Nightly's own new code in `agentmuxai/agentmux` is
  limited to the "detect an untagged pending release + tag it" step
  (§4.3) — no email code lives in this repo at all.

### 1.3 Non-goals

- **Not a second, lower-trust build channel.** Every previous draft's
  "alpha/dogfood channel, no stability promise" framing is dropped — what
  nightly publishes *is* the stable release, just tagged on a schedule
  instead of by hand. There is exactly one trust tier here, matching
  whatever bar `chore: release` PRs already clear today.
- **Not a version-bump/changeset-consumption feature.** Nightly never runs
  `task release`, never touches `.changesets/*.md`, never decides a bump
  level. That stays a human/agent decision, exactly as it is today — see
  §3.3. Nightly only publishes a bump someone already decided to make and
  already got merged.
- **Not a new CI gate.** Same principle as the first draft: reuse
  `ci-nightly-build.yml`'s existing pass/fail bar, don't invent a stricter
  one (§5.6).
- **Not a replacement for `gh-reporter`'s existing CI-health email.** Same
  reasoning as before (§5.5) — different audience, different content.

---

## 2. Industry survey — kept for context, only partially applicable now

| Project | Versioning | Cadence / skip logic | Channel separation | Retention |
|---|---|---|---|---|
| **VS Code Insiders** | Date+build-id embedded in version, e.g. `1.107.0-<buildid>` ([Insiders Release Notes](https://github.com/microsoft/vscode/wiki/Insiders-Release-Notes), [GH #279435](https://github.com/microsoft/vscode/issues/279435)) | Nightly from `main`, always builds (dev team dogfoods it live) | Separate installer/binary identity from Stable, own auto-update channel, progressive rollout | Old builds not listed on the download page; must be fetched from update-server API by exact version |
| **Rust nightly** | `nightly-YYYY-MM-DD` toolchain name, date-suffixed archive folder ([rustup channels doc](https://rust-lang.github.io/rustup/concepts/channels.html)) | Built daily from master; a date can have **no** working build (not every night ships) | Fully separate `rustup toolchain` install target from stable/beta; `rustc --version` embeds the date | Dated archives kept indefinitely per date (`static.rust-lang.org/dist/<date>/`) |
| **Electron `electron-nightly`** | Ships under a **separate npm package name**, decoupled from the real semver line entirely ([Electron Versioning docs](https://www.electronjs.org/docs/latest/tutorial/electron-versioning)) | main branch itself never carries real version numbers until a release branch cuts | Total package-identity separation | N/A (npm registry, effectively unbounded) |
| **React Canary** | `<version>-canary-<commit-hash>-<date>` | Built from every commit to main that passes CI | Separate npm dist-tag (`canary`) from `latest` | Effectively unbounded (npm) |

**Why agentmux's design (§5) doesn't copy these:** every project above
builds and ships *raw trunk* every night — there is no equivalent, upstream
"this specific commit was already reviewed and approved as a release" step
in their process, so they *need* a separate lower-trust channel/versioning
identity to avoid confusing an automated trunk snapshot with a deliberate
release. **agentmux already has that upstream step** (§1.1 step 2-3) — the
`chore: release` PR *is* the deliberate-release decision, made by a human
or agent, reviewed like any other PR. Nightly here isn't building trunk,
it's finishing the publish of a decision that was already made. That's a
meaningfully different problem than what any of the surveyed projects
solve, which is why this spec's design diverges from all four rather than
picking one to imitate.

The one lesson from the survey that **does** still apply: §5.2 explains why
reusing the *real* `vX.Y.Z` tag format (rather than inventing a nightly-only
one) is safe here specifically *because* nightly and manual tagging now
publish literally the same kind of artifact — there's no second namespace
to keep separated from a first, so the collision risk the original draft
worried about doesn't arise in the new design at all.

---

## 3. This repo's existing infrastructure (what to reuse)

### 3.1 `ci-nightly-build.yml` — still the quality gate

`.github/workflows/ci-nightly-build.yml` (`schedule: '0 7 * * *'` UTC +
`workflow_dispatch`) runs `cargo build --release --workspace` +
`cargo test --workspace` + `vitest` across windows/ubuntu/macos, Windows
required/blocking, Linux/macOS `continue-on-error`. This remains the
trigger-chaining and go/no-go signal for nightly-release (§5.6) — unchanged
from the first draft's reasoning. One added nuance now that the *target*
of publishing is a specific (possibly not-HEAD) commit: this gate checks
whatever commit `ci-nightly-build.yml` most recently tested (typically
`main`'s current tip), which may be *ahead of* the release commit being
tagged if further un-bumped work has landed since. That's fine and
intentional — branch protection on `main` already required the release
commit itself to pass its own PR-level CI before merge (confirmed this
session: PR #2767 required `check --tests + test (windows-latest)`,
`(ubuntu-latest)`, and `vitest` green, plus an approving review, before
merge was possible). `ci-nightly-build.yml`'s gate is an *additional*
broader-matrix sanity check (it's the only workflow that runs the macOS
leg) on top of that already-satisfied bar, not the sole gate.

### 3.2 `release.yml` / `container-image.yml` — reused entirely unmodified

This is the biggest simplification versus the first draft: nightly no
longer needs to mirror, wrap, or re-invoke `release.yml`'s build jobs at
all. Its only job is to push a tag matching the exact pattern
`release.yml` (`v[0-9]+.[0-9]+.[0-9]+`) and `container-image.yml` (`v*`)
already listen for. Once that tag lands, **both existing workflows fire
exactly as they do for a manual tag push** — same builds, same GitHub
Release creation, same `repository_dispatch` to `agentmux-landing`. Nightly
contributes zero new packaging, signing, or publishing code anywhere.

### 3.3 Changesets — nightly never touches them, full stop

Unlike the first draft (which had to carefully avoid *consuming* changesets
while still wanting *content* from them), the revised design doesn't read
changesets at all. By the time nightly runs, `task release` already
consumed and deleted them, and already wrote their content into
`VERSION_HISTORY.md`'s new top section as part of the `chore: release`
commit (confirmed structure, see §5.5). Nightly's email sources from that
file, not from `.changesets/`.

### 3.4 No email infrastructure in this repo — and none is needed (§0.1)

`grep -rliE "smtp|sendgrid|resend|nodemailer|aws-ses|mailgun|postmark"`
across `agentmuxai/agentmux` returns nothing relevant — this repo has zero
first-party email code, and per §0.1 it should stay that way.
`a5af/shared-infrastructure`'s `gh-reporter` Lambda already sends a nightly
CI-health email over SES (proven, zero-bounce per
`SPEC_UNIFIED_RELEASE_CICD_2026_06_29.md` OQ3), already holds standing
GitHub App credentials scoped to `agentmuxai/agentmux`, and — confirmed by
direct code inspection — has a section-based template built for exactly
this kind of addition. §5.5 extends it directly rather than adding a
second sender anywhere.

### 3.5 `VERSION_HISTORY.md` structure (confirmed by direct inspection)

```
# AgentMux Version History

## 0.55.21 — 2026-08-22
- feat(tabs): quick-fork keybinding, non-Claude fallback banner, ...
- fix(statusbar): stop double-applying chrome zoom to status bar popovers
...

## 0.55.20 — 2026-08-22
...
```

Each version's section is delimited by an `## X.Y.Z — YYYY-MM-DD` heading
and runs until the next `## ` heading — directly `awk`-extractable
(`awk '/^## 0\.55\.21 /{f=1} /^## 0\.55\.20 /{f=0} f'` or equivalent) with
no need to re-derive content from `git log`. This is a strictly better
content source than the first draft's raw-commit-log approach: it's the
same human/agent-curated, changeset-derived summary that already ships in
every release's changelog today.

### 3.6 Spec doc conventions confirmed

Unchanged from the first draft: `docs/specs/SPEC_<TOPIC>_<YYYY_MM_DD>.md`,
header block (Date/Status/Scope), numbered `##` sections, tables for
comparative decisions, explicit "Files to create/modify" and "Open
questions" sections.

---

## 4. Trigger design

### 4.1 Timezone reasoning (unchanged)

`git log --date=iso -5` shows recent commits at `-0700` (US Pacific
Daylight Time). `ci-nightly-build.yml` (`0 7 * * *` UTC = 00:00 PDT) and
`ci-nightly-artifacts.yml` (`0 6 * * *` UTC = 23:00 PDT) are already tuned
to fire around Pacific midnight, after the observed evening commit window.

### 4.2 Recommended trigger: `workflow_run` chained off `ci-nightly-build.yml`

Same reasoning as the first draft, still valid: `workflow_run` on
`ci-nightly-build.yml` completing avoids DST drift and avoids guessing a
fixed buffer for how long the 3-platform build+test matrix takes. Add
`workflow_dispatch: {}` alongside it for manual testing and on-demand runs.
If a fixed-cron equivalent is wanted as a documented fallback: `0 8 * * *`
UTC (01:00 PDT / 00:00 PST).

### 4.3 Skip condition — redefined around "is there a pending untagged bump"

Before anything else:

```bash
# Find the most recent "chore: release vX.Y.Z" commit reachable from main
match=$(git log origin/main --grep='^chore: release v[0-9]' -E -n 1 --format='%H%x09%s')
sha="${match%%$'\t'*}"
subject="${match#*$'\t'}"
version=$(grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' <<<"$subject")

if [ -z "$sha" ]; then
  echo "No release commit found on main at all — nothing to do." | tee -a "$GITHUB_STEP_SUMMARY"
  exit 0
fi

if git ls-remote --exit-code --tags origin "refs/tags/${version}" >/dev/null 2>&1; then
  echo "Latest release commit (${version}, ${sha:0:9}) is already tagged — nothing pending. Skipping." | tee -a "$GITHUB_STEP_SUMMARY"
  exit 0
fi

echo "sha=${sha}"   >> "$GITHUB_OUTPUT"
echo "version=${version}" >> "$GITHUB_OUTPUT"
```

This is the exact same tag-existence check performed manually this session
before tagging `v0.55.21` (`git tag -l "v0.55.21"` returning empty).
Notably, this correctly handles the case where several ordinary
(non-release) commits have landed on `main` *after* the release commit —
the version being checked is whatever the most recent `chore: release`
commit's subject says, not `HEAD`'s version, so pending unreleased feature
work never blocks or confuses the check. It also correctly handles the
rarer case of two `chore: release` commits landing before either gets
tagged (e.g. a human bumps twice in one day) — only the newest is ever
considered; an older, superseded bump is intentionally never tagged on its
own, matching how a human doing this manually would also behave.

---

## 5. Design decisions

### 5.1 High-level architecture

```
ci-nightly-build.yml (existing, unmodified)
  schedule: 0 7 * * * UTC — cargo build+test, Windows required / Linux+macOS best-effort
        │
        │ workflow_run (completed)
        ▼
nightly-release.yml (NEW, agentmuxai/agentmux) — ONE job: detect-and-tag
  ┌─────────────────────────────────────────────────────────────┐
  │  - conclusion == 'success'? if not → stop (no tag)            │
  │  - find latest `chore: release vX.Y.Z` commit on main (§4.3)  │
  │  - already tagged? → skip silently (summary-only)             │
  │  - else: git tag -a vX.Y.Z <sha> && git push origin vX.Y.Z    │
  └──────────────────────────┬────────────────────────────────────┘
                              │ (tag pushed)
                              ▼
        ┌─────────────────────────────────────┐
        │  release.yml           (EXISTING,     │
        │  container-image.yml    UNMODIFIED,    │
        │                          fires exactly  │
        │                          as for a manual │
        │                          tag push)        │
        │  → builds win/linux/mac                    │
        │  → gh release create (real, non-prerelease) │
        │  → repository_dispatch → agentmux-landing    │
        │    (landing page now correctly updates — §6)  │
        └───────────────────────────────────────────────┘

                    (no further step in agentmuxai/agentmux —
                     agentmux's own workflow ends once the tag is pushed)

gh-reporter (EXISTING Lambda, a5af/shared-infrastructure, cross-repo)
  EventBridge cron, independently polls the GitHub API once nightly —
  no signal is pushed to it from agentmux at all
  ┌─────────────────────────────────────────────────────────────┐
  │  - poll nightly-release.yml's latest run (mirrors the         │
  │    existing get_nightly_build_status() pattern)                │
  │  - poll GET /repos/agentmuxai/agentmux/releases/latest         │
  │    (was a new release published since yesterday's poll?)       │
  │  - render a new "Nightly Release" section into the SAME         │
  │    GitHub Nightly Report email it already sends (§5.5)           │
  └─────────────────────────────────────────────────────────────┘
```

Only **one** `workflow_run` chain link now (agentmux's own
`nightly-release.yml` off `ci-nightly-build.yml`) — the second chain link
from the §0 revision (a "job 2: notify" step waiting on `release.yml`) is
gone entirely, per §0.1: `gh-reporter` discovers the outcome itself on its
own existing cron, agentmux never needs to wait around for or react to
`release.yml`'s completion at all.

### 5.2 Versioning — no new scheme

Nightly publishes the exact `vX.Y.Z` tag the pending `chore: release`
commit already computed via `task release`. No `-nightly` suffix, no date
suffix, no separate namespace. This is deliberately different from the
first draft's `nightly-YYYYMMDD` proposal (§0) — there is now exactly one
tag format in play, matching exactly what `release.yml` and
`container-image.yml` already expect, so there is no second-namespace
collision risk to design around at all. (The first draft's warning that a
nightly tag must never accidentally match `release.yml`'s or
`container-image.yml`'s trigger pattern is trivially satisfied here — it's
*supposed* to match, that's the entire mechanism.)

### 5.3 Publishing mechanics

- **Not a prerelease.** `release.yml`'s own `gh release create` step
  (unmodified) runs exactly as it does for a manual tag — a real, fully
  visible GitHub Release, picked up by `/releases/latest`, same as every
  other version to date.
- **No retention/pruning logic needed.** Unlike the first draft's
  once-per-night disposable-artifact model, this produces at most one real
  release per pending bump — the same rate real releases have always
  shipped at, nothing new to bound or prune.
- **Cadence in practice:** most nights this will either (a) tag and publish
  whatever bump landed that day, or (b) skip silently because the last
  bump was already tagged (e.g. tagged manually earlier, or tagged by a
  previous night's run with no new bump since). Multiple nights could pass
  with nothing to publish if no one runs `task release` — expected and
  fine, matches §1.3's non-goal that nightly never decides to bump on its
  own.

### 5.4 Build reuse — literally nothing new

Simpler than the first draft's already-lean plan: nightly doesn't call
`build-windows.yml`/`build-linux.yml`/`build-macos.yml` itself at all —
pushing the tag is sufficient for `release.yml` to do so, unmodified. The
only new code in this entire spec is the "detect + tag + push" step (§4.3)
and the notification email (§5.5).

### 5.5 The nightly email — extend `gh-reporter`, per §0.1

**Superseded:** the standalone-Resend-sender design from the first
revision (§0). Confirmed by direct inspection of
`a5af/shared-infrastructure`'s `gh-reporter` Lambda that extending it is
both what the repo owner wants ("one email") and materially the easier
path — no new provider, no new secret, no new code in `agentmuxai/agentmux`
at all.

**How `gh-reporter` works today (confirmed by code inspection):**
- Python 3.12 Lambda, CDK-deployed. Source:
  `gh-reporter/lambda/lambda_function.py` (orchestration),
  `gh-reporter/lambda/github_client.py` (GitHub App API client),
  `gh-reporter/lambda/email_report.py` (HTML rendering). Infra:
  `gh-reporter/lib/gh-reporter-stack.ts`, `gh-reporter/bin/gh-reporter.ts`.
- Triggered by an EventBridge Scheduler cron, `0 0 * * ? *` **Pacific time**
  (`gh-reporter-stack.ts:249-259`, `bin/gh-reporter.ts:28`) — not a webhook.
  On each invocation it **polls** the GitHub Actions API itself; nothing
  needs to be pushed to it.
- Nightly CI status specifically comes from
  `GitHubClient.get_nightly_build_status()`
  (`github_client.py:341-382`), which calls
  `GET /repos/{repo}/actions/workflows/{workflow_filename}/runs?per_page=1`
  against workflow filenames read from env vars `NIGHTLY_BUILD_WORKFLOW` /
  `NIGHTLY_ARTIFACTS_WORKFLOW` (`lambda_function.py:56-58`, wired in
  `gh-reporter-stack.ts:227-229`, `bin/gh-reporter.ts:40-51`). A third env
  var, e.g. `NIGHTLY_RELEASE_WORKFLOW=nightly-release.yml`, slots into this
  exact existing pattern.
- Sends via SES (`lambda_function.py:151-158`, `Source=noreply@asaf.cc`,
  recipient from `RECIPIENT_EMAIL` — both pulled from Secrets Manager
  secret `services/infra`, no agentmux-side secret involved at all).
- The email body is one concatenated HTML string
  (`generate_html_email()`, `email_report.py:349-596`) built from a fixed
  render order of independently-fetched sections
  (`lambda_function.py:244-254`, `_ALL_SECTIONS` at line 47), each wrapped
  in `_safe_fetch()` so one section's failure can't blank the whole email.
  The existing `nightly_builds`/`nightly_artifacts` sections
  (`_render_nightly_section`, `email_report.py:318-346`) are the exact
  template to copy for a new section.

**What to add — a new `nightly_release` section, following that exact
pattern:**
1. `github_client.py`: a new method mirroring
   `get_nightly_build_status()` that polls
   `nightly-release.yml`'s latest run (via `NIGHTLY_RELEASE_WORKFLOW`), plus
   a new call to `GET /repos/agentmuxai/agentmux/releases/latest` to check
   whether a new (non-prerelease) release was published since the last
   report — comparing `published_at` against "since yesterday's poll" is
   enough to tell "new since last night" from "same one as last night."
   Optionally also fetches `VERSION_HISTORY.md`'s top section via the
   Contents API (`GET /repos/{owner}/{repo}/contents/VERSION_HISTORY.md?
   ref=<tag>`, §3.5's structure) to embed the real changelog instead of
   just a link.
2. `lambda_function.py`: add `nightly_release` to `_ALL_SECTIONS` (line 47)
   and a `_safe_fetch(...)` entry in the `sections` dict
   (lines 244-254) — same shape as the existing two.
3. `email_report.py`: a `_render_nightly_release_section()` mirroring
   `_render_nightly_section()` (lines 318-346), inserted into the
   concatenation order (lines 582-588), reporting one of three states:
   - **Skipped** — "No pending version bump — nothing to publish" (§4.3's
     common case).
   - **Published** — version, the `VERSION_HISTORY.md` changelog, unique
     contributor list (`git log <prev-tag>..<new-tag> --format=%an` via
     the compare API), and links to the GitHub Release and
     `https://agentmux.ai` (confirming the landing page reflects it, per
     §6).
   - **Tagged but not found published** — `nightly-release.yml` ran and
     pushed a tag, but no matching GitHub Release shows up — inferred
     downstream `release.yml` failure (§5.7 case 3), surfaced with a link
     to investigate.
4. CDK (`bin/gh-reporter.ts`, `gh-reporter-stack.ts`): add
   `NIGHTLY_RELEASE_WORKFLOW` to the `ReportConfig` interface / `reports`
   array and the Lambda's env, then `cdk deploy`.

**Deploy coordination — call this out explicitly:** unlike everything else
in this spec, this is a real cross-repo change requiring its own PR review
and `cdk deploy` in `a5af/shared-infrastructure`, independent of
`agentmuxai/agentmux`'s own release cadence. Sequencing matters: deploy the
`gh-reporter` change first (it fails safe via `_safe_fetch()` if
`nightly-release.yml` doesn't exist yet — worst case that section is empty,
not a broken email), *then* merge/enable `nightly-release.yml` itself, not
the other way around.

**A real timing risk worth flagging now, not discovering later:**
`gh-reporter`'s cron fires at `0 0 * * ? *` **Pacific** — i.e. right around
the same moment `ci-nightly-build.yml` (`0 7 * * *` UTC = 00:00 PDT) even
*starts*. The full chain this spec adds
(`ci-nightly-build.yml` finishes → `nightly-release.yml` tags → `release.yml`
builds 3 platforms and publishes) took **roughly 30 minutes** end-to-end in
this session's own manually-triggered `v0.55.21` run, and could run longer
under load. If `gh-reporter` polls at exactly midnight PT, it will very
likely see last night's data, not tonight's — the "Published" case would
consistently read one day stale. This needs a fix before relying on it:
either push `gh-reporter`'s cron later (e.g. 02:00 PT, comfortably after
the full chain should be done) or accept the one-day lag as a known,
documented tradeoff. Recommend the former — it's a one-line CDK change
already being touched for the new env var anyway.

### 5.6 Quality gate

Unchanged in mechanism from the first draft (§3.1): gated on
`ci-nightly-build.yml`'s `conclusion == 'success'`, Windows
required/blocking, Linux/macOS best-effort — the same bar already
considered "good enough" today. What's new in this revision: this gate is
now explicitly a **second, additional** check layered on top of the
release commit's own already-passed PR-level CI (§3.1), not the sole gate
standing between an untested commit and a public release. Both checks
already exist; nightly-release.yml just chains off one of them.

### 5.7 Failure handling

Three distinct cases, all now surfaced through the **single** nightly
report email (§5.5, per §0.1) rather than any agentmux-side notification —
agentmux's own workflow has no email step to fail out of:

1. **No pending untagged release commit (§4.3).** `nightly-release.yml`
   exits cleanly with a summary-only note. `gh-reporter`'s new section
   reports "Skipped — no pending version bump."
2. **`ci-nightly-build.yml` failed.** `nightly-release.yml`'s gate stops
   before tagging anything. `gh-reporter`'s *existing* `nightly_builds`
   section already reports the red CI run — no new handling needed, and
   the new `nightly_release` section can simply note "not attempted — CI
   was red" alongside it, in the same email, same read.
3. **Tag pushed successfully, but `release.yml` itself then fails** (a
   platform build breaks, `gh release create` errors, etc.). This is a
   real risk unique to this design — nightly just autonomously pushed a
   real stable-release tag with nobody watching in real time the way a
   human pushing it manually would be. Per §5.5, `gh-reporter` infers this
   case itself (tag/run exists, no matching new release found) and reports
   it as "Tagged but not found published — investigate" in the same
   section. **Do not** auto-delete the pushed tag on this path from
   `nightly-release.yml`'s side: if e.g. Windows and Linux assets already
   uploaded before the macOS leg failed, deleting the tag ref doesn't clean
   up those already-published assets and would make an already-partial
   state harder to reason about, not easier — leave it for manual triage.

**Tradeoff worth being explicit about:** folding case 3 into the daily
digest (rather than a first-draft-style immediate urgent email) means a
mid-pipeline failure surfaces on `gh-reporter`'s next cron tick — up to
~24h later, not immediately. The repo owner's explicit ask was "one email,"
so this spec accepts that latency tradeoff rather than reintroducing a
second, faster channel. If same-night visibility into case 3 specifically
ever becomes necessary, revisit as a targeted addition then — see OQ5.

---

## 6. Landing page: now correctly triggered — this is the point, not a risk

**Revised recommendation: let `release.yml`'s existing `repository_dispatch`
to `agentmux-landing` fire exactly as it already does for a manual tag —
no change needed, no new dispatch to build.**

The first draft (§0) treated this dispatch as a risk to suppress, on the
theory that a nightly build might be unvetted `main` HEAD reaching the
public download page. That risk doesn't apply to the revised design:

- The commit nightly tags is not raw HEAD — it's the specific, already
  merged, already-PR-reviewed, already-CI-gated `chore: release` commit
  (§1.1, §4.3). It already cleared the same bar a manually-tagged release
  clears today.
- `release.yml`'s dispatch, its landing-page consumer
  (`agentmux-landing`'s `landing-deploy.yml` → `fetch-release.mjs`), and
  its "read the latest GitHub Release" logic (per
  `SPEC_RELEASE_CICD_CORRECTION_2026_06_30.md` §2) all continue to work
  exactly as designed — there is no new "is this a nightly or a real
  release" distinction for them to get wrong, because there's no longer a
  second release *type* in this design at all (§5.2). What reaches the
  landing page is, definitionally, the same kind of release it has always
  served.
- This is in fact the specific behavior the repo owner asked for directly:
  "it would also update the landing page" — confirmed as a goal, not
  merely tolerated.

**Nothing in `agentmux-landing` needs to change.** The only behavioral
difference from today is *how often* a fresh tag shows up for it to react
to — potentially nightly instead of whenever a human remembers — which is
exactly the intended effect.

---

## 7. Files to create / modify

| File | Repo | Action |
|---|---|---|
| `.github/workflows/nightly-release.yml` | agentmuxai/agentmux | **Create** — the single detect-and-tag job described in §5.1; no email code |
| `.github/workflows/release.yml`, `container-image.yml` | agentmuxai/agentmux | **No change** — nightly pushes a tag matching their existing patterns exactly, by design |
| `.github/workflows/build-windows.yml` / `build-linux.yml` / `build-macos.yml` | agentmuxai/agentmux | **No change** — never invoked directly by nightly at all (§5.4) |
| `agentmux-landing` (any file) | agentmuxai/agentmux-landing | **No change** — §6 |
| `gh-reporter/lambda/github_client.py` | a5af/shared-infrastructure | **Modify** — add `nightly-release.yml` run polling + `releases/latest` check (§5.5) |
| `gh-reporter/lambda/lambda_function.py` | a5af/shared-infrastructure | **Modify** — register the new `nightly_release` section (§5.5) |
| `gh-reporter/lambda/email_report.py` | a5af/shared-infrastructure | **Modify** — add `_render_nightly_release_section()` (§5.5) |
| `gh-reporter/bin/gh-reporter.ts`, `gh-reporter/lib/gh-reporter-stack.ts` | a5af/shared-infrastructure | **Modify** — add `NIGHTLY_RELEASE_WORKFLOW` env var; consider moving the cron later (§5.5's timing risk) |

---

## 8. Rollout plan

1. **Implement the detect-and-tag step (§4.3) standalone first**, testable
   locally against real repo history before touching CI at all — confirm
   it correctly identifies `v0.55.21` as already-tagged (post-merge of
   PR #2767 in this session) and correctly reports "nothing pending" as of
   today.
2. **Implement `nightly-release.yml` job 1**, `workflow_dispatch`-only
   while testing. Dry-run against a real pending bump if one exists at
   test time, or temporarily point it at a scratch/fork repo with its own
   throwaway `chore: release` commit to avoid accidentally tagging
   production ahead of schedule.
3. **Confirm the downstream trigger fires correctly** — after job 1 pushes
   a real tag, verify `release.yml` and `container-image.yml` both start,
   exactly as they did for the manual `v0.55.21` tag push earlier this
   session.
4. **Confirm §6 stays true**: verify the `repository_dispatch` to
   `agentmux-landing` fires and the landing site actually updates — this
   flips from "confirm it does NOT fire" in the first draft to "confirm it
   DOES fire," worth a deliberate manual check the first time end-to-end.
5. **Implement the `gh-reporter` side (§5.5) in `a5af/shared-infrastructure`**
   as its own PR: `github_client.py` polling additions, the new
   `nightly_release` section in `lambda_function.py`/`email_report.py`, the
   CDK env-var wiring, and — while touching the CDK stack anyway — the
   cron-timing fix (push `0 0 * * ? *` PT later, e.g. to `02:00` PT, per
   §5.5's timing risk). Test locally against `agentmuxai/agentmux`'s
   *current* state first (today: `v0.55.21` already tagged, nothing
   pending) to confirm the "Skipped" rendering before any real tag exists
   to test the "Published" rendering against.
6. **Deploy `gh-reporter` first** (`cdk deploy`), confirm the new section
   renders (as "Skipped," since nothing's pending yet) in that night's real
   report before enabling anything on the `agentmuxai/agentmux` side — per
   §5.5's sequencing note, `_safe_fetch()` makes this safe to deploy ahead
   of `nightly-release.yml` existing at all.
7. **Wire `nightly-release.yml`'s real `workflow_run` trigger** off
   `ci-nightly-build.yml`, let it run unattended through a handful of real
   nights. Confirm: quiet nights stay silent, a real pending bump gets
   tagged + published + landing-page-updated, and the *next* `gh-reporter`
   report correctly shows "Published" with the right version/changelog/
   links — end-to-end across both repos, not just each half in isolation.
8. **Exercise the §5.7 case-3 failure path deliberately** (e.g. a throwaway
   branch with a `release.yml` step forced to fail) and confirm
   `gh-reporter` correctly renders "Tagged but not found published" rather
   than silently omitting the section or misreporting it as a normal skip.

---

## 9. Alternatives considered

- **The first draft's design: separate `nightly-YYYYMMDD` snapshot channel,
  no landing-page dispatch.** Superseded, not merely rejected — see §0.
  Was a reasonable design for the problem as originally understood ("ship
  something downloadable every night, similar to nightly CI"), but the
  repo owner clarified the actual desired behavior ("nightly simply
  publishes the last bump... it would also update the landing page"),
  which maps onto a materially simpler and lower-risk design once
  correctly understood — no new tag namespace, no new trust tier, no
  prerelease/retention machinery, and the landing-page update is now
  correct rather than dangerous.
- **A single rolling `nightly` tag instead of per-bump tags.** Not
  applicable to the revised design — there's no repeated nightly artifact
  type to roll over; each publish is a distinct real version.
- **Have nightly also run `task release` itself** (auto-bump, not just
  auto-tag). Rejected — explicitly out of scope per the repo owner ("during
  dev we bump the patch and set the changesets" — a human/agent decision
  stays a human/agent decision) and per §1.3. Nightly only finishes a
  publish decision someone already made, never makes the decision itself.
- **New self-contained Resend-based sender, gated behind a new agentmux
  repo secret** (the §0-revision design). Superseded per §0.1, not merely
  rejected — the repo owner explicitly asked for one email, not two, and
  `gh-reporter` turned out to already have everything needed (standing
  GitHub App auth, proven SES sending, a section-based template built for
  exactly this) once actually inspected — extending it is now strictly
  less new infrastructure than building a second sender would have been,
  not just a stylistic preference.

---

## 10. Open questions

| # | Question | Notes |
|---|---|---|
| OQ1 | Should the auto-published release's GitHub Release notes carry any visible "published via nightly automation" marker, for anyone browsing the Releases page directly (independent of the email, §5.5)? | Low-cost, recommend yes — a one-line footer in the release body, e.g. appended by `release.yml`'s own `--notes-file` generation, but this would be the one (minimal) touch to `release.yml` this spec would introduce; needs a decision on whether that's worth it vs. leaving `release.yml` fully untouched. |
| OQ2 | If a `chore: release` PR sits open/unreviewed for many days, should anything nag about it, or is silence fine (matches today's status quo)? | Recommend: out of scope, silence is fine — nightly only reacts to already-merged commits, never to open PRs. |
| OQ3 | Should `nightly-release.yml` re-verify the five-location version-consistency invariant (`CLAUDE.md` "Release consistency invariant") on the release commit before tagging, as defense-in-depth beyond what `release-consistency.yml` already enforced at merge time? | Cheap to add, catches a hypothetical direct-push-to-main edge case; not blocking for Phase 1 since branch protection should already prevent this. |
| OQ4 | Is `02:00 PT` the right new time for `gh-reporter`'s cron (§5.5), or should it be even later / made dynamic (e.g. triggered off `release.yml` completing via a cross-repo signal instead of a fixed offset)? | Recommend starting with the fixed later time — simplest fix, matches the existing polling-not-webhook architecture; revisit only if the fixed offset proves unreliable in practice (e.g. build times grow past the buffer). |
| OQ5 | Does the §5.7 case-3 failure (tag pushed, release build failed) ever need same-night visibility instead of waiting for the next daily digest? | Explicitly deferred per the repo owner's "one email" direction (§0.1) — revisit only if a real incident demonstrates the ~24h lag is actually a problem in practice, rather than speculatively building a second channel now. |
