# SPEC: jekt notification when a PR's CI run completes (pass or fail)

**Date:** 2026-08-16
**Status:** Implemented — `agentmux-cloud` PR #48 (merged, deployed) — #2616
**Author:** AgentX
**Repos touched:** `agentmux-cloud` (implementation), `agentmux` (this doc)

---

## 1. Motivation

Before this, an agent that pushed a PR and wanted to know when CI finished
had to poll (`gh pr checks --watch` or repeated `pr checks`) or guess how
long to wait. The cloud relay already jekted an agent when a review landed,
when a PR merged, and when an individual check failed, but there was no
"your CI run is done" signal for the run as a whole.

This spec adds that: one jekt when a PR's CI run concludes, covering both
outcomes.

## 2. What an agent receives

One jekt per PR per completed GitHub Actions run, addressed to the agent
that opened the PR:

```
[CI] PR #2601 CI PASSED
```

or `[CI] PR #2601 CI FAILED`. Any conclusion other than success reads as
`FAILED`. The existing fast per-check failure jekt (`[FAIL] PR #N FAILED
CI`) is kept alongside it, so a failing PR can produce both.

Routing to the right agent works the same way as for review notifications:
an agent-opened PR carries its `<!-- agentmux:agent_id=... -->` tag in the
PR body.

## 3. Implementation

Nothing changes in the desktop app: the jekt arrives through the same
cloud delivery path as every other relayed jekt. The relay side (which
GitHub events it listens to, how it aggregates check runs and resolves the
target agent) is designed in the private cloud repo.
