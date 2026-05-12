# Spec: Agent pane session-replay framework

**Status:** Spec (no implementation yet)
**Owner:** AgentA
**Date:** 2026-05-12
**Driving requirement:** *"Build a framework that makes it easy to simulate agent pane conversations — record one real session, reuse it as a deterministic fixture in tests, smoke runs, and bug repros. We don't want to run through a whole conversation manually."*

---

## 1. TL;DR

The agent pane has three input channels: **Claude stream-json** (subprocess stdout), **WPS broker events** (live `tool_chunk`, controller status, etc.), and **reducer dispatches** (initiated by the frontend itself). Today, exercising any non-trivial code path requires launching a real Claude agent end-to-end — OAuth, CLI spawn, API tokens, network — which is slow, costly, and non-deterministic.

This spec proposes:

1. A single **`.session.ndjson`** fixture format that captures all three channels with relative timestamps + stable sequence numbers.
2. A **record harness** that wraps `useAgentStream` + the WPS broker + the dispatch transport, taps each event, and emits the fixture to disk.
3. A **replay harness** that drives the SolidJS agent pane through the same three injection points in record-order, with configurable speed (1×, instant, single-step).
4. A **test surface** in three tiers — Vitest unit replay for reducer + parser, Storybook stories for visual snapshots, Playwright integration replay for full-pane DOM assertions.
5. **Redaction + normalization at record time** so fixtures are safe to commit: OAuth tokens, working-dir paths, `session_id`s, `tool_use_id`s, timestamps all scrubbed.

Closest prior art: **OpenHands** (event-sourced state + `mock_llm_server.py` + `pytest-textual-snapshot`). We adopt their dual-track strategy (mocked unit / scheduled real integration) and their tagged-NDJSON event log shape.

---

## 2. Today's pain

- **OAuth in every test run.** First Claude launch on a fresh data dir triggers the auth flow; no auth = no test. We hit this twice in the last session alone.
- **Non-deterministic CLI output.** Same prompt → different `session_id`, `tool_use_id`, timestamps. Snapshot tests break on every run.
- **End-to-end is slow.** 4 minutes from `task dev` cold start to a usable agent pane. Iteration loop on rendering bugs is brutal.
- **Live-log streaming bugs are race-sensitive.** The fix we just merged (broker replay-on-subscribe) only became reproducible after instrumented logs. Without a deterministic replay, we burned ~6 cycles narrowing the race.
- **No regression catch.** Past visual regressions (replaceChild crash from `<Show>` cascade — PR #808) shipped to portable before we noticed. Storybook + visual diffs would have caught it on the PR.

---

## 3. Goals

1. **One-line replay.** `vitest run agent-pane.replay.test.ts` loads a fixture and drives the pane through to assertion in under 2 seconds.
2. **One-line record.** `agentmux dev --record-session=path.ndjson` wraps a real live session and writes the fixture.
3. **Composable fixtures.** Three "verbs" — `agent-message`, `tool-call+result`, `tool-chunk-burst` — can be mixed by hand to build edge-case scenarios without running a real agent.
4. **Visual smoke without OAuth.** Storybook story per fixture renders the final document tree; Playwright takes a screenshot. CI runs both on every PR.
5. **Deterministic.** Same fixture + same code = same DOM. Timestamps in the rendered output are the *fixture's* relative times, not wall-clock.
6. **Honest scope coverage.** Reducer, parser, render, AND the WPS broker dispatch + subscribe path are all exercised — not just unit-level reducer tests.

## 4. Non-goals

- **Replaying the live Claude API** with real tokens (that's promptfoo's job, separate concern).
- **Recording sub-agent (`Task` tool) recursion** in v1 — flatten as opaque blocks for now; structured sub-agent replay is v2.
- **Cross-version replay** (a fixture recorded on schema v7 should not be expected to play cleanly on schema v9). Fixtures are pinned to a schema version and migration is opt-in.
- **Replaying the agent's bash side effects.** The wrapper's PTY child does not actually run during replay — we replay the *recorded* stdout, no real bash spawn.

---

## 5. Fixture format — `.session.ndjson`

One file per recorded session. Each line is a JSON object tagged by source. Lines are ordered by `seq` (insertion order); `t_ms` is the relative offset from session start (replay scheduler honors it for live-feel pacing or ignores it for instant replay).

```jsonc
// Header (line 1): metadata + schema version
{
  "kind": "header",
  "version": 1,
  "agentmux_version": "0.33.817",
  "schema_version": 8,
  "recorded_at": "2026-05-12T15:30:00Z",
  "provider": "claude",
  "block_id": "<placeholder-uuid-1>",
  "instance_name": "test-fixture",
  "redactions": ["session_id", "tool_use_id", "cwd"]
}

// Each subsequent line is one event
{ "seq": 1, "t_ms": 0,     "src": "stream-json", "line": "{\"type\":\"message_start\",...}" }
{ "seq": 2, "t_ms": 14,    "src": "stream-json", "line": "{\"type\":\"content_block_start\",...}" }
{ "seq": 3, "t_ms": 152,   "src": "wps",         "event": "tool_chunk", "scopes": ["block:<placeholder-uuid-1>"], "data": { "op": "chunk", "tool_id": "<placeholder-tool-1>", "kind": "stdout", "content": "fixture line 1", "timestamp": 152 } }
{ "seq": 4, "t_ms": 197,   "src": "wps",         "event": "tool_chunk", "scopes": ["block:<placeholder-uuid-1>"], "data": { "op": "chunk", "tool_id": "<placeholder-tool-1>", "kind": "stdout", "content": "fixture line 2", "timestamp": 197 } }
{ "seq": 5, "t_ms": 1340,  "src": "dispatch",    "blockId": "<placeholder-uuid-1>", "action": { "type": "PaneClicked", ... } }
{ "seq": 6, "t_ms": 1880,  "src": "stream-json", "line": "{\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"<placeholder-tool-1>\",...}}" }
{ "seq": 7, "t_ms": 4500,  "src": "stream-json", "line": "{\"type\":\"user\",\"message\":{\"content\":[{\"tool_use_id\":\"<placeholder-tool-1>\",\"type\":\"tool_result\",...}]}}" }
// ... etc

// Optional trailer (last line): final document state + assertion targets
{ "kind": "trailer", "final_doc_node_count": 23, "final_status": "complete", "wall_time_ms": 4870 }
```

### Why NDJSON, not JSON

- **Append-friendly recording** — record harness writes line-by-line, doesn't need to hold the session in memory.
- **Streamable replay** — replay reads lines, schedules them, no parse-the-world.
- **Diff-friendly** — git diffs and PR reviews stay readable.

### Why three tags

- `stream-json`: opaque pass-through of Claude CLI output. Goes through the real `stream-parser.ts`. Tests every text/thinking/tool_use/tool_result path.
- `wps`: events from the broker. Goes through the real `waveEventSubscribe` handler. Tests the broker bridge.
- `dispatch`: reducer commands originating in the frontend (clicks, scroll, command-K, etc.). Captured from `recordDispatch` audit (already exists — PR #764). Tests the slot store + audit ring + downstream cells.

### Redactions

Done at **record time**, not replay time. A redactor function takes each line and replaces:

| Field | Replacement |
|---|---|
| `session_id` (UUID, Claude's) | `<placeholder-session>` |
| `tool_use_id` (`toolu_*`) | `<placeholder-tool-N>` (stable across session) |
| `block_id` (UUID, AgentMux's) | `<placeholder-uuid-1>` (one per recorded pane) |
| `cwd` | `<placeholder-cwd>` |
| OAuth tokens, API keys | `<redacted>` |
| `accessToken` / `refreshToken` | `<redacted>` |
| `timestamp` (ISO 8601) | `<placeholder-time-N>` or normalized to `t_ms`-derived synthetic |
| Anthropic `id` fields | `<placeholder-id-N>` |

Replay can opt-out of de-placeholdering (default: replay reinflates with real-looking UUIDs/timestamps for the duration of the test run).

---

## 6. Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Record mode                                                 │
│                                                              │
│  Real agent → stream-parser → reducer → SolidJS render       │
│             ↘ (tap)                                          │
│  WPS broker → useAgentStream → reducer                       │
│             ↘ (tap)                                          │
│  Frontend → recordDispatch → reducer                         │
│             ↘ (tap)                                          │
│                                                              │
│  All three taps → redactor → ndjson writer → fixture.ndjson  │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  Replay mode                                                 │
│                                                              │
│  fixture.ndjson → scheduler → demux by src                   │
│                                                              │
│      stream-json lines  → fake subprocess emit              │
│      wps events         → broker.publish (or direct sub)    │
│      dispatch actions   → dispatchPane / dispatchDoc         │
│                                                              │
│  Same parser + reducer + renderer as production.             │
│  Test asserts on the rendered DOM / store state.             │
└──────────────────────────────────────────────────────────────┘
```

### Three injection points (same in record + replay)

| Layer | Record tap | Replay inject |
|---|---|---|
| **Subprocess** | wrap stdout stream | feed bytes into the real stream-parser |
| **WPS broker** | broker middleware on `publish` | call `broker.publish(event)` |
| **Reducer** | `recordDispatch` audit hook (already present) | call `dispatchPane` / `dispatchDoc` directly |

### Scheduler modes

- `instant` — fire every event in `seq` order with zero delay (default for unit tests). Sync.
- `realtime` — respect `t_ms` deltas. Async. For visual smoke + perf measurements.
- `step` — manual `.next()` calls. For debugger walkthroughs.

---

## 7. Implementation phases

| Phase | Scope | LoC est. |
|---|---|---|
| **0** | Spec + fixture-format definition + JSON schema for `.session.ndjson`. | ~50 |
| **1** | Record harness: `--record-session=path.ndjson` CLI flag on `task dev`. Taps the three channels in a running srv + frontend. Writes redacted NDJSON. | ~400 |
| **2** | Replay harness: a TS library `frontend/test/replay/agent-session.ts` exposing `playFixture(path, mode)` that demuxes by `src` and drives the pane. | ~250 |
| **3** | Reducer + parser unit tests (`*.replay.test.ts`) under Vitest. One per fixture, asserting final node count + selected cell values. | ~150 + 5 fixtures |
| **4** | Storybook stories: one per fixture, mode=`instant`, renders the final tree. | ~100 |
| **5** | Playwright visual snapshot CI step (mode=`realtime`, screenshot at end). Compare against committed PNG. | ~150 |
| **6** | Synthetic fixture authoring: a tiny TS DSL (`fixture.bash("echo hi").assistant("here it is")`) for composing edge cases without recording. | ~200 |

Phase 0 ships first as this spec.
Phase 1 + 2 + 3 land as one PR (a minimal end-to-end demo: record a real session, replay it in a Vitest test, assert document state).
Phase 4 + 5 are visual + CI infra.
Phase 6 is the productivity power-up for authoring edge cases.

---

## 8. Best-practice references (annotated)

| Source | What we borrow |
|---|---|
| **OpenHands** ([SDK paper](https://arxiv.org/html/2511.03690v1), [AGENTS.md](https://github.com/OpenHands/OpenHands-CLI/blob/main/AGENTS.md)) | Event-sourced state + tagged-NDJSON event log + dual-track (mocked unit / scheduled real-LLM integration) + `pytest-textual-snapshot` for visual. Closest peer. |
| **Temporal replay-testing** ([Bitovi write-up](https://www.bitovi.com/blog/replay-testing-to-avoid-non-determinism-in-temporal-workflows)) | Replay history against current reducer code; throw on non-determinism. Right correctness model for a reducer-backed pane. |
| **VCR.py / vcrpy** ([docs](https://vcrpy.readthedocs.io/en/latest/advanced.html), [pytest-recording](https://github.com/kiwicom/pytest-recording)) | Redact at record time via `before_record_response`; order-based matching for streaming chunks. |
| **PollyJS** ([config](https://github.com/Netflix/pollyjs/blob/master/docs/configuration.md)) | HAR-compliant JSON for browser-side. Lesson: don't reinvent the format — JSON Schema'd ours so editors lint it. |
| **MSW v2** ([SSE docs](https://mswjs.io/docs/sse/), [streaming](https://mswjs.io/docs/http/mocking-responses/streaming/)) | First-class SSE + streaming-body APIs. We'll use MSW to mock the `/agentmux/wps/publish` endpoint in browser-only tests. |
| **Playwright** ([HAR mock](https://playwright.dev/docs/mock), [HAR-with-dynamic-params](https://medium.com/@sdgroup/harmageddon-is-cancelled-how-we-taught-playwright-to-replay-har-with-dynamic-parameters-efc4cc24894e)) | `routeFromHAR()` and `routeWebSocket()` for visual smoke. HAR-normalization pattern (replace dynamic tokens *in the HAR* before replay) — we do the same in our redactor. |
| **LangChain `FakeStreamingListLLM`** ([api ref](https://python.langchain.com/api_reference/core/language_models/langchain_core.language_models.fake.FakeStreamingListLLM.html)) | Confirmed: list-replay with optional inter-chunk sleep. We generalize the same pattern to three event sources. |
| **promptfoo caching** ([Semgrep blog](https://semgrep.dev/blog/2024/does-your-llm-thing-work-how-we-use-promptfoo/)) | "Snapshot all template variables to JSON" → reproducible. Equivalent rule for us: every fixture commits with its full input set. |
| **Claude Code CLI** ([reference](https://code.claude.com/docs/en/cli-reference), [Khan/format-claude-stream](https://github.com/Khan/format-claude-stream)) | `claude -p --output-format stream-json --include-partial-messages` → NDJSON. Community pattern: `tee fixture.ndjson` to record, `cat \| replayer` to replay. We adopt this as our `src: "stream-json"` line format directly. |
| **Spatie event-sourcing** ([replaying-events](https://spatie.be/docs/laravel-event-sourcing/v7/advanced-usage/replaying-events)) | Timestamp normalization at replay time vs record time. We pick record-time (simpler; replay is deterministic). |

---

## 9. Open questions

1. **Where does the record harness live?** Probably a small wrapper around `useAgentStream` initialized when `AGENTMUX_RECORD_SESSION=path` env var is set. Frontend-side, since that's where all three channels converge. Alternative: a srv-side recorder that taps the subprocess + broker, but it'd miss `dispatch` events.

2. **Fixture identity / pinning.** Do we commit fixtures alongside the test that uses them, or in a central `frontend/test/fixtures/agent-sessions/`? Central wins for discovery + dedup; local wins for refactor-safety.

3. **Should `dispatch` events replay the action, or assert that the action *would have been* dispatched?** Replay is simpler. Assert mode catches divergence (current code dispatches X, fixture has Y) — that's Temporal's non-determinism check. Default to replay; add assert mode in v2.

4. **Visual diff tolerance.** Pixel-perfect Playwright screenshots break on font kerning. Tolerance threshold or SVG snapshot? OpenHands uses SVG (textual-snapshot) which is more forgiving. For our DOM-rendered pane, a `toMatchSnapshot()` against HTML markup (not pixels) is probably what we want.

5. **Recording while live UI is being driven.** Concurrent user clicks during recording: do we capture them as `dispatch` events (yes), or skip them (cleaner fixtures)? Capture; have a `--filter-input` flag to strip user-input dispatches on demand.

6. **Sub-agent (`Task` tool) recursion.** A spawn of a sub-agent creates a new agent pane with its own stream. v1: flatten the sub-agent's output as opaque `text_delta` lines tagged `sub_agent_id`. v2: structured replay where each sub-agent gets its own nested fixture.

7. **Wrapper publishes outside the pane.** The bash wrapper publishes from a separate process, outside frontend taps. We catch them at the broker (`src: "wps"`). But if someone runs the agent on a different machine and the broker is remote, the recorder needs to pick them up via the WebSocket subscribe channel. Out of scope for v1 (single-machine).

8. **API drift.** When `stream-parser.ts` adds support for a new Claude event type, old fixtures stay valid (opaque pass-through). When the reducer adds a new command, old fixtures need replay-time tolerance. Plan: replay tolerates unknown commands by warning; new fixtures opt in to the new behavior.

---

## 10. Cross-references

- Live-log streaming spec: `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md`
- Named-agent continuation: `docs/specs/SPEC_NAMED_AGENT_CONTINUATION_2026_05_12.md`
- Live-log retro (motivates this work): `docs/retros/2026-05-11-live-log-streaming-wrapper-failures.md`
- `useAgentStream`: `frontend/app/view/agent/useAgentStream.ts`
- Stream parser: `frontend/app/view/agent/stream-parser.ts`
- Reducer slot store + audit: `frontend/app/store/agent-document/` (PR #764)
- `recordDispatch` audit: same module
- WPS broker: `agentmux-srv/src/backend/wps.rs`
