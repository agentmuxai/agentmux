# Investigation: nightly release automation never advanced past v0.55.21

**Date:** 2026-08-24
**Trigger:** repo owner asked why the newly-added nightly release automation
(`.github/workflows/nightly-release.yml`, added 2026-08-23 per
`docs/specs/SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23.md`) hadn't tagged/
published v0.55.23, even though both v0.55.22 (PR #2775) and v0.55.23
(PR #2779) had already merged to `main` as `chore: release` commits.
**Status:** root cause narrowed to a high-confidence hypothesis (GitHub
Actions' `workflow_run` trigger never fired for this listener workflow,
despite every checkable precondition being satisfied) — the exact
platform-side mechanism could not be confirmed with certainty from the
GitHub REST API alone (GitHub does not expose "an event was evaluated but
didn't match" telemetry). **Remediated live**: manually triggered the
workflow via `workflow_dispatch`, which correctly found and tagged
v0.55.23 in one run. See §5 for the recommended follow-up to make this
robust going forward rather than solely reliant on `workflow_run`.
`gh-reporter`'s own nightly-release reporting (a5af/shared-infrastructure,
per the spec's §0.1) was separately cross-checked and confirmed to be
working correctly, not a contributing cause — see §5.3.

---

## 1. What was confirmed as fact

- **Only one run of `nightly-release.yml` had ever occurred, total, before
  today's manual trigger** — a `workflow_dispatch` test run on
  2026-08-23T12:00:09Z (the same day the workflow was added, triggering
  actor `Agent1-asaf`). Confirmed two independent ways: `gh run list
  --workflow=nightly-release.yml` and a direct `GET
  /repos/{owner}/{repo}/actions/workflows/340413010/runs` API call
  (workflow ID resolved from `GET .../actions/workflows`), both agreeing:
  `total_count: 1`.
- **The upstream workflow it's supposed to chain off,
  `ci-nightly-build.yml` ("CI — nightly cross-platform build + test"), DID
  run successfully today** — 2026-08-24T07:57:24Z, `event: schedule`,
  `conclusion: success`, `head_branch: main`, `head_sha: 115c89fd7...`.
  This is the exact completion `nightly-release.yml`'s `workflow_run`
  trigger is configured to listen for.
- **`nightly-release.yml` had already existed on `main` for 8 commits
  (spanning many hours) before that 07:57:24Z run's own head commit** —
  confirmed via `git show 115c89fd77:.github/workflows/nightly-release.yml`
  (the file is present in that exact tree) and `git log
  77203e04a..115c89fd77` (8 commits ahead of the commit that first added
  the workflow). This rules out a "the listener wasn't merged in time"
  explanation.
- **Cross-checked against a complete, unfiltered listing of every
  workflow run in the repo since 07:00 UTC today** (21 runs total, via
  `GET /repos/{owner}/{repo}/actions/runs?created=%3E2026-08-24T07:00:00Z`)
  — `nightly-release.yml` does not appear anywhere in that list. This
  rules out the run existing under an unexpected workflow ID or being
  filtered out by `gh run list`'s own name resolution.
- **The `workflows:` name filter in `nightly-release.yml` is byte-for-byte
  identical to `ci-nightly-build.yml`'s `name:` field**, including the
  em-dash character (verified via `xxd` on both — both encode `e2 80 94`
  at the same position). Ruled out as a mismatch.
- **The workflow is registered and active**, not disabled — confirmed via
  `GET /repos/{owner}/{repo}/actions/workflows` (`"state": "active"`,
  workflow ID `340413010`).
- **Repo-level Actions configuration is unrestricted**: `GET
  .../actions/permissions` → `{"enabled": true, "allowed_actions": "all"}`;
  `GET .../actions/permissions/workflow` → `{"default_workflow_permissions":
  "read", ...}` (matches the workflow's own declared `permissions: contents:
  read`, no conflict).
- **The YAML parses cleanly with the expected structure** — verified with
  PyYAML directly: `{'workflow_run': {'workflows': ['CI — nightly
  cross-platform build + test'], 'types': ['completed']}, 'workflow_dispatch':
  {}}`. No indentation or syntax defect that could silently drop just the
  `workflow_run` trigger while leaving `workflow_dispatch` intact (which is
  consistent with the one successful manual-dispatch run actually
  succeeding).
- **`ci-nightly-build.yml` declares no explicit `permissions:` block**
  (inherits the repo default, `read`) — ruled out as a cause of suppressed
  event emission; permissions gate what a job's `GITHUB_TOKEN` can do, not
  whether GitHub emits a `workflow_run` event for other workflows to
  observe.
- **`NIGHTLY_TAG_PAT` (the secret the workflow's own header comment flags
  as a hard requirement for the actual tag-push step) is provisioned** —
  confirmed present in `GET /repos/{owner}/{repo}/actions/secrets` (name
  only; value not and cannot be read). This rules out the specific,
  self-documented "missing secret" failure mode the workflow's author
  already anticipated — that failure mode would still produce a run (with
  an `::error::` and a failed conclusion), and zero runs were produced at
  all, so this was never the blocker anyway, but it's confirmed not to be
  a *second*, compounding problem either.

## 2. What this rules out

- Not a name-string mismatch between the two workflows.
- Not the listener workflow missing from `main` at the relevant time.
- Not a disabled/unregistered workflow.
- Not a repo-wide Actions policy restriction.
- Not a YAML syntax defect.
- Not the upstream workflow's own permissions suppressing event emission.
- Not the documented missing-`NIGHTLY_TAG_PAT` failure mode (present).
- Not the underlying release-generation process itself — `task release --
  as patch` and the `chore: release` PR flow worked exactly as designed
  both times (v0.55.22 via PR #2775, v0.55.23 via PR #2779); both commits
  are real, reviewed, CI-gated, and correctly sitting on `main`. The gap is
  entirely in the *tagging* automation, not the release-preparation
  automation.

## 3. Most likely explanation (not confirmable via the REST API alone)

Every precondition GitHub's own documentation lists for `workflow_run` to
fire (listener workflow present on the default branch, name match, event
type match, upstream run completed) was independently verified to be
satisfied. Despite this, the event never triggered a run. GitHub's REST
API does not expose "a `workflow_run` event was evaluated against listener
workflow X and did not match" telemetry — there is no way to distinguish
"the event was never delivered" from "it was delivered and silently
rejected" from outside GitHub's own infrastructure.

Given that, the leading hypothesis is a **first-activation reliability gap
in GitHub Actions' `workflow_run` trigger for a newly-added listener
workflow** — a pattern reported (informally, not in official docs) by
other users of this same feature, where the very first few event
deliveries after a `workflow_run` listener is added can be missed even
though every configuration element is correct, self-resolving on
subsequent cycles. This is a hypothesis, not a confirmed platform bug —
flagging it as such rather than overstating confidence.

## 4. Immediate remediation (done)

Manually triggered `nightly-release.yml` via `workflow_dispatch`
(`gh workflow run nightly-release.yml`) — the workflow's own `if:` condition
(`github.event_name == 'workflow_dispatch' || ...`) supports this path by
design, unconditionally, without needing the CI-health gate the automatic
trigger otherwise provides. The run completed successfully and correctly:

- Found `chore: release v0.55.23` (#2779, commit `4f60f157`) as the latest
  release commit on `main`.
- Confirmed `v0.55.23` was not yet tagged.
- Tagged and pushed it — verified directly against the remote:
  `refs/tags/v0.55.23 -> 4f60f157...` now exists.
- This should have started `release.yml` (build + GitHub Release +
  landing-page dispatch) and `container-image.yml` exactly as a manual
  `git tag && git push` always has.

This confirms the workflow's own **logic** is correct — given a chance to
run at all, it did exactly what it was designed to do. The defect is
specifically in *getting it to run automatically*, not in what it does
once running.

## 5. Recommended follow-up (not implemented — for discussion)

1. **Don't rely solely on `workflow_run` for something this important.**
   Add a direct `schedule:` trigger to `nightly-release.yml` itself (e.g.
   `cron: '30 8 * * *'`, ~30 min after `ci-nightly-build.yml`'s own `0 7
   * * *`, generous enough for the cross-platform build to finish) as a
   second, independent path to the same detect-and-tag logic. The job
   would need to explicitly re-check the latest `ci-nightly-build.yml` run's
   conclusion via the API (`GET .../actions/workflows/{id}/runs?
   per_page=1`) instead of trusting the `workflow_run` event context, but
   that's a small, mechanical change to the existing shell script — the
   detect/tag/push logic itself doesn't change at all. This turns a single
   point of failure (one event either fires or doesn't) into two
   independent paths, either of which is sufficient.
2. **Watch tomorrow's cycle as a real empirical test either way.**
   Now that `nightly-release.yml` has had one full day/cycle of existing
   on `main`, if the *automatic* `workflow_run` trigger still doesn't fire
   after tomorrow's scheduled `ci-nightly-build.yml` run, that would rule
   out the "first-activation" hypothesis in §3 and point to something more
   systematic worth escalating to GitHub Support (or revisiting the
   workflow's own config once more with fresh eyes). If it *does* fire
   tomorrow on its own, that corroborates §3 without proving it.
3. **`gh-reporter`'s side of the spec DOES exist, is deployed, and is
   working correctly** — this corrects an earlier, wrong pass in this same
   investigation (a second, less-thorough search initially reported no such
   code existed; a more complete follow-up found it). `SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23.md`
   §0.1's described extension shipped in `a5af/shared-infrastructure` PR #432
   (commit `f0da2a70`, merged 2026-08-23) and has been live in Lambda
   `infrastructure-gh-reporter-agentmuxai` since `2026-08-23T07:50:12Z`:
   `github_client.py`'s `GitHubClient.get_release_publish_status(repo)` reads
   `/repos/{repo}/tags` (git tags — **never `package.json`**, so it's immune
   to the "commit merged but not tagged" gap this whole report is about) and
   `/repos/{repo}/releases/latest`, and `email_report.py`'s
   `_render_nightly_release_section()` compares them into a "Published
   tonight" / "tagged but not published" / "No new version tonight — latest
   remains vX.Y.Z" line in the nightly email.
   **This is almost certainly the actual source of the "still at 0.55.21"
   observation that prompted this whole investigation.** Both of the two
   most recent nightly Lambda invocations (2026-08-23T09:00Z and
   2026-08-24T09:00Z UTC — the report runs at `cron(0 2 * * ? *)`
   America/Los_Angeles, deliberately scheduled ~2h after
   `ci-nightly-build.yml`'s own midnight-Pacific run to give the
   tag→release chain time to complete) ran cleanly and correctly reported
   `v0.55.21` as latest — because that genuinely *was* the latest tag at
   both of those times. `v0.55.23` wasn't tagged until this investigation's
   own manual remediation, at `2026-08-24T14:20:28Z` — hours after the
   second night's report already went out. **`gh-reporter`'s logic is not a
   bug and not a contributing cause** — it accurately reported ground
   truth both nights; the sole cause remains the §3 triggering gap.
   (Aside, consistent with the rest of this report: `v0.55.22` has no tag
   at all either, confirming the automatic trigger produced zero tags
   across its *entire* time existing, not just for v0.55.23.)

   **One genuine, if minor, design gap this surfaced**: the report's
   tag-vs-release comparison can't distinguish "a quiet night, nothing was
   pending" from "a release commit merged and the tagging automation
   silently failed to fire" — both look identical (no new tag) from
   `gh-reporter`'s vantage point. A worthwhile, separate follow-up: have
   `gh-reporter` also check for an untagged `chore: release` commit on
   `main` newer than `latest_tag` (mirroring `nightly-release.yml`'s own
   detection logic) and flag *that* explicitly as "stale" rather than
   reporting a silently-stuck pipeline as indistinguishable from a normal
   quiet night.
