---
type: patch
---

The Codex review gate retries GitHub's transient errors (5xx, rate limits, dropped connections), so one bad response no longer leaves a PR's required Codex review status stuck.
