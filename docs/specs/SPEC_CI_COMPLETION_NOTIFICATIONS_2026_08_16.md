# SPEC: jekt notification when a PR's CI run completes (pass or fail)

**Date:** 2026-08-16
**Status:** Implemented — `agentmux-cloud` PR #48 (merged, deployed)
**Author:** AgentX
**Repos touched:** `agentmux-cloud` (implementation), `agentmux` (this doc)
**Related:** `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` (agent-resolution
priority this spec reuses unchanged), `events/ci-failure.ts` (existing
per-check-run failure notifier this spec sits alongside)

---

## 1. Motivation

Today an agent that pushes a PR and wants to know when CI finishes has to
poll (`gh pr checks --watch` or repeated `pr checks`) or just guess how
long to wait. The `github-consumer` Lambda in `agentmux-cloud/muxbus/`
already jekts an agent for two GitHub events — a review lands
(`events/review.ts`) and a PR merges (`events/merge.ts`) — plus a narrower
third case, an individual **failing** check run
(`events/ci-failure.ts`). There is no "your CI run is done" signal at all,
success or failure, for the run as a whole.

This spec adds that: one jekt when a PR's CI run concludes, covering both
outcomes.

## 2. The core design problem: `check_run` fires per job, not per PR

`check_run` — the event `ci-failure.ts` already consumes — fires once
**per individual check**, not once per PR. A typical `agentmux` PR (see
`.github/workflows/ci-pr.yml`, `release-consistency.yml`) produces at
least 4 separate `check_run` completions: 3 matrix legs of
`CI (PR) — compile tests + run` (Ubuntu/Windows/frontend) plus
`Release consistency check`. Naively generalizing `ci-failure.ts` to also
notify on `conclusion === 'success'` would jekt an agent 4 separate times
per green PR — noise, not signal.

**Proposed fix: subscribe to `check_suite`, not `check_run`.** GitHub
aggregates every check run from one CI provider (GitHub Actions is one
provider/app for this purpose, regardless of how many workflow files ran)
into a single check suite per commit SHA, and fires exactly one
`check_suite` `completed` event with an overall `conclusion`
(`success`/`failure`/`neutral`/etc.) once every check run in it has
finished. That's the "your CI run is done" signal this spec wants, and
GitHub computes the aggregation for us instead of the consumer having to.

**Alternative considered and rejected:** aggregate `check_run` events
ourselves — on each completion, call the GitHub API to list all check runs
for the SHA and notify only once everything's done. Rejected because it
needs new cross-event dedup (two check runs completing near-simultaneously
could both observe "everything else is done" and double-notify), on top
of an extra GitHub API call per event. `check_suite` gets the same outcome
for free and reuses the existing per-SNS-message idempotency
(`claimEventOnce()`, `handler.ts:51-71`) with no new dedup logic.

## 3. Relationship to the existing `ci-failure.ts` (open question — flagging, not deciding)

This spec's `check_suite` notification and the existing `check_run`
failure notification are not mutually exclusive — they answer different
questions ("something just broke, right now" vs. "the whole run is
over"). Two ways to reconcile them, not resolved here:

- **(a) Keep both.** `ci-failure.ts` still fires fast, per-job, the moment
  a check fails (early signal, e.g. useful on a slow CI matrix where one
  leg fails long before the others finish). The new `check_suite` handler
  additionally fires once, always, when the suite concludes — meaning a
  failing PR gets two jekts (one fast per-failure ping, one final
  summary). Simple, no behavior change to existing code, but a failing PR
  is chattier than a passing one.
- **(b) Retire `ci-failure.ts`, replace it entirely with the new
  `check_suite` handler.** One jekt per PR either way, but loses the fast
  per-job failure signal on multi-leg CI (agent finds out only once
  *everything* finishes, not when the first thing breaks).

Recommend (a) as the safer default — it's additive, doesn't touch
known-working code — but this is a judgment call for whoever implements
this, not something to guess at silently (same posture as the open
question flagged in `reagent` PR #194 for reply-mode's keyword matching).

## 4. Payload and trust gating

`check_suite` payload shape mirrors `check_run`'s relevant fields —
`action`, `check_suite.conclusion`, `check_suite.pull_requests[]` (each
with `number` and `head.ref`), `repository.full_name`. Only process
`action === 'completed'`; ignore `requested`/`rerequested`/`in_progress`.

Reuse `ci-failure.ts`'s existing pattern unchanged:

- Skip suites with zero associated PRs (`pull_requests.length === 0`) —
  same as `ci-failure.ts:87-89`.
- Fetch PR details via `fetchPRDetails()` (`handler.ts:371-382`, already
  generic) to get `user.login` and `head.repo.owner.login`.
- Gate on `isTrustedHeadRepo(prDetails.head.repo?.owner?.login)`
  (`agent-mapping.ts`) **before** resolving anything from the payload —
  identical fork-impersonation protection as `ci-failure.ts:111-117` and
  `review.ts:134-138`.

**New, relative to `ci-failure.ts`:** also add the PR-body-tag fallback
that `review.ts` has and `ci-failure.ts` currently lacks
(`extractAgentIdFromBody()`, `review.ts:73-90`) — i.e. resolve the target
agent the same way `review.ts` does: PR author's GitHub username first
(`getAgentId()`), only falling back to the `<!-- agentmux:agent_id=... -->`
body tag when the username doesn't resolve (e.g. a PR pushed under the
shared `GenericAgentX-<host>` fallback account). `ci-failure.ts` skipping
this fallback today means a `GenericAgentX`-authored PR's CI failures
currently notify nobody; this spec's handler should not repeat that gap.
(Whether to also backport the fallback into `ci-failure.ts` itself is a
separate, smaller fix — worth doing, not blocking on this spec.)

## 5. Message format

Mirror the existing bracketed-status convention
(`[FAIL] PR #N FAILED CI`, `[ReAgent] PR #N reviewed`, `MERGED | PR #N
merged`):

```
[CI] PR #2601 CI PASSED

Title: chore: release v0.55.10
Branch: agentx/release-patch-2026-08-16
Checks: 4/4 passed
Details: https://github.com/agentmuxai/agentmux/pull/2601/checks
```

or, on failure:

```
[CI] PR #2601 CI FAILED

Title: chore: release v0.55.10
Branch: agentx/release-patch-2026-08-16
Checks: 3/4 passed, 1 failed
Details: https://github.com/agentmuxai/agentmux/pull/2601/checks
```

"Checks: X/Y" requires listing the suite's check runs
(`GET /repos/{repo}/commits/{sha}/check-runs` — a new API call not needed
by any existing handler) to count pass/fail; alternatively, omit the
count and rely on `check_suite.conclusion` alone for a simpler first
version (`[CI] PR #N CI PASSED` / `FAILED`, no "Checks:" line) — cheaper,
one fewer API call, marginally less informative. Pick one before
implementing; not resolved here.

## 6. Infrastructure changes required

1. **`muxbus-stack.ts:284`** — widen the SNS filter policy's `event_type`
   allowlist from `['pull_request_review', 'check_run', 'pull_request']`
   to include `'check_suite'`.
2. **Verify upstream subscription separately** — the SNS filter policy
   only filters events that already reach the topic; confirm the GitHub
   App/webhook config feeding `github-webhooks-topic-arn` (the
   `github-router` fan-out this spec's research did not trace) is itself
   subscribed to the `check_suite` webhook event, not just `check_run`.
   If not, this is a required deployment prerequisite, not just an SNS
   filter change.
3. **New `consumers/github/events/ci-complete.ts`** (+ `.test.ts`) —
   `processCICompleteEvent()`, structured like `ci-failure.ts`'s
   `processCIFailureEvent()`.
4. **`handler.ts`** — new `case 'check_suite':` branch in
   `processGitHubEvent()` (alongside the existing `check_run` branch,
   `handler.ts:413-427`), wired to the new processor.
5. No changes needed to `injectWithRetry()`, `claimEventOnce()`,
   `signReagentMessage()`, or the TTL/retry machinery — all reused as-is.

## 7. Out of scope for this pass

- Per-check-name filtering (e.g. only notify for `CI (PR)`, ignore
  nightly/non-PR workflows) — `check_suite.pull_requests` already
  naturally excludes suites with no associated PR, which covers most of
  this; an explicit allowlist is not proposed here.
- Notifying on `neutral`/`cancelled`/`timed_out` conclusions distinctly
  from `failure` — proposed default is to treat any non-`success`
  conclusion as the `FAILED` message; splitting these into their own
  wording is a possible future refinement, not required now.
- Configurability (per-repo opt-in/out, per-agent notification
  preferences) — none of the three existing handlers have this either;
  consistent to omit it here too.

## 8. Implementation notes (added post-merge)

What actually shipped in `agentmux-cloud` PR #48, resolving this spec's
open questions and one gap found during reagent's review that this spec
did not anticipate:

- **§3 (relationship to `ci-failure.ts`):** resolved as (a) — kept both,
  additive. `ci-failure.ts` is unmodified except for the `PullRequestDetails`
  interface widening (§6 below).
- **§5 (message format):** shipped the simpler form —
  `[CI] PR #N CI PASSED/FAILED` with no "Checks: X/Y" breakdown, no extra
  API call.
- **New, not anticipated by this spec — the check-suite `app` filter
  (reagent P1 on #48):** a repo can have multiple GitHub Apps producing
  check suites on the same commit (GitHub Actions, CodeQL/code-scanning,
  Dependabot, ...) — each is an independent `check_suite` with its own
  `completed` event. Without filtering, an unrelated app's suite (e.g. a
  near-instant Dependabot check) could send a premature "CI PASSED" before
  the real CI workflow finishes, or an unrelated app's failure could send
  a spurious "CI FAILED". Shipped: gate on
  `check_suite.app.slug === 'github-actions'` before processing.
- **New, not anticipated by this spec — multiple associated PRs (reagent
  P2 on #48):** the original design (like `ci-failure.ts`) only processed
  `pull_requests[0]`, silently dropping stacked PRs sharing a head commit.
  Shipped: `processCICompleteEvent` now takes a
  `Map<number, PullRequestDetails>` (one `fetchPRDetails` call per
  associated PR, done in `handler.ts` before calling the processor) and
  returns a `notifications[]` array — every PR is processed independently,
  one PR failing its trust/agent-resolution gate does not block the
  others.
- **§6 infra changes:** all four items landed as described. Item 2 (verify
  the upstream GitHub webhook subscription actually includes
  `check_suite`) could not be confirmed from either repo's code — verified
  empirically post-deploy instead (see PR #48's description for the
  end-to-end check).
