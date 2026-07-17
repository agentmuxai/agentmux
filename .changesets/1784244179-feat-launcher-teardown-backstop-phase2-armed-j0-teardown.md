---
type: patch
---

feat(launcher): teardown backstop Phase 2 — armed J0 teardown for a wedged host

Completes SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 (#2092), the last undelivered item from Discussion #1680's §9 scorecard. Phase 1's observe-only UI-thread liveness probe now feeds an armed state machine: arm on PoolDrained / OrphanInstance drift (last user window closed, host still alive), disarm on any WindowOpened or host exit, and — once armed past a 30s grace with ≥2 consecutive delivered-but-unanswered UI-thread probes — TerminateJobObject(J0) with a distinct launcher exit code (86). A host whose UI thread wedges after the last window closes no longer lingers as an orphaned process tree. Includes the spec's debug:hang_ui verification hook (double-gated behind AGENTMUX_DEBUG_HANG=1), a consecutive-miss counter in ui_liveness (any pump since a probe's send disqualifies it as wedge evidence), and reducer-style unit tests for the state machine. Docs riding along: 2026-07-16 lifecycle program status snapshot, SPEC_BRIDGE_INIT_RECOVERY correction (its reload-preserves-creds premise was proven false by #2181), and tracking-doc updates.
