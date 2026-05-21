# SPEC: Benchmark Auth-File Discovery — Dev and Portable Instances

**Date:** 2026-05-20  
**Status:** Implemented (same PR as SPEC_DEAD_TERMINAL_PANE_2026_05_20.md)  
**Affected tooling:** `tools/tests/bench-term-echo.mjs`, `tools/tests/authfile.ps1`  
**Affected host:** `agentmux-cef/src/main.rs`, `agentmux-cef/src/dev_authfile.rs`

---

## Motivation

`bench-term-echo.mjs` measures terminal echo latency end-to-end through the WS
App API. Its primary use case is long-running session diagnostics: run the
benchmark before and after a suspected regression (e.g. memory pressure
accumulating over hours) to catch latency degradation.

Currently the benchmark is **dev-only**: it discovers the running instance by
reading `authkey.dev`, which the host writes only when `RuntimeMode == Dev`.
Portable and installed builds never write this file, so the benchmark cannot
target them without manually passing `--ws-url` and `--auth-key` — which
requires knowing the ephemeral WS port the sidecar bound to.

For long-session regression testing the portable build is the correct target:
it reflects the exact shipped binary, it runs without `task dev` overhead, and
it accumulates memory state identically to a production deployment.

---

## Current Architecture

```
RuntimeMode::Dev    →  data dir: ~/.agentmux/dev/<branch>/data/
RuntimeMode::Installed/Portable  →  data dir: ~/.agentmux/versions/<version>/data/
```

`agentmux-cef/src/main.rs` (lines ~427-453):
```rust
if is_dev {
    dev_authfile::write_dev_auth_file(
        &data_dir_path, &auth_key, &ws_endpoint, ... host_pid
    ).ok();
}
```

`is_dev` is `true` only when `RuntimeMode` matches `Dev { .. }` — i.e., when the
exe is under `dist/cef-dev/`. Portable and installed builds leave `is_dev = false`
and the file is never written.

`bench-term-echo.mjs` `findAuthFile()` already searches both:
```js
const searchRoots = [
    join(home, ".agentmux", "dev"),
    join(home, ".agentmux", "versions"),
];
```

So if portables write `authkey.dev` to their data dir, the benchmark finds it
with no changes to its discovery logic.

---

## Change

### 1. Write `authkey.dev` unconditionally (`agentmux-cef/src/main.rs`)

Remove the `if is_dev {` gate. The auth file is written for **all** runtime modes:
dev, portable, and installed.

The file path remains `data_dir/authkey.dev` in all cases. For a portable running
version `v0.37.1` the file is at:

```
~/.agentmux/versions/0.37.1/data/authkey.dev
```

### 2. No changes needed to `bench-term-echo.mjs` discovery

`findAuthFile()` already searches `~/.agentmux/versions/*/data/authkey.dev`. The
benchmark gains portable support for free.

### 3. Minor benchmark UX — label instance type in output

Add `(portable)` / `(dev)` label to the "Using authkey.dev" line so the operator
knows which instance type they are targeting:

```
Using authkey.dev: ~/.agentmux/versions/0.37.1/data/authkey.dev
  (instance=v0.37.1, pid=12345, mode=portable)
```

---

## Security Considerations

`authkey.dev` already exists for dev builds and is considered acceptable:

- The WS server binds to `127.0.0.1` (loopback-only). No remote machine can use
  the auth key.
- The file is in the user's own home directory; any process running as the same
  user already has equivalent access to the WS port via raw TCP.
- The `host_pid` liveness check means stale auth files from crashed instances
  are skipped automatically.

Extending to portable builds does not change the threat surface.

---

## Long-Session Benchmark Workflow

```bash
# 1. Launch portable (extracts to any dir)
#    AgentMux writes ~/.agentmux/versions/<v>/data/authkey.dev at startup.

# 2. Let it run for a long session (hours). Open tabs, use terminals.

# 3. When latency is suspected, run the benchmark — no manual auth flags needed:
node tools/tests/bench-term-echo.mjs --busy --output-file before.json

# 4. Compare against a fresh-start baseline:
node tools/tests/bench-term-echo.mjs --busy --output-file after.json

node -e "
const b = JSON.parse(require('fs').readFileSync('before.json'));
const a = JSON.parse(require('fs').readFileSync('after.json'));
const fmt = (x) => x.toFixed(1) + ' ms';
console.log('           before  after');
console.log('quiet p95:', fmt(b.quiet.p95), '->', fmt(a.quiet.p95));
console.log('busy  p95:', fmt(b.busy.p95),  '->', fmt(a.busy.p95));
"
```

---

## Relationship to Dead-Terminal Spec

`SPEC_DEAD_TERMINAL_PANE_2026_05_20.md` documents that dead terminals appear
under memory pressure (≥93% RAM). The benchmark is the instrument for catching
**latency degradation** that precedes or accompanies that condition. Running
`bench-term-echo.mjs --busy` when the system is under moderate pressure (70–85%)
should show rising p95/p99 before terminal failures begin.

A future CI step could fail if p95 exceeds a threshold, giving early warning.

---

## Related

- `docs/specs/SPEC_TEST_API_ACCESS.md` §5–§6 — auth file format and security model
- `docs/specs/SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md` — benchmark design
- `docs/specs/SPEC_DEAD_TERMINAL_PANE_2026_05_20.md` — failure modes this workflow targets
- `agentmux-cef/src/dev_authfile.rs` — auth file writer
- `agentmux-cef/src/main.rs` — write gate (the `if is_dev {` block)
