# Structured review rollout verification — agentmuxai/agentmux

**Date:** 2026-08-19

This PR exists to verify that `reagent-structured-review-enabled` (widened to include `agentmuxai/agentmux` in `services/infra`) actually routes this repo's reviews through the new schema-constrained `--json-schema` path (`a5af/reagent` PRs #205/#206) in production. This repo is a separate verification from the earlier `agentmux-landing` check — higher traffic, and the one `agentmux#2664` is waiting on.

Expected: a clean review comes back `APPROVED` via `review_verdict.compute_review_event([])`, confirmed via `[REVIEW] Structured findings` in CloudWatch logs, not the old `determine_review_event()` text-parsing path.

This PR is expected to be closed without merging once the review lands and is confirmed — it carries no lasting change.
