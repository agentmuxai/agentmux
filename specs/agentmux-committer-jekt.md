# Spec: AgentMux Jekt for Recent Committer

**Author:** Agent4
**Date:** 2026-01-21
**Status:** ✅ COMPLETED
**Location:** `agentmux/consumers/github/`

---

## Problem

Previously, the AgentMux consumer only sent jekt notifications to the **PR author** when a review was submitted. This missed an important case:

**Scenario:** Agent4 pushes commits to a PR authored by `a5af` (human). When the PR gets reviewed, Agent4 should be notified since they're actively working on it.

## Solution Implemented

### Architecture

The AgentMux consumer was migrated from github-router to the agentmux repo and now receives events via SNS subscription:

```
GitHub webhook → Router → SNS fan-out
                              └→ agentmux-github-consumer Lambda
                                  ├── pull_request (merges)
                                  ├── pull_request_review (reviews)
                                  └── check_run (CI failures)
```

### Implementation Details

**Files created/modified:**
- `consumers/github/handler.ts` - Main Lambda handler (SNS + API Gateway)
- `consumers/github/events/review.ts` - PR review event processing
- `consumers/github/events/ci-failure.ts` - CI failure event processing
- `consumers/github/events/merge.ts` - PR merge event processing
- `consumers/github/agent-mapping.ts` - GitHub username to agent ID mapping
- `infrastructure/lib/agentmux-stack.ts` - CDK stack with SNS subscription

**Committer jekt logic (review.ts):**
```typescript
export function processReviewEvent(
  event: PullRequestReviewEvent,
  headCommitAuthor?: string
): ReviewHandlerResult {
  const agentsToNotify = new Set<string>();

  // 1. Check PR author
  const prAuthorAgent = getAgentId(event.pull_request.user.login);
  if (prAuthorAgent) agentsToNotify.add(prAuthorAgent);

  // 2. Check most recent committer
  if (headCommitAuthor) {
    const committerAgent = getAgentId(headCommitAuthor);
    if (committerAgent) agentsToNotify.add(committerAgent);
  }

  return {
    shouldNotify: agentsToNotify.size > 0,
    targetAgentIds: Array.from(agentsToNotify),
    message: formatReviewNotification(event),
  };
}
```

**Agent mapping (agent-mapping.ts):**
```typescript
// Static mappings (for bots, special accounts)
const GITHUB_TO_AGENT_MAP: Record<string, string> = {
  'agentx-workflow[bot]': 'AgentX',
  'agentx-asaf': 'AgentX',
  // ... etc
};

// Dynamic patterns for case-insensitive matching
const GITHUB_APP_PATTERN = /^agent([xya-g]|\d)-workflow\[bot\]$/;
const PAT_PATTERN = /^agent([xya-g]|\d)-asaf$/;
```

## Deployment

| Component | PR | Status |
|-----------|-----|--------|
| AgentMux consumer | #80 | ✅ Merged & Deployed |
| Router cleanup | #227 | 🔄 Pending review |

## Testing

Verified working via SNS subscription:
1. PR review submitted on a5af/marketdata
2. Consumer received via SNS fan-out
3. Jekt sent to PR author (if agent)
4. Jekt sent to head commit author (if agent and different)

## Edge Cases Handled

| Case | Behavior |
|------|----------|
| Author = Committer (both agent) | Single jekt (deduplicated via Set) |
| Author = agent, Committer = human | Jekt to author only |
| Author = human, Committer = agent | Jekt to committer only |
| Both human | No jekt |
| Commit fetch fails | Log warning, still notify author if agent |
| Self-review (reviewer = author) | Still send (agent might want confirmation) |

---

## Related PRs

- **agentmux#80** - Consumer implementation with committer jekt
- **shared-infrastructure#227** - Router cleanup (remove old consumers)
- **shared-infrastructure#225** - ReAgent SNS migration
- **shared-infrastructure#226** - SSL fix
