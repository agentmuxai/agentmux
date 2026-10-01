# Report: intermittent `vitest` CI failure, "Expected an Uint8Array" from the tear-off snapshot tests

**Status:** analysis
**Date:** 2026-09-30
**Author:** agent1
**Trigger:** the repo owner, asking what's wrong with
https://github.com/agentmuxai/agentmux/actions/runs/36792706166/job/110149077648
(push to `main` @ `c91c140b4`).

## 1. Symptom

The `vitest` job fails with **every test passing**, because vitest fails a run that
had an uncaught exception:

```
Test Files  510 passed | 2 skipped (512)
     Tests  7059 passed | 3 skipped (7062)
    Errors  1 error

TypeError: Expected an Uint8Array
 ❯ assertU8        node_modules/@exodus/bytes/fallback/_utils.js:11
 ❯ toBase64        node_modules/@exodus/bytes/base64.js:40
 ❯ FileReaderImpl._setResult  node_modules/jsdom/lib/jsdom/living/file-api/FileReader-impl.js:131
 ❯ Immediate.<anonymous>      node_modules/jsdom/lib/jsdom/living/file-api/FileReader-impl.js:108
This error originated in "frontend/app/drag/tearoff-snapshot.test.ts" test file.
```

## 2. Pattern

| What | Finding |
|---|---|
| Frequency | 5 of the last 10 `CI (PR)` push runs on `main` failed; passes and failures alternate across consecutive commits (`af0583099` ✗, `6640ddd9e` ✓, …, `29dabe0bb` ✗, `c91c140b4` ✗) |
| Same cause each time | All 5 failed runs: the same `Expected an Uint8Array`, originating in `tearoff-snapshot.test.ts` |
| Onset | The previous ~20 push runs were green. Failures start right after #4091 (Korp, 2026-09-30), which added the window-tab snapshot tests |
| Not tied to a commit | Same code passes and fails on re-runs; it's a race, not a regression in any one change |
| Local reproduction | **None.** Windows, Node 24.12 (CI: Linux, Node 24): the file alone x12 with `--sequence.shuffle`, and the full frontend suite x3 with `--maxWorkers=2`; 0 hits |

## 3. What throws, and why it can

- `frontend/app/drag/tearoff-snapshot.ts` `blobToBase64()` encodes a JPEG blob with
  `FileReader.readAsDataURL`.
- jsdom 29.1.1 does that read in two `setImmediate` hops. In the second it calls
  `@exodus/bytes` 1.15.1 `toBase64(file._bytes)`.
- That function asserts `arg && arg instanceof Uint8Array`.
- jsdom's own `Blob-impl.js` documents that `_bytes` "may" be a `Uint8Array` from another
  realm. An `instanceof` check fails for a foreign-realm array, or for no array at all.
- So this code path can throw whenever the read completes in a context where those
  globals don't line up, such as after the test's environment has started tearing down.
  It throws inside jsdom's `setImmediate`, so our `onerror` never sees it, and vitest
  reports it as uncaught.

## 4. Why it outlives its test (most likely trigger)

Several tests start a capture and finish without waiting for it to complete.
`resetTearOffSnapshotForTests()` only drops the module's `held` reference; the capture
keeps running.

| Test | How the capture is left running |
|---|---|
| "ignores a picture of a different pane" | warms up `b1`, takes `b2`; the `b1` capture is never awaited |
| "drops a stale picture" | the take returns on age before awaiting |
| "gives up on a capture still in flight after the budget" | the capture never resolves (by design) |
| "drops a picture too large for the open-window request" | pushes a cap-sized blob through `FileReader`; `take` waits at most **60 ms** and expects `undefined` anyway, so a slow encode passes the test while still running |
| "keeps shrinking until the picture fits, and gives up if it never does" | the same, with three cap-sized encodes |
| "a window tab's picture is never handed to a pane with the same id…" | two warm-ups, both taken under the other key |

On a slower CI runner, a pending large `FileReader` read from one of these finishes after
its test, or its file, has torn down. That matches both the onset (#4091 added the
cap-sized window-tab encodes) and the intermittency.

**Confidence:** the throw site and the orphaned work are confirmed in code. That
the orphan is what puts `_bytes` in a foreign realm is the best-supported explanation,
not reproduced.

## 5. Fix

1. **Remove the throw site:** `blobToBase64` encodes from `await blob.arrayBuffer()`
   instead of `FileReader`. In jsdom that copy is synchronous, into the right realm, and
   never reaches `@exodus/bytes`; in Chromium it's equivalent. Whatever the realm or
   timing, this code path can no longer throw from inside jsdom.
2. **No work outlives its test:** the module tracks its in-flight base64 encodes, and the
   test file's `afterEach` awaits them (`settleTearOffSnapshotForTests()`). Only encodes
   are tracked, not whole captures: the budget test's capture never resolves by design.
3. **A regression test** stubs `FileReader` to throw, and checks that a snapshot still
   encodes.

**Verification limit:** the failure never reproduced locally, so the proof is green
`vitest` runs on `main` after the merge. Watch the next several push runs.
