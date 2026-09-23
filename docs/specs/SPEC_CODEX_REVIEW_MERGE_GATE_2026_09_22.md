# SPEC — block a merge until Codex has finished reviewing the head commit

**Date:** 2026-09-22
**Author:** AgentY
**Status:** proposed
**Triggered by:** PR #3513, merged 65 seconds before Codex's next review landed.
Five subsequent findings — four of them P1 — were raised against a PR that had
already closed, and the five commits fixing them never reached `main`. Recovered
in #3523.

---

## 1. The incident

| Time (UTC) | Event |
|---|---|
| 17:36:36 | `reagentx-workflow[bot]` — **CHANGES_REQUESTED** |
| 17:39:07 | `chatgpt-codex-connector[bot]` — review (P2) |
| 17:59:48 | `9404a718f` pushed, addressing both |
| 18:03:11 | `reagentx-workflow[bot]` — **APPROVED** |
| **18:07:27** | **merged by `a5af`** (`headRefOid: 9404a718f`) |
| 18:08:32 | Codex — **P1** ("Claim the eager spawn before launching the process") |
| 18:51:22 | Codex — P2 |
| 18:59:22 | Codex — P1 |
| 19:29:48 | Codex — P1 |
| 19:48:11 | Codex — P1 |
| 20:02:49 | Codex — "Didn't find any major issues." |

No `auto_merge_enabled` event appears in the timeline, so this was a direct
merge four minutes after one reviewer's approval — not an auto-merge race.

**reagent did not miss anything.** It reviewed, requested changes, saw them
fixed, and approved its own findings. Every orphaned fix came from Codex, a
*second* reviewer whose pass was still running. The defect is in the merge
policy: nothing required the second reviewer to have spoken.

Two aggravating factors, both worth designing against:

- **Pushes to a merged PR are silently useless.** Five fix commits were pushed
  to the branch after the merge. GitHub accepted every push. Nothing surfaced
  that the PR was closed, and the author did not notice for ~100 minutes.
- **Codex follows the merge commit, not the branch.** Every Codex pass from
  18:51 onward stamps `Reviewed commit: 00a61f1303` — the squash-merge commit
  on `main`, not the branch head. So the post-merge review cycle was reviewing
  `main`, and the fixes written in response to it were never reviewed at all.

## 2. What Codex actually signals

Documented from observed behaviour on #3513 and #3523, because OpenAI's own
docs do not specify the wire-level signals. **This section is the load-bearing
part of the spec — a gate can only key off what Codex actually emits.**

| Outcome | Mechanism | Identifiable by |
|---|---|---|
| In progress | 👀 reaction | Reaction only; carries no SHA |
| Findings | PR **review**, state `COMMENTED` | Body starts `### 💡 Codex Review`; inline comments carry `P1`/`P2` badges |
| Clean | Top-level **issue comment** | Body starts `Codex Review: Didn't find any major issues.` |

Both terminal forms carry a machine-readable `**Reviewed commit:** <short-sha>`
in the body. **That string is the only trustworthy binding between a Codex
verdict and a commit** — see §5.

Three properties matter for gate design:

**2.1 Codex never posts `APPROVED`.** All six reviews on #3513 and the one on
#3523 were `COMMENTED`. It cannot satisfy a required-approvals rule, by
construction.

**2.2 Codex only reviews PRs opened by human accounts.** Measured across the
recent corpus:

| PR | Author | Codex reviews |
|---|---|---|
| #3513, #3523 | `a5af` (human) | 6, 1 |
| #3508, #3511, #3514, #3516, #3517, #3518 | `*-workflow[bot]` | 0 |

Bot-authored PRs are the majority of this repo's traffic. **A gate that
unconditionally requires a Codex verdict would deadlock nearly every PR we
open.** This single fact rules out the naive design and is why §4 is
conditional.

The same asymmetry applies to triggering: per the github-consumer notification
(citing `a5af/reagent`'s `specs/eyes-emoji-reaction-spec.md`), Codex responds to
`@codex review` **only from the `a5af` account, not from bot mentions**. A
workflow authenticating as `GITHUB_TOKEN` posts as `github-actions[bot]` and
will be ignored; triggering therefore requires a PAT for `a5af`.

**2.3 Codex is nondeterministic across passes on identical code.** Commit
`00a61f1303` was reviewed five times. The first four each produced a *different*
P1/P2 finding; the fifth declared it clean. Same bytes, five passes, five
verdicts.

This bounds what the gate can honestly claim. It prevents "merged before Codex
spoke". It does **not** make a clean pass evidence of correctness — had the
18:51 pass been the only one, four real P1s would have gone unreported, and the
20:02 "no major issues" verdict is on a commit Codex itself had flagged P1
fourteen minutes earlier. **Do not let this gate become a reason to review
less.**

## 3. Why the obvious approaches don't work

**Required reviewers / CODEOWNERS — dead end.** CODEOWNERS accepts users and
teams; GitHub Apps are not a documented entity type there. Required approvals
need an `APPROVED` review, which Codex never posts. GitHub separately documents
that Copilot's reviews do not count toward required approvals. Three
independent reasons, any one sufficient.

**Merge queue — no help.** A PR enters the queue only after required checks
already pass, so the gate does its work before the queue is involved. Queue CI
runs on `merge_group`, which Codex does not review.

**Auto-merge — a hazard to design around, not a solution.** Auto-merge
evaluates against currently-known contexts, so a check created late can be
missed. Mitigated by naming the context in branch protection, which makes
GitHub treat it as missing-and-blocking rather than absent.

## 4. Design: one required status check

A single job, `codex-review-gate`, required by name in branch protection.

**4.1 Why a status check.** GitHub documents that a required check which is
never reported blocks the merge with "Waiting for status to be reported" —
missing is *not* passing. That is exactly the property the incident needed and
the only mechanism that provides it.

**4.2 The workflow must have no path or branch filters.** This repo has already
been bitten by the distinction (#3511, and the earlier "skipped matrix job
permanently blocked docs-only PRs" bug):

- A workflow skipped by `paths:`/`branches:` filters leaves its checks
  **pending forever** and blocks the merge.
- A *job* skipped by an `if:` condition reports **success** and does not block.

So: no filters on the workflow; skip via `if:` inside the job. Report one
aggregate check, never a matrix — matrix jobs report as `name (param)`.

**4.3 Logic.**

1. On `pull_request: opened | synchronize | reopened | ready_for_review`, post
   the check as `pending` as the very first step, so auto-merge cannot race a
   context that does not exist yet.
2. **If the PR author is a bot → report `success` immediately** with a reason
   string. Per §2.2 Codex will never review it, and blocking would deadlock the
   repo. This is the single most important branch in the design.
3. Otherwise resolve the head SHA and look for a Codex verdict bound to it:
   - a review by `chatgpt-codex-connector[bot]` whose body contains
     `Reviewed commit: <head-sha-prefix>`, **or**
   - an issue comment from the same actor starting `Codex Review: Didn't find
     any major issues.` with the same `Reviewed commit:`.
4. Found → `success`, naming which form and which SHA.
5. Not found → stay `pending`, re-evaluated on each `issue_comment` and
   `pull_request_review` event.
6. **Timeout → `success`, with the reason recorded** (§6.1).

**4.4 Match on the SHA, not on recency.** The incident's own transcript shows
why: after the merge, Codex kept posting verdicts stamped with a *different*
commit than the branch head. A gate keying off "the most recent Codex comment"
would have passed on a verdict about entirely different code. `Reviewed commit:`
is the binding; a 👍 reaction is not, because reactions carry no SHA.

## 5. Failure modes this deliberately accepts

| Mode | Behaviour | Rationale |
|---|---|---|
| Codex outage | Merges after timeout | Fail-closed blocks the whole repo on a third-party outage |
| Bot-authored PR | Never gated | Codex will not review it (§2.2); gating is unenforceable |
| Codex reviews a stale SHA | Stays pending until it reviews the head | Correct — that is the incident |
| Codex misses a real bug | Merges | Out of scope; the gate proves Codex *spoke*, not that the code is right (§2.3) |

## 6. Open questions

**6.1 Fail-open or fail-closed on timeout?** Recommended fail-open, because
fail-closed converts an OpenAI outage into a repo-wide freeze. This is a real
tradeoff and the reason the gate is worth less than it first appears: a merge
that beats the timeout is indistinguishable, to the gate, from a merge Codex
approved. Timeout length should be set from observed Codex latency (2m54s for
reagent on #3523; Codex's own passes on #3513 ranged from under a minute to
~14 minutes between triggers). A 20-minute timeout is a starting point, not a
measured value — **measure before fixing it.**

**6.2 Should the gate trigger Codex, not just wait for it?** It could post
`@codex review` itself, but only from a PAT for `a5af` (§2.2). That turns a
read-only gate into one holding a human's credential, and makes the `a5af`
account the author of automated comments. Probably not worth it; prefer relying
on Codex's own auto-trigger for human-authored PRs.

**6.3 Should this extend to reagent?** reagent *does* post `APPROVED`, so it can
be handled by ordinary required-approvals branch protection. No custom gate
needed. Worth confirming that required approvals is actually enabled — the
incident shows a merge proceeding four minutes after one approval, which is
consistent with it being enabled, but that was never verified.

**6.4 Should a merged PR reject further pushes?** Out of scope here, but the
incident's second-order cost — 100 minutes of work pushed to a closed PR — would
be cheaply prevented by deleting the head branch on merge, which GitHub can do
automatically.

## 7. Scope

**In scope:** one workflow file and one branch-protection context name.

**Out of scope:**
- Making Codex's verdict *reliable* (§2.3) — it is advisory by design, and
  OpenAI documents that its reviews "don't replace tests, branch protections, or
  required approvals".
- Gating on reagent (§6.3).
- Anything about Codex's finding quality or triage.

## 8. Testing

- A human-authored PR with no Codex verdict for its head SHA → check pending,
  merge blocked in the UI.
- Same PR after a Codex verdict stamped with that SHA → check green.
- Push a new commit → check returns to pending (the old verdict names the old
  SHA).
- A bot-authored PR → check green immediately, reason recorded.
- A docs-only PR → check green, **not** stuck pending (the §4.2 regression).
- Timeout path → green, reason recorded.
