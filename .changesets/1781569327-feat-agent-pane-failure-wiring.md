---
type: patch
---

feat(agent-pane): surface the classified failure cause when an agent exits non-zero (SPEC_AGENT_FAILURE_DIAGNOSTICS Phase 2 — pane path). The `SubprocessController` now captures a stderr tail, runs it through `failure::classify`, and emits an `agentfailure` event; the pane shows the real reason (auth, rate-limit, OOM, context, …) + stderr tail instead of a bare "exited with code N".
