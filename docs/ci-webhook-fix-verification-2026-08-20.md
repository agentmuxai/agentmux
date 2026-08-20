# CI webhook fix verification

**Date:** 2026-08-20

This PR exists to verify that the `agentmuxai` org webhook (id 598909651) now correctly delivers `check_run`/`check_suite` events to `github-router` after adding them to its event list (they were previously missing entirely, which is why CI-failure jekt notifications never fired for any repo in this org, despite the downstream code being correct).

Expected: once CI completes on this PR, `infrastructure-github-router-function`'s logs show a received `check_run` webhook, and `muxbus-github-consumer`'s logs show it processing that event.

This PR is expected to be closed without merging once confirmed — it carries no lasting change.
