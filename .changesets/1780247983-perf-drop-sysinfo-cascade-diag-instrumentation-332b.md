---
type: patch
---

perf: drop sysinfo cascade-diagnostic instrumentation

Commit ac70ff59 (2026-05-28) added temporary diagnostic instrumentation to
the sysinfo broadcast hot path — two `tracing::warn!` calls in
`Broker::publish` (one per missing-client branch, one per zero-routes
sysinfo branch) and two per-tick `console.log("[fe] sysinfo:* handler …")`
emits in the status-bar widgets (`SystemStats`, `BackendStatus`). All four
were tagged "Remove once the cascade root cause lands."

The cascade root cause has since landed. The diag is no longer needed and
is shipping in production builds, where:

- The two FE `console.log` calls fire every sysinfo tick (~1 Hz × 2
  widgets) → routed through CEF's console→host bridge → synchronous
  `tracing::info!` → disk. Locally measured at ~70k `[fe] sysinfo:*` lines
  per day in the host log on Linux. Each emit also rides the main thread's
  IPC bridge, adding noise to the input-first hot path.
- The two backend `tracing::warn!` calls fire from inside the broker
  mutex, with `event.scopes` formatted as `?` debug on every sysinfo
  publish whose route lookup is empty.

This commit removes all four diagnostics. The widget bodies revert to the
shape they had pre-ac70ff59 (no diagnostic logging around the reactive
setStats / setUptimeSecs calls). The broker's `client is None` arm
becomes a plain early return; the zero-routes branch is deleted entirely.

Net: 1 insertion / 29 deletions across 3 files.
