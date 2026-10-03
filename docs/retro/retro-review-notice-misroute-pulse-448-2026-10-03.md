# RCA: ReAgent's review notices for Agent2's pulse #448 went to AgentY, never to Agent2

**Status:** retro
**Date:** 2026-10-03
**Owner:** AgentY, at the owner's request ("did you debug what is causing the misroute? pull in latest from a5af/reagent, write a rca to file")
**Area:** `agentmux-cloud/muxbus/consumers/github/` (the github-consumer Lambda, `agentmux-github-consumer`)
**Builds on:** `docs/reports/REPORT_REVIEW_NOTICE_ROUTED_TO_STALE_PR_AUTHOR_2026_10_03.md` (the first look, from code only). This RCA adds the production logs, the July history of the PR, how widespread the cause is, and a second defect that took away the backstop. It corrects one claim in that report (§7).

## 1. Summary

ReAgent reviewed `a5af/pulse#448` twice today. Both notices went to AgentY and neither went to Agent2, who owns the PR and was waiting on the approval to merge it.

The consumer takes the agent from the PR's **author** first and reads the `agent_id` tag in the PR body **only when the author does not resolve to an agent**. #448 was opened in July under the `AgentY-asaf` GitHub account, the account that most agents on this host were using to open PRs at the time. So the author resolves to `agenty`, and the tag `agent2` is never read.

The secondary check that adds the head-commit author also cannot help. Every GitHub API call the consumer makes has failed since at least 2026-09-28, because the Secrets Manager key it reads its token from does not exist.

ReAgent itself is not involved. It posts the review on GitHub, and the routing happens entirely in the consumer. `a5af/reagent` at `83206a1` (latest `main`, pulled for this RCA) has no agent-routing code.

## 2. Timeline (UTC)

| When | What | Source |
|---|---|---|
| 2026-07-06 19:04 | Commit `60d1a02e` on branch `agent2/remove-unused-monitoring-alarms`, git author `AgentY-asaf` | PR commits API |
| 2026-07-06 19:05 | PR #448 opened. GitHub author **`AgentY-asaf`**. Body already ends `agent_id=agent2` (first entry in the body's edit history) | PR API, GraphQL `userContentEdits` |
| 2026-07-06 19:06 | ReAgent reviews (later dismissed) | reviews API |
| 2026-10-03 14:49 | Agent2 rebases: commit `22bfbb1b`, git author `Agent2 <agent2@agentmux.local>` (no GitHub login). Body edited, still tagged `agent2` | commits API, edit history |
| 2026-10-03 14:50:07 | ReAgent: COMMENTED on `22bfbb1b` | reviews API |
| 2026-10-03 14:50:09 | Consumer: `PR author is agent: AgentY-asaf -> agenty`, then `Could not fetch a5af/pulse#448 to confirm review freshness`. No tag line, no head-commit line. Notice sent to `agenty` only | CloudWatch, request `b4cb2fc5` |
| 2026-10-03 19:42:40 | Agent2 pushes `4f504810` (same identity) | commits API |
| 2026-10-03 19:42:54 | ReAgent: APPROVED on `4f504810` | reviews API |
| 2026-10-03 19:42:57 | Consumer: the same three lines. Notice sent to `agenty` only | CloudWatch, request `d883c99d` |

The two consumer requests log nothing else between "Processing SNS message" and the end of the invocation, so the target set was `{agenty}` both times.

## 3. Root cause

### 3.1 The tag is ignored when the author resolves (the direct cause)

`events/review.ts`, `processReviewEvent`:

```ts
const prAuthorAgent = fromTrustedRepo ? getAgentId(prAuthorLogin) : undefined;   // :202
if (prAuthorAgent) {
  agentsToNotify.add(prAuthorAgent);                                             // tag never read
} else {
  const bodyAgentId = extractAgentIdFromBody(event.pull_request.body, ...);     // :207
  ...
}
```

`getAgentId("AgentY-asaf")` lowercases the login and matches `NUMBERED_PAT_PATTERN` (`/^agent([xya-g]|[1-5])-[a-z0-9]+$/`, `agent-mapping.ts`), so it returns `agenty`. This ordering is deliberate (`SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`: a standard agent identity is "unambiguous on its own"). That holds only if the author account belongs to the agent doing the work. The tag is the only routing field anyone can change after a PR is opened, and this rule means a correct tag can never override a wrong author.

### 3.2 Why the author is AgentY-asaf: a host-wide identity, June to August

#448 is not a one-off. A search of every PR authored by `AgentY-asaf` in `a5af` and `agentmuxai` (546 PRs, 2026-01-09 to 2026-09-20) found **135 whose own tag names another agent**:

| Tagged agent | PRs | | Repo | PRs |
|---|---|---|---|---|
| agent3 | 39 | | agentmuxai/agentmux | 119 |
| agent2 | 36 | | a5af/reagent | 8 |
| agent1 | 29 | | a5af/shared-infrastructure | 4 |
| camper / camper-0622h | 10 | | agentmuxai/agentmux-docs | 2 |
| loap / loap3 / loap-06183 | 7 | | a5af/dev-tools | 1 |
| lark / lark-06209 | 5 | | a5af/pulse | 1 (#448) |
| agentx | 4 | | | |
| korp | 3 | | | |
| smike, manoz | 1 each | | | |

They were created between 2026-06-30 and 2026-08-27 (15 in June, 110 in July, 10 in August). A further 263 have no tag at all, so the real count is higher. In that period, agents on this host opened PRs through a GitHub session that was AgentY's, not their own. `retro-agentmux-corp-pr5-shared-gh-cli-session-misattribution-2026-08-24.md` traced two ways that happened (a shared `gh` login, and an `AgentX GitHub` account linked to the wrong agent and injected as `GH_TOKEN`). That era has ended: agents now act as their own GitHub Apps through `gh-agent`, and the last PR authored by `AgentY-asaf` is from 2026-09-20.

**But the PRs outlive the era.** The author of a PR can never change. Any of those PRs that gets reviewed again is routed to AgentY. Today #448 is the only one still open, which is why this surfaced only now: Agent2 revived its stale July PR this afternoon.

### 3.3 The backstop is dead: the consumer cannot call GitHub

After the author/tag step, `processReviewEvent` also adds the **head-commit author's** agent. The handler fetches it with `fetchCommitAuthor`, and checks the PR's live state with `fetchPRDetails` (`handler.ts:355-407`). Both get their token from `getGithubToken()` (`handler.ts:110-116`), which reads `services/infra` → `github.token`.

That key does not exist. A key-name-only read of `services/infra` (values not printed) shows no `github` key; the only GitHub-related keys are `gh-reporter-app-id`, `gh-reporter-app-key` and `github-router-webhook-secret`. `getGithubToken` throws, both fetchers catch and return `undefined`, and nothing is logged except the freshness warning.

CloudWatch, from 2026-09-28 (where the log group starts) to now:

| Day | Review events | Routed by author | Routed by committer | Freshness fetch failed |
|---|---|---|---|---|
| 09-28 | 5 | 0 | 0 | 1 |
| 09-29 | 109 | 68 | 0 | 87 |
| 09-30 | 333 | 84 | 0 | 258 |
| 10-01 | 349 | 104 | 0 | 248 |
| 10-02 | 231 | 94 | 0 | 203 |
| 10-03 | 306 | 190 | 0 | 234 |

Over that window the freshness fetch failed 1,031 times and never once succeeded: there is no `(live)` staleness skip in the logs at all, only 242 `(payload)` ones. It failed for every repo, including the public `agentmuxai/agentmux` (814 times), which an unauthenticated call could read. That points at the token, not at repo permissions. `REPORT_GITHUB_CONNECT_AND_SIGN_IN_2026_09_30.md` §9 already noted that the key was missing; nothing changed afterwards.

For #448 the committer check would not have helped even with a working token, because Agent2's commits carry `agent2@agentmux.local`, which has no GitHub login. So 3.3 did not cause this misroute. It does mean the consumer has had no second opinion for any PR, and that two other features are silently off:
- **Live staleness:** a review of a commit that has since been superseded, or of a PR merged in the meantime, is still delivered.
- **Urgent priority:** "changes requested" is marked urgent only when the live PR was fetched, so every review notice goes out at normal priority.

### 3.4 Contributing: any `agenty-*` login maps to AgentY

`NUMBERED_PAT_PATTERN` accepts `agent<slot>-<anything>` on any host, by design (`SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`, host-agnostic). So a retired PAT-era account still maps to the live agent in that slot. This is not wrong in itself; it becomes wrong only together with 3.1.

## 4. Impact

- **Missed message:** Agent2 never got either review. Its standing rule is to merge on approval, so #448 sits approved with nobody acting on it. (I did not message Agent2 or merge it.)
- **False messages:** AgentY got two notices for a PR it does not own. Neither asked for an action. A "changes requested" notice could have been acted on by the wrong agent.
- **Latent:** any of the 135 (or more) misattributed PRs that is reopened, or reviewed again, repeats this.
- **Separately, from 3.3:** stale review notices are delivered, and no review notice has been urgent since at least 2026-09-28. Other consumer paths that call GitHub (required-check lookups for CI notices, `handler.ts:409-600`) use the same token and need checking.

## 5. What did not cause it

- **ReAgent / `a5af/reagent`.** It reviews the PR and posts to GitHub only; the notice text and routing are the consumer's.
- **The body tag.** It was correct (`agent2`) from the first version in July, and the trusted-head-repo gate passes (`a5af`).
- **Machine-wide git identity.** The commits that triggered both reviews were authored as Agent2. AgentMux already sets a per-agent git identity at spawn (`GIT_AUTHOR_NAME`/`EMAIL`, `GIT_COMMITTER_*`, `crates/srv/src/server/agent_handlers/input.rs:179`; this session's env shows `AgentY <agenty@agentmux.local>`).

## 6. Fixes

| # | Fix | Where | Effect |
|---|---|---|---|
| **F1** | **A valid tag that names an agent wins over the author.** Read the tag whenever the head repo is trusted; if it names an agent, notify that agent, and use the author only when there is no valid tag. Log when author and tag disagree | `events/review.ts` (and the same author-first code in the other event handlers, if present) | Fixes #448 and all 135 latent PRs. Adds no trust: the tag is read under the same trusted-repo gate as the author. Recommended |
| **F2** | **Give the consumer a working GitHub credential**: mint an installation token from a GitHub App whose key is in `services/infra` (the `gh-reporter` App is already there), instead of reading the missing `github.token` | `handler.ts:getGithubToken` | Restores committer routing, live staleness and urgent priority. Recommended |
| F3 | Make `getGithubToken` failures loud: one `error` log per invocation, plus a CloudWatch metric/alarm | `handler.ts` | Keeps this from being silent for another week |
| F4 | When author and tag disagree, add one line to the notice ("PR author `AgentY-asaf`, tagged for `agent2`") | notice formatting | Helps whoever receives it |
| F5 | Tests for F1: author `AgentY-asaf` + tag `agent2` → `agent2` only; App author + no tag → author agent; untrusted head repo → tag ignored; invalid tag → ignored; tag equal to author → one notice | `events/review.test.ts` | |

F1 and F2 change production behaviour and only take effect once the relay/consumer is promoted. Per the standing rule, that needs the owner's OK.

**Immediate, no code:** Agent2 can be told that #448 is approved, or the owner can merge it.

## 7. Correction to the earlier report

`REPORT_REVIEW_NOTICE_ROUTED_TO_STALE_PR_AUTHOR_2026_10_03.md` listed this host's global git identity as cause 3 and proposed F4 "AgentMux sets a per-agent git identity at spawn". **AgentMux already does this** (`input.rs:179`). The global `~/.gitconfig` (`AgentY-asaf`) now affects only processes AgentMux did not start. That report also said "the consumer's logs are not available to me"; they are (CloudWatch, `/aws/lambda/agentmux-github-consumer`, `us-east-1`), and §2 and §3.3 here come from them.

## 8. Not verified

- **Which mechanism produced #448's July authorship.** Shared `gh` login or a cross-agent `GH_TOKEN` binding: both are documented for that period, and neither can be checked now.
- **The 263 untagged `AgentY-asaf` PRs.** Their owners can't be told apart from AgentY's own without reading each branch name.
- **Logs before 2026-09-28.** The log group starts there (the consumer moved stacks on 2026-09-27, `REPORT_AGENTMUX_PROD_MIGRATION_2026_09_27.md`). Whether the GitHub fetches worked before the move is unknown.
- **The exact HTTP error.** I inferred the token failure from the missing key and the 100% failure rate. I did not call GitHub with any credential.
