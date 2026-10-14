# Plan — the Codex review gate survives a transient GitHub error, and the CI script clean-ups around it

**Date:** 2026-10-14
**Status:** active — step 1 merged (#4897) and step 2 merged (#4900), both verified in production on 2026-10-14; step 3 in review (#4903); step 4 built, opened after step 3 merges. Four steps, one PR each (§4).
**Author:** Clamk@narko
**Touches:** `scripts/ci-codex-review-gate.mjs`, `scripts/ci-codex-review-gate.test.mjs`, `.github/workflows/codex-review-gate.yml`, a new `scripts/lib/`, and the "is this script being run directly" check in 16 scripts (step 4).

## 1. What happened (#4863)

#4863 was approved by muxreview on its head `3cd12698f` with every CI check green, and sat unmergeable for 8 h 40 min on the required `Codex review` status, which read "Waiting for Codex on 3cd12698fe: muxreview asks after approving, or comment '@muxreview codex re-review'".

Timeline (UTC, 2026-10-14):

| Time | Event |
|---|---|
| 07:14:32 | Push → `pull_request_target` run of the gate sets `Codex review` to **pending** ("Waiting for Codex"). Correct. |
| 07:15:40 | muxreview approves and asks Codex (`@codex review` with the trigger marker for `3cd12698fe`). |
| 07:15:48 | Codex answers with its out-of-quota notice ("You have reached your Codex usage limits…"). |
| 07:15:51 | `issue_comment` run of the gate starts. It evaluates correctly: `#4863 3cd12698fe: success — Codex is out of review quota; 3cd12698fe passes without it`. |
| 07:15:59 | Its `POST /repos/agentmuxai/agentmux/statuses/3cd12698f…` gets **HTTP 500** from GitHub. `gh()` throws, the run exits 1. |
| 07:16 → 15:54 | Nothing re-runs the gate. The head keeps the 07:14 **pending** status. Auto-merge waits on it. |
| 15:54:56 | The failed job is re-run by hand (`gh run rerun 38527192994 --failed`). It posts **success** at 15:55. |
| 15:56:07 | #4863 merges as `bc089c8d0`. |

So the gate's logic was right, and the quota handling was right. Three things in our code turned one GitHub 500 into an 8 h 40 min block:

1. **No retry.** `gh()` (`scripts/ci-codex-review-gate.mjs:348`) makes one attempt and throws on any non-2xx, including 5xx and rate limits.
2. **The failure is invisible on the PR.** An `issue_comment` run belongs to the default branch, not to the PR, so the failed `evaluate` never appears in the PR's checks. What the PR shows is the previous status, "pending — Waiting for Codex", which reads as normal waiting, not as an error.
3. **Nothing ever looks again.** The gate runs only on events. Codex's comment was the last event the PR would get, so a lost evaluation of it is lost for good. The same is true of a dropped webhook, or a push-time run that fails before posting (the new head then has no `Codex review` status at all, and the required check shows "Expected — waiting" forever).

A fourth, quieter failure mode of the same shape: `changedFiles()` (`:378`) swallows every error, including a 5xx, and returns "unknown", so a transient error while comparing makes a Codex OK fail to carry across a docs-only push. The head is then pending until something else happens, with the same misleading description.

## 2. Goals

- A single transient GitHub error (5xx, rate limit, dropped connection) never leaves a PR's `Codex review` status wrong.
- Whatever slips through anyway (an outage longer than the retries, a dropped webhook, a cancelled run) heals on its own within about 30 minutes, without anyone noticing and re-running a job, and without spending a Codex review.
- While here: the gate's own duplication, stale names and comments, and the repo-wide "run main?" check, cleaned up, each in a PR small enough to review.

Non-goals: changing *what* the gate decides (§1 shows the decision was right); changing when muxreview asks Codex; converting the `gh api` shell steps in other workflows (§5).

## 3. Design

### 3.1 A small GitHub REST client with retry — `scripts/lib/github-rest.mjs`

Move `gh()`, `ghAll()` and the JSON handling out of the gate into one module:

```js
export function createGitHubClient({ token, fetch = globalThis.fetch, sleep = defaultSleep, log = console.log,
                                     maxAttempts = 4, maxWaitMs = 60_000 }) {
    return { request, json, paginate };   // request(path, init) -> Response; json(path, init) -> body; paginate(path) -> items
}
export class GitHubError extends Error { status; method; path; }   // thrown after the last attempt
```

Retry rules (`isRetryable`):

| Response | Retry? | Wait |
|---|---|---|
| network error (`fetch` throws) | yes | backoff |
| 500, 502, 503, 504 | yes | backoff |
| 429, or 403 with `retry-after` / `x-ratelimit-remaining: 0` | yes, if the wait is ≤ `maxWaitMs` | `retry-after` seconds, else until `x-ratelimit-reset` |
| any other 4xx (401, 404, 422, …) | no | — |

Backoff: 2 s, 4 s, 8 s, each plus 0–1 s jitter; at most 4 attempts. Worst case for one call is about 15 s of waiting (60 s when GitHub names a longer wait), well inside the job's 5-minute timeout. Each retry logs one line (`GET /repos/…/pulls/4863 -> HTTP 500; retry 2/4 in 4.3s`), so a run that recovered says so in its log.

**Retrying every method is safe here, and the module says why.** Every write this workflow makes is idempotent in effect: a commit status is append-only and the latest per context wins, so a POST that succeeded server-side and still returned 500 just records the same status twice; a duplicate `workflow_dispatch` (§3.2) starts a second run that the per-PR concurrency group cancels. A future caller with a non-idempotent write passes `{ retry: false }`.

`paginate()` keeps today's behaviour (`per_page=100` until a short page).

`changedFiles()` stops swallowing everything: a 404 or 422 from compare (the reviewed commit was force-pushed away, or isn't comparable) still means "unknown"; any other error after retries propagates and fails the run. Failing is better than posting a wrong pending, because the sweep (§3.2) can only heal a status that is *older* than the input it missed.

**The sparse checkout must list the new module.** The workflow checks out only `scripts/ci-codex-review-gate.mjs` from the default branch (so a PR cannot change the gate that judges it). A relative import the checkout doesn't list passes every test, passes CI, merges, and then crashes the gate on every PR: every PR in the repo blocked. So:

- `sparse-checkout:` lists `scripts/ci-codex-review-gate.mjs` and `scripts/lib/github-rest.mjs` (step 4 adds `scripts/lib/is-main.mjs`). Still main's copy only, still no PR code: the security property is unchanged.
- A test (§3.4) reads the gate's relative imports and fails if the workflow's sparse list misses one.

**Found while verifying step 1: the script and the list must come from the same commit.** The checkout used `ref: <default branch>`, which is the branch's head when the job *starts*, while GitHub reads the workflow file (and so the sparse list) from the commit the *event* saw. An `issue_comment` on #4898 at 16:19:23 was queued against `d1a38e4cd`, the parent of #4897's merge; its job started at 16:19:32, two seconds after the merge, checked out the new script with the old one-file list, and failed with `ERR_MODULE_NOT_FOUND`. (A later event re-evaluated #4898 within two minutes, so nothing stuck.) Any merge that changes the gate's imports opens that window, step 4's included. Step 2 therefore checks out `github.sha`, the event's own commit, whenever `github.ref` is the default branch (pull_request_target into it, issue_comment, workflow_dispatch on it). It keeps the default branch for `pull_request_review`, whose `github.sha` is the PR's merge commit, so PR code still never runs. A test pins the expression.

### 3.2 A scheduled sweep that re-evaluates only PRs that missed an input

Add a `schedule` trigger to `codex-review-gate.yml` (`cron: '17,47 * * * *'`, off the top of the hour, where GitHub delays scheduled runs most) and a second job, `sweep`, in its own concurrency group `codex-review-gate-sweep`, with `actions: write` added to that job only.

The sweep **does not evaluate or post statuses itself.** For each PR it decides needs a look, it dispatches the existing `workflow_dispatch` path (`POST /repos/{repo}/actions/workflows/codex-review-gate.yml/dispatches`, `inputs: { pr }`, `ref: main`). Dispatches made with `GITHUB_TOKEN` do start workflow runs (GitHub's documented exception to the "token events don't trigger workflows" rule), and each runs in that PR's own `codex-review-gate-<pr>` concurrency group, serialized with event-driven runs. That is why the sweep dispatches instead of evaluating: a sweep evaluating dozens of PRs in one job can't join their per-PR groups, so it could read a PR just before Codex's comment and post a stale pending over the fresh success an event run posted a second later.

Which PRs need a look (`sweepReason`, a pure function next to `evaluateCodexGate`), among open, non-draft PRs:

1. **No `Codex review` status on the head at all.** The push-time run failed or never ran.
2. **Status is `pending`, and a gate input is newer than the status:** a Codex comment or review, or a muxreview skip note, created or edited after the status's `updated_at`. "Gate input" is the same predicate the workflow's `if:` encodes, exported from the gate module (`isGateInput(comment)`; a review counts when Codex wrote it), so the sweep, the event filter and the tests can't disagree about it.

A PR legitimately waiting (no Codex answer yet, muxreview not yet approved) matches neither rule, so it is not dispatched. Typical sweep: one call for open PRs, then for each one status read plus its issue comments and reviews: about 3 calls a PR, so ~60–90 requests for 20–30 open PRs, twice an hour, against `GITHUB_TOKEN`'s 1,000 an hour. It costs no Codex reviews (the gate never asks Codex), and Actions minutes on this public repo are free on standard runners.

Worst case after this plan: a missed input heals within ~30 minutes plus one gate run.

The sweep is a mode of the same script (`SWEEP=1`), reusing the client and the predicates. `main()` runs `sweep(client, repo, { ref })` when `SWEEP` is set, and today's single-PR evaluation otherwise.

### 3.3 Re-evaluating by hand, documented

Until §3.2 lands, and as the documented escape afterwards:

```bash
gh-agent workflow run codex-review-gate.yml -f pr=<number>      # re-evaluate one PR now
```

It reads state and posts a status; it never asks Codex, so unlike `@muxreview codex re-review` it costs no review quota (a manual re-review while Codex is out of quota just gets another quota notice). The script header gets a short "If the status looks stuck" paragraph naming it; the 140-character status description stays as is.

### 3.4 Tests

`scripts/lib/github-rest.test.mjs` (fake `fetch` returning a scripted sequence, fake `sleep` recording waits):
- a 500 then a 200: one retry, returns the 200, logs one retry line;
- four 500s: throws `GitHubError` with status 500 after four attempts;
- 404 and 422: no retry, thrown at once;
- a thrown network error then a 200: retried;
- 429 with `retry-after: 3`: waits 3 s; with `retry-after: 600`: not retried (over `maxWaitMs`);
- 403 with `x-ratelimit-remaining: 0` and a reset 5 s out: waits until the reset;
- `retry: false`: a 500 throws at once;
- `paginate`: stops on a short page, and a page that fails transiently is retried, not dropped.

`scripts/ci-codex-review-gate.test.mjs`, added:
- `sweepReason`: missing status → candidate; pending with a Codex quota notice newer than it → candidate; pending with only an older Codex answer → not; pending with a newer comment by someone else → not; success → not, whatever came after; an edited muxreview skip note whose `updated_at` is newer → candidate; draft PR → not.
- **Workflow pins** (read the YAML as text, no parser needed): the `if:` names `CODEX_LOGIN`, every login in `SKIP_AUTHORS` and the `reagent:codex-skipped` marker (the 2026-10-07 App rename had to edit both places by hand); and every relative import of the gate module, followed transitively, appears in `sparse-checkout:` (§3.1).
- `changedFiles`: 404 → `null`; a 500 that persists after retries → throws.

### 3.5 DRY and modularity in the gate itself (step 3)

The gate splits cleanly into policy (parse Codex output, decide a status: pure, ~340 lines, already well tested) and I/O (the client, `main`). §3.1 takes the client out; these tidy the policy:

| # | Today | Change |
|---|---|---|
| a | `c.user?.login === CODEX_LOGIN` written out 4 times (`:139`, `:146`, `:216`, `:229`) | `isCodex(x)` |
| b | three hand-written sort comparators on `at` (`:151` with a tie-break on `order`, `:203`, `:238`) | `byAt(a, b)` (and `byAt` then `order` where the tie-break matters) |
| c | the "this sha names that commit" prefix test in three spellings (`:157`, `:255`, `:282`) | `namesCommit(fullSha, shortSha)`, lower-casing in one place |
| d | `reviewedCommit(body)` called twice on the same comment (`:142`); `skipNote(c)` three times (`:201–202`) | compute once (`flatMap` in `reagentSkips`) |
| e | the quota-skip `if` ladder (`:290–299`) | a `SKIP_DESCRIPTION` table keyed by reason (`quota`, `round-cap`, `docs-only`); the unknown-reason case is already rejected by the regex |
| f | `skippedHead()` and `SKIP_AUTHOR` exported only for one test each | drop both exports; the tests use `skipNote()` and `[...SKIP_AUTHORS][0]` |
| g | `reagentSkips`, and comments calling the bot "reagent" or "reagentx-workflow[bot] only" (`:33`, `:54`, `:98`, `:103`) | `muxreviewSkips`; comments say muxreview and name both logins. **The `reagent:codex-*` markers stay**: they are the wire format muxreview writes and must not change on one side only |
| h | comments at `:29`, `:39`, `:55` and `:98` name a file and a spec inside muxreview's private repository | reword to "muxreview's Codex policy"; this is a public repository |
| i | the run-as-script guard (`:425`) is its own variant | step 4's shared `isMain` |

Deliberately left alone:
- `main()` computes Codex's outputs twice (once for `latestCodexOutput`, again inside `evaluateCodexGate`). It's a few dozen comments; merging them would widen `evaluateCodexGate`'s tested signature for no measurable gain.
- `isDocsOnlyPath` here and the docs rule in `ci-classify-changes.mjs` look alike but differ on purpose (this one treats `prompts/`, `CLAUDE.md` and `AGENTS.md` as code; that one treats every `.md` outside `.github/`/`scripts/`/`tools/` as docs). Sharing them would couple two rules that have to be free to differ. Both files already say so. muxreview keeps its own mirror of this rule and of the markers; a change to either here still needs the matching change there, as today.

### 3.6 One `isMain` for every script (step 4)

Sixteen scripts each write their own "am I being run directly?" check, in five variants:

| Variant | Scripts | Correct? |
|---|---|---|
| `import.meta.url === pathToFileURL(argv[1]).href` | check-comment-hygiene, docs-stale-sweep, gnome-grabs, verify-shortcuts-l3-linux | yes |
| `fileURLToPath(import.meta.url) === resolve(argv[1])` | check-tsrs-serde-warnings, gen-mux-events, gen-rpc-stubs, link-md-paths | yes (not through a symlink) |
| `realpathSync(resolve(argv[1]))` compare | gen-docs-index | yes, symlinks too |
| `import.meta.url.endsWith(<basename of argv[1]>)` | check-file-sizes, check-ui-primitives, ci-classify-changes, ci-required-check, sync-lockfile-version | mostly: a name with a space or other escaped character never matches, and any same-named file anywhere does |
| `` import.meta.url === `file://${argv[1]…}` `` | check-new-spec-status, ci-codex-review-gate (which adds an `endsWith` fallback) | **no, on Windows** |

The last row is a real bug, checked on this machine with Node 24. On Windows `import.meta.url` is `file:///C:/…` and the template builds `file://C:/…`, so the test is false. `node scripts/check-new-spec-status.mjs` run locally on Windows therefore never calls `main()`, prints nothing and exits 0: a silent pass. CI is unaffected because it runs the check on Linux. The gate survives only through its `endsWith` fallback.

`scripts/lib/is-main.mjs`:

```js
export function isMain(metaUrl, argv1 = process.argv[1]) {
    if (!argv1) return false;
    try { return realpathSync(fileURLToPath(metaUrl)) === realpathSync(resolve(argv1)); }
    catch { return false; }
}
```

All 16 call it. Tests: a Windows-style and a POSIX path, a path with a space, a symlinked invocation (skipped where symlinks need privileges), no `argv[1]`, and being imported by another module (false).

## 4. Steps

Each step is one PR with its own changeset. Step 1 fixes the live bug and goes first; 2–4 can follow in any order after it, but step 4 touches the gate's sparse list again, so land it after step 1.

| Step | PR | Contents | Size |
|---|---|---|---|
| 1 | fix(ci): the Codex review gate retries transient GitHub errors | §3.1, its tests (§3.4), the workflow pins test, the §3.3 paragraph, `changedFiles` error split | ~250 lines, half tests |
| 2 | fix(ci): a scheduled sweep re-runs the Codex gate for PRs that missed an input | §3.2 (`sweepReason`, `sweep` and their tests, `actions: write` on the sweep job only), and §3.1's same-commit checkout | ~300 lines, most of it tests |
| 3 | refactor(ci): tidy the Codex gate's policy code | §3.5 a–h; no behaviour change, the existing tests unchanged except (f) | slightly net negative |
| 4 | fix(scripts): one `isMain` for every script, so check-new-spec-status runs on Windows | §3.6 | the new module and its test, plus a few lines in each of the 16 scripts |

## 5. Out of scope, with reasons

- **The `gh api` steps in other workflows** (`changeset.yml`, `ci-pr.yml`, `no-coauthor-trailers.yml`, `release.yml`, `srv-image.yml`, `reagent-review-log-guard.yml`, `docs-stale-issue-comment.sh`). They run on the PR's own events, so a failure shows as a red check on the PR and the next push or a re-run clears it. That is the visibility the Codex gate lacked; a transient error there costs a click, not an afternoon. Revisit only if they start failing for this reason.
- **Making the gate post `error` when it can't post `success`.** Posting needs the API that just failed; the retries and the sweep cover the same ground without that circularity.
- **Anything in muxreview.** Its quota handling worked as designed on #4863.

## 6. Rollout and verification

1. Before merging step 1: `DRY_RUN=1 GITHUB_REPOSITORY=agentmuxai/agentmux PR_NUMBER=<an open PR> node scripts/ci-codex-review-gate.mjs` from the branch, against a real PR, prints a decision; the unit tests cover the retries.
2. Right after each of steps 1, 2 and 4 merges (the ones that change what the workflow runs): `gh-agent workflow run codex-review-gate.yml -f pr=<an open PR>` and confirm the run succeeds and the PR's `Codex review` status is fresh. **The gate runs main's copy on every PR, so a broken gate blocks every PR in the repo; this check is not optional.** If it fails, revert the PR first and investigate second.
3. After step 2, the first two scheduled runs: confirm in their logs that they listed candidates and dispatched only those (expect zero most of the time).
4. When this ships with step 1, set **Status:** to `active` and list merged steps here, as other plans do (`check-new-spec-status.mjs` refuses a `proposed` plan shipped beside the code that cites it).
