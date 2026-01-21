# Spec: AgentMux Jekt for Recent Committer

**Author:** Agent4
**Date:** 2026-01-21
**Status:** Draft
**Location:** `reagent/lambdas/review_notifier.py` (NEW)

---

## Problem

Currently, the AgentMux consumer only sends jekt notifications to the **PR author** when a review is submitted. This misses an important case:

**Scenario:** Agent4 pushes commits to a PR authored by `a5af` (human). When the PR gets reviewed, Agent4 should be notified since they're actively working on it.

## Current Architecture

```
GitHub webhook → Router → Direct agentmux consumer call
                      └→ SNS fan-out → ReAgent sns_consumer (pull_request, issues)
```

## Proposed Architecture

Move agentmux notifications to ReAgent as an SNS consumer (consistent with SNS migration):

```
GitHub webhook → Router → SNS fan-out
                              └→ ReAgent sns_consumer (pull_request, issues)
                              └→ ReAgent review_notifier (pull_request_review) ← NEW
```

## Current Behavior

```
PR review submitted
  └── Check PR author (pull_request.user.login)
       └── If agent → send jekt
       └── If human → skip (no notification)
```

## Proposed Behavior

```
PR review submitted
  └── Check PR author (pull_request.user.login)
  │    └── If agent → send jekt to author
  └── Check most recent committer (head commit author)
       └── If agent AND different from author → send jekt to committer
```

## Implementation

### 1. Add GitHub Token Caching

Reuse the pattern from `ci_failure.py`:

```python
_cached_github_token = None

def get_github_token() -> str:
    """Get GitHub token from Secrets Manager (with caching)."""
    global _cached_github_token
    if _cached_github_token is None:
        response = secrets_manager.get_secret_value(SecretId=WEBHOOK_SECRET_NAME)
        secret_data = json.loads(response['SecretString'])
        _cached_github_token = secret_data.get('github', {}).get('token')
        if not _cached_github_token:
            raise ValueError(f"GitHub token not found in {WEBHOOK_SECRET_NAME}")
    return _cached_github_token
```

### 2. Add Commit Fetch Function

```python
def fetch_head_commit_author(repo: str, sha: str) -> Optional[str]:
    """
    Fetch the author of a commit from GitHub API.

    Args:
        repo: Repository full name (e.g., "a5af/marketdata")
        sha: Commit SHA

    Returns:
        GitHub login of commit author, or None on failure
    """
    try:
        github_token = get_github_token()
        commit = get_json(
            f"https://api.github.com/repos/{repo}/commits/{sha}",
            headers={
                "Authorization": f"token {github_token}",
                "Accept": "application/vnd.github.v3+json"
            },
            timeout=5
        )
        # commit.author is the GitHub user who authored the commit
        # (different from commit.commit.author which is git config)
        return commit.get('author', {}).get('login')
    except HTTPError as e:
        logger.warning(f"Failed to fetch commit {sha}: {e}")
        return None
```

### 3. Modify route_to_agentmux()

Update the main function to notify both PR author and recent committer:

```python
def route_to_agentmux(event_type: str, payload: Dict, headers: Dict) -> Dict:
    # ... existing validation ...

    pull_request = payload.get('pull_request', {})
    review = payload.get('review', {})
    review_state = review.get('state', '').lower()

    # ... existing state validation ...

    # Collect agents to notify
    agents_to_notify = set()

    # 1. Check PR author
    pr_author_login = pull_request.get('user', {}).get('login')
    pr_author_agent = extract_agent_id_from_user(pr_author_login)
    if pr_author_agent:
        agents_to_notify.add(pr_author_agent)

    # 2. Check most recent committer
    repo = payload.get('repository', {}).get('full_name')
    head_sha = pull_request.get('head', {}).get('sha')
    if repo and head_sha:
        committer_login = fetch_head_commit_author(repo, head_sha)
        committer_agent = extract_agent_id_from_user(committer_login)
        if committer_agent:
            agents_to_notify.add(committer_agent)

    # Skip if no agents to notify
    if not agents_to_notify:
        logger.debug(f"No agents to notify: author={pr_author_login}, committer={committer_login}")
        return {"success": True, "agents_notified": [], "error": None}

    # ... build message ...

    # Send to all identified agents
    results = []
    for agent_id in agents_to_notify:
        try:
            response = send_jekt_message(agent_id, message, source_agent=source_agent, pr_number=pr_number)
            results.append({"agent_id": agent_id, "success": True, "injection_id": response.get('injection_id')})
        except HTTPError as e:
            results.append({"agent_id": agent_id, "success": False, "error": str(e)})

    return {
        "success": all(r["success"] for r in results),
        "agents_notified": results,
        "error": None
    }
```

### 4. Update Return Type

The return type changes from single agent to list:

**Before:**
```python
{
    "success": bool,
    "agent_id": str | None,
    "message_id": str | None,
    "error": str | None
}
```

**After:**
```python
{
    "success": bool,
    "agents_notified": [
        {"agent_id": str, "success": bool, "injection_id": str | None, "error": str | None}
    ],
    "error": str | None
}
```

## Edge Cases

| Case | Behavior |
|------|----------|
| Author = Committer (both agent) | Single jekt (deduplicated via set) |
| Author = agent, Committer = human | Jekt to author only |
| Author = human, Committer = agent | Jekt to committer only |
| Both human | No jekt |
| Commit fetch fails | Log warning, still notify author if agent |
| Reviewer = recipient | Still send (agent might want notification) |

## Dependencies

- `utils/http.py` - already has `get_json()` from SSL fix
- GitHub token in secrets (`github.token` in `WEBHOOK_SECRET_NAME`)

## Testing

1. **Unit test:** Mock GitHub API, verify both author and committer extracted
2. **Integration test:**
   - Create PR as human (`a5af`)
   - Push commit as agent (`Agent4-asaf`)
   - Submit review
   - Verify jekt sent to `agent4`

## Rollout

1. Implement changes
2. Create PR
3. Deploy via CDK
4. Test with marketdata PR #1 (push as agent, get review)

## Metrics

Add logging:
```python
logger.info(f"Agents to notify: {agents_to_notify} (author={pr_author_login}, committer={committer_login})")
```

---

## Appendix: Webhook Payload Reference

`pull_request_review` event contains:

```json
{
  "action": "submitted",
  "review": {
    "user": {"login": "reviewer"},
    "state": "approved",
    "html_url": "..."
  },
  "pull_request": {
    "number": 1,
    "title": "...",
    "user": {"login": "author"},
    "head": {
      "sha": "abc123",
      "ref": "branch-name"
    }
  },
  "repository": {
    "full_name": "a5af/marketdata"
  }
}
```

Note: `head.sha` is the latest commit, but we need to fetch commit details to get the author.
