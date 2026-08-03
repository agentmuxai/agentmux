# REPORT — Extending `muxspect` to diagnose pre-spawn refusals

**Date:** 2026-08-03
**Type:** Extension to `muxspect` (PR #2380, merged 2026-08-02) — implemented,
this report is both the incident writeup and the design doc for the fix.
**Status:** Implemented. `last_error` is now a field on both
`/api/v1/muxspect/list` and `/api/v1/muxspect/describe`.

---

## 1. The incident that motivated this

An agent's pane ("Agent1") sat showing "Agent encountered error" for over two
hours. Closing/reopening the pane and resending messages did nothing.
Root-causing it required three separate, manual investigations: grepping two
days of srv logs for the block id, then a direct read-only query against the
live `store.db`. `muxspect` — which already existed at the time — was checked
*last*, and even then came back diagnostically empty:

```
$ node ~/.agentmux/shell/muxspect.mjs describe e913dc16-c0a6-4470-9acf-692dfc720f90
controller_type:     (no controller)
liveness_confidence: none
processes (0):
  (none tracked)
```

This is **correct** — there genuinely was no controller and no process for
that block — but it's indistinguishable from a perfectly healthy idle block.
The actual cause, reconstructed by hand from srv logs and `db_agent_identity_links`,
was one line, entirely knowable at the time:

```
identity.spawn.blocked: no credentials for provider claude (definition
938e343c-..., identity bundle-agenty-default) — no account bound for the
agent's provider; spawn refused (single-point enforcement — use_ambient_login=true, ignored)
```

The agent definition had a `github` identity link but no `claude` link — so
the v0.54.8 identity migration correctly classified it as fail-by-default
(not ambient), and every spawn attempt was refused before a controller or
process ever existed. Closing/reopening the pane could never have fixed this:
there was nothing for the client to retry against.

## 2. The signal already existed — nothing read it

This is the load-bearing finding: **the fix here isn't new instrumentation,
it's reading something that was already being written.**

`agentmux-srv/src/identity/resolver/inject.rs`'s spawn gate
(`gate_oauth_failure`) returns a `SpawnGateError` whose `Display` impl
(`errors.rs`) is already a good, actionable, user-facing message:

> "no credentials for claude: the bound account was deleted or is
> unresolvable. Bind an account for this provider in the Armory."

Per `agent_handlers/input.rs`'s own existing comment, that message is
**deliberately persisted into the block's own `output` file** as an
`error_during_execution` frame — the same frame shape the agent pane's
frontend renders as its error bubble — specifically so *"the frame must be
PERSISTED to the block file... otherwise the error vanishes on pane
reload/reconnect"* (reagent P1, PR #2164 round 2). The same frame shape is
also written by two other pre-spawn failure paths — `container_spawn.rs`'s
`exec`/`ensure_running` failures and `host_spawn.rs`'s queued-message-drain
failure — so this isn't identity-specific, it's the general "spawn never
happened, here's why" signal for the whole app.

The data `muxspect` needed already lived in exactly one place, was already
durable, and was already the *same* source the frontend itself reads.
Extending `muxspect` to read it is a direct application of its own design
constraint (`SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` §5.1/§3 pt.
8 — "never a second, independent state-tracker"): a second reader over an
existing durable store, not a new one.

## 3. What was implemented

### 3.1 `last_error_frame()` — `agentmux-srv/src/server/muxspect_handlers.rs`

A bounded reverse-tail read (last 8KB, `LAST_ERROR_TAIL_BYTES`) of a block's
`output` file via the existing `FileStore` (`stat` + `read_at` — the same
store `blockfile.rs`'s `read_range`/`line_count` handlers already read).
Deliberately only inspects the **last non-blank line** — a block that
errored once and then kept producing normal output afterward must NOT be
flagged; only "the last thing that happened to this block was an unrecovered
error" is surfaced. If that line parses as
`{"type":"result","is_error":true,"subtype":"error_during_execution",...}`,
its `error.message` is extracted and classified (`classify_last_error_source`)
into a best-effort `source` tag (`identity` / `container_spawn` / `host_spawn`
/ `unknown`) by matching the message's own stable prefix — each of the four
construction sites in the codebase writes a distinct, unambiguous phrasing.

**Age, resolved:** the frame itself carries no wall-clock timestamp at
write time. Rather than touching four call sites across two subsystems
(identity, container/host spawn) to add one, this reads the `output` file's
own `modts` (already tracked by `FileStore` on every write) — correct for
exactly the case this feature exists for: nothing appends to a block's
output after an unrecovered pre-spawn failure, so the file's last-modified
time *is* the frame's timestamp. Returned as a raw epoch-ms `written_ms`
(matching the existing `last_computed_ms` convention), not a precomputed
duration — the CLI computes age with the same `ageString()` helper it
already uses for every other timestamp in the response.

### 3.2 Wired into both routes, unconditionally

Both `handle_muxspect_list` and `handle_muxspect_describe` now call
`last_error_frame()` for every block/`block_id` they already handle and add
a `last_error` field (`null` when absent) — additive only, no change to the
existing response shape. Computed unconditionally rather than gated on "no
live controller": it's cheap (one bounded tail read), and gating it would
mean re-deriving the same liveness logic `process_status` already computes
just to decide whether to bother. In practice it's non-null only for the
case it exists for.

**A scope note on `list`:** `handle_muxspect_list` iterates
`ProcessBroker::list()`, which is scoped to `get_all_controllers()` — blocks
with a currently-registered controller. This PR decorates the rows `list()`
already returns; it does not change what block_ids `list()` enumerates. A
block whose spawn is refused *before* any controller is ever registered may
still not appear as a `list()` row at all (independent of this change,
matching the route's own existing "every controller-backed block" scope —
see its doc comment). **`describe` has no such limitation** — it already
accepts and correctly reports on any block_id string, controller-backed or
not (see the existing `muxspect_describe_composes_status_for_an_unknown_block`
test), so it's the reliable, primary fix for the diagnostic gap; `list`'s
new column is a real improvement for whatever it already shows, not a
guarantee that every wedged block will surface there. If a future incident
shows blocks going wedged with no controller record at all, widening
`list()`'s source of block_ids (e.g. to every known agent pane, not just
currently-registered controllers) is a reasonable, separate follow-up — not
attempted here.

### 3.3 CLI rendering — `muxspect.mjs`

- `muxspect list`: new `last_error` column, `yes (<age>)` or `-`.
- `muxspect describe`: new `last_error:` section (message / source / age),
  printed last since it's usually the answer someone's there for precisely
  when everything above it is empty.
- `--json` output carries the raw field for either command, unchanged
  shape otherwise.

### 3.4 Docs

`docs/MUXSPECT.md`'s "what it can and can't see" section now documents
`last_error` alongside the existing lifecycle/process-tree/staleness bullets.

## 4. What this would have caught

Re-running the actual incident with this change live:

```
$ muxspect describe e913dc16-...
last_error:
  message: [AgentMux] no credentials for claude: the bound account was
           deleted or is unresolvable. Bind an account for this provider
           in the Armory.
  source:  identity
  age:     2h14m
```

That's the entire investigation in one command, with the actual fix printed
verbatim — the same conclusion that took a two-file srv-log grep plus a live
`store.db` read to reconstruct by hand.

## 5. Explicitly out of scope

- **No mutation.** `muxspect` stays read-only — no auto-rebind, no
  auto-retry. Same invariant as Phase 1.
- **No stable pre-spawn failure taxonomy.** `source` is a best-effort tag
  inferred from message text, not a tested enum like
  `agents::failure::FailureClass` (which only classifies **post-spawn** exit
  failures — `classify()` never runs for a spawn refused before a process
  existed). Building that taxonomy properly is real, separate work belonging
  to whoever owns `SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md`.
- **Not fixing the frontend error bubble.** Whether the pane's own "Agent
  encountered error" banner should show this message text is a product/UI
  question, not addressed here.
- **Not a general blockfile tail viewer.** Scoped to exactly one frame
  shape, not "show me the last N lines of any block" — that's `muxlog`'s
  job.
- **Not widening `list()`'s block enumeration** — see §3.2's scope note.

## 6. Testing

- `agentmux-srv/src/server/muxspect_handlers.rs` — 6 new unit tests for
  `last_error_frame`/`classify_last_error_source` directly (missing block,
  empty output, frame present, frame present-then-recovered, non-error
  result frame, every known message-prefix classification).
- `agentmux-srv/src/server/tests.rs` — 2 new HTTP-level tests: `describe`
  surfaces `last_error` for a block with a persisted identity-gate refusal
  as its last output line; `describe` returns `last_error: null` for a
  block with no output at all.
- `cargo test -p agentmux-srv` — 1959 passed, 0 failed (full suite, not
  just the new tests).
- `npx vitest run muxspect.test.mjs` — 8/8 passing, unchanged (no
  `parseArgs` changes in this PR).
- Not run this pass: a live rebuild + manual `muxspect describe` against a
  real wedged block in a running instance (same scope decision PR #2380 and
  #2373 each made in their own test plans) — the unit + integration
  coverage above exercises the exact code path directly.
