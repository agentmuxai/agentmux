# SPEC: `x-agentmux-srv-version` response header

**Date:** 2026-08-22
**Status:** Implemented
**Author:** Korp
**Repos touched:** `agentmux` (`agentmux-srv/src/server/mod.rs`,
`agentmux-srv/src/backend/shellintegration/muxspect.mjs`)
**Related:** Ext 5 of `docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`

## 1. Problem

Live, during the same debugging session that produced the report:
`muxspect conversations` returned a bare `404 Not Found` against the
actual running instance, because the running `agentmux-srv` binary
predated the route (the feature had just merged to `main` but this
instance hadn't been rebuilt yet). Nothing in the response said so — the
404 looked identical to "route genuinely doesn't exist" and "this build is
just old," and diagnosing which one required manually cross-referencing
the source tree against the running binary's version by hand.

## 2. Change

Every HTTP response from `agentmux-srv` now carries an
`x-agentmux-srv-version` header stamping the instance's own
`CARGO_PKG_VERSION`. Implemented as a small `axum::middleware::from_fn`
layer in `build_router` (`agentmux-srv/src/server/mod.rs`), applied to the
whole router — health/webhook routes included — so it's universal rather
than requiring every individual handler to set it. The version string is
captured by value from `AppState.version` before the layer closure, so no
`AppState: Clone` bound is needed.

`muxspect.mjs`'s `apiGet`/`apiPost` read the header on every call and:
- Print it to **stderr** (`[srv v0.55.19]`) — deliberately not stdout, so
  it never pollutes piped/parsed `--json` output.
- On a **404 specifically**, fold a hint into the failure message: "this
  instance is running srv vX.Y.Z — a 404 here may mean it predates this
  command; check you're not talking to a stale build." 404 is singled out
  because a version mismatch is the most plausible explanation for that
  specific status, versus e.g. a 400 (bad request) or 500 (real server
  error) where the version is much less likely to be the story.

## 3. Non-goals

- Does not add version-skew *enforcement* (refusing to talk to a
  mismatched instance) — purely diagnostic, matching every other
  `muxspect` command's read-only posture.
- Does not attempt to compare the caller's own build version against the
  server's — the CLI core (`muxspect.mjs`) doesn't carry its own version
  identity separately from whatever srv deployed it next to.
- Does not backfill this into already-running instances — like every
  other Rust change in this report, it only takes effect once an instance
  is rebuilt and relaunched on the new binary.

## 4. Testing

- `logSrvVersion()` (the header-read/log function) is pure enough to unit
  test directly: 3 cases (header present → returns + logs; header absent
  → returns falsy + logs nothing) using a fake `Response`-shaped object,
  with `console.error` mocked via `vi.spyOn` so test output stays clean.
- Not verified against a live rebuilt instance for the same reason as Ext 4
  (`SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md` §3) — would need a
  full srv rebuild + relaunch. Verified via `cargo check` and the CLI unit
  tests instead.
