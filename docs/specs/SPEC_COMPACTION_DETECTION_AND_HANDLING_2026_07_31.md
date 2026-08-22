# Spec: Detecting and Handling Context Compaction in Agent Panes

**Date:** 2026-07-31
**Repo:** agentmuxai/agentmux
**Trigger:** investigating AgentA's long compaction stretch (see `REPORT_AGENTA_AUTH_AUDIT_AND_COMPACTION_2026-07-31.md`) required reconstructing *when* compaction happened from git commit timestamps — AgentMux itself couldn't answer that question directly. This spec is about closing that gap structurally.

**Implementation status:** Tier 1 (live "compaction started" signal + status), Tier 2 (elapsed-time counter), and Tier 3 (predictive countdown) — plus the backend §4.1 fix they depend on — have shipped. Tier 4 (estimated progress bar) is the remaining explicit follow-up, not yet built; see §7. §4.2's original hook-mechanism sketch ("write a sentinel line to a pipe" / "curl a loopback endpoint") has been superseded by the concrete implementation described in §4.2 below — kept updated to match what actually shipped rather than left as a stale sketch.

---

## 1. Problem

Today, AgentMux does not know when a Claude Code session is compacting. It finds out **after the fact**, and only heuristically:

`frontend/app/store/agent-pane-state/reducer.ts` — on every `TokensIn` event, if the new input-token count is `< 50%` of the previous count (and the previous count was `> 10,000`), the reducer *infers* a compaction happened and emits a synthetic `context-compacted` pane event. This is consumed by `useAgentStream.ts` to push a `ContextCompactedNode` into the transcript (`DocumentRow.tsx`, a fixed 48px summary row).

This has three real gaps, all of which showed up in the AgentA incident:

1. **No "it's happening now" signal.** The heuristic only fires once the *next* turn's tokens come in. If the agent goes idle or the next turn is delayed, there is no signal at all during the gap — which is exactly the "long period with no visibility" the user asked about.
2. **It's a guess, not a fact.** A ≥50% drop from a >10k baseline is a reasonable proxy but conflates auto-compaction, manual `/compact`, and any other large context drop. It can't report *why* compaction happened, or how long it took.
3. **No timestamp is retained.** Even after detection, nothing persists *when* the boundary occurred beyond "sometime between this TokensIn event and the last one" — which is why answering "when did AgentA's compaction start" required cross-referencing external git commit times instead of just reading it off the pane.

## 2. Ground truth: Claude Code already tells us this, precisely

Confirmed directly against a real Claude Code session transcript (`~/.claude/projects/.../<session>.jsonl`, same event shape as what the CLI emits live over `stream-json`):

```json
{
  "type": "system",
  "subtype": "compact_boundary",
  "content": "Conversation compacted",
  "level": "info",
  "compactMetadata": {
    "trigger": "manual",
    "preTokens": 783887,
    "postTokens": 11775,
    "cumulativeDroppedTokens": 772112,
    "durationMs": 231606,
    "preCompactDiscoveredTools": [ ... ],
    "preservedSegment": { "headUuid": "...", "anchorUuid": "...", "tailUuid": "..." }
  },
  "timestamp": "2026-07-21T17:55:35.500Z"
}
```

This single event answers every question the heuristic can't:
- **`trigger`**: `"auto"` (context filled up) vs `"manual"` (`/compact` was typed) — distinguishable, not guessed.
- **`preTokens` / `postTokens` / `cumulativeDroppedTokens`**: exact counts, not an inferred ratio.
- **`durationMs`**: how long compaction itself took (231.6s in this example) — this is the actual answer to "when did it start": `start = timestamp − durationMs`.
- **`timestamp`**: when compaction *completed* (the event is emitted after the summarization finishes).

Separately, Claude Code exposes a **`PreCompact` hook** — configured the same way AgentMux already configures `PreToolUse` (see §4) — that fires synchronously *before* compaction begins. This is the only way to get a true "compaction is starting right now" signal in real time, as opposed to reconstructing the start time after `compact_boundary` arrives. ([Claude Code hooks reference](https://code.claude.com/docs/en/hooks.md); community writeups confirm the same shape: [PreCompact/PostCompact guide](https://www.developersdigest.tech/guides/pre-post-compact-hook), [hooks lifecycle overview](https://claudefa.st/blog/tools/hooks/hooks-guide).)

### Corrected `PreCompact` hook contract

Verified against Claude Code's docs during implementation — the following corrects/refines what earlier research assumed:

- The `PreCompact` hook's stdin payload carries **only the common hook fields** — `session_id`, `transcript_path`, `cwd`, `permission_mode`, `hook_event_name`. There is **no `trigger` field on stdin**, unlike what a naive reading of the `PreToolUse` payload shape might suggest.
- `PreCompact` **requires** an explicit `matcher` in `settings.json` (`"manual"` or `"auto"`) — there is no confirmed wildcard-all value. AgentMux therefore registers **two separate hook entries**, one per matcher, each invoking the same binary with a **different static CLI arg** baked into the command string (`agentmux-bashwrap precompact --trigger=manual` / `--trigger=auto`) so the binary knows which trigger fired without needing it from stdin.
- An observe-only hook like this one must **exit 0 with NO stdout output at all** — not even `{}`. This differs from `PreToolUse`'s `agentmux-bashwrap hook` response, which prints `{}` as an explicit "no opinion" passthrough; for `PreCompact` specifically, printing anything is unnecessary and silence is the correct signal. A hook failure must never block or error the user's Claude session, so every failure mode (malformed stdin, missing WPS env, unreachable sidecar) degrades to the same outcome: exit 0, nothing printed.

## 3. Root cause: AgentMux already receives this and threw it away

`agentmux-srv/src/agents/translator/claude.rs` (before this PR):

```rust
match frame_type {
    "stream_event" => handle_stream_event(self, &frame, &mut out),
    "user" => handle_user_message(self, &frame, &mut out),
    "assistant" => handle_assistant_message(self, &frame, &mut out),
    "result" => handle_result(self, &frame, &mut out),
    _ => {}
}
```

Every `"system"` frame — which is exactly the type `compact_boundary` arrives as — fell through to `_ => {}` and was silently dropped. This wasn't accidental drift; there was a dedicated test confirming it was the intended behavior:

```rust
assert!(t.translate(json!({ "type": "system" })).is_empty());  // claude.rs test suite
```

So the authoritative signal was already on the wire, in the exact process AgentMux was already parsing (`agentmux-srv/src/agents/runner.rs` spawns `claude --print --output-format=stream-json` and drains it line-by-line through this same translator) — it was discarded one layer before it would ever reach the frontend's heuristic. The frontend heuristic existed *because* the backend threw the real data away, not because the real data wasn't available.

## 4. Design

### 4.1 Stop discarding `system`/`compact_boundary` frames (backend)

- `claude.rs` gained a `"system"` arm in `match frame_type` that checks `frame["subtype"] == "compact_boundary"` and extracts `compactMetadata`. Any other `system` subtype, or a `compact_boundary` frame with missing/malformed `compactMetadata` fields, still produces an empty `Vec` — malformed data degrades to "no event," never a bad one or a panic.
- New `AgentEvent` variant (`agentmux-srv/src/agents/types.rs`, alongside `AssistantText`/`ToolUse`/`Cost`/`Done`/`Error`):
  ```rust
  CompactionBoundary {
      trigger: CompactionTrigger,       // Auto | Manual
      pre_tokens: u64,
      post_tokens: u64,
      cumulative_dropped_tokens: u64,
      duration_ms: u64,
  },
  ```
- Forwarded through the existing `mpsc::UnboundedSender<AgentEvent>` channel in `drain_async_reader` (`runner.rs`) exactly like every other event — the existing wildcard arm in the match already forwards it as-is; no special accumulation needed.

### 4.2 Real-time "starting now" signal via `PreCompact` hook (backend + new bashwrap subcommand)

- `build_settings_with_hooks` (`agentmux-srv/src/backend/agent_config.rs`) auto-injects **two** `PreCompact` entries (matcher `"manual"` / `"auto"`) the same way it already auto-injects the `PreToolUse` bashwrap hook — same merge discipline (user hooks preserved/prepended, ours appended last, parse failures logged at `warn!` rather than silently dropped). Both the legacy `content_map["hooks"]` merge path and the `settings.json`-level `hooks` merge path were extended identically, since a user-supplied `PreCompact` entry would otherwise hit the generic `hooks_obj.entry(k).or_insert(v)` fallback and be silently and permanently dropped the moment `PreCompact` became an auto-injected key — the exact bug class PR #813 already fixed for `PreToolUse`.
- New `agentmux-bashwrap precompact --trigger=manual|auto` subcommand (`agentmux-bashwrap/src/precompact.rs`): reads the `PreCompact` stdin payload (best-effort — a parse failure just means an empty `session_id`, never a crash), and if `AGENTMUX_LOCAL_URL`/`AGENTMUX_AUTH_KEY` are present, publishes a `compaction_started` WPS event (`{ trigger, sessionId, startedAt }`, camelCase) scoped to `block:<AGENTMUX_BLOCKID>`. Always exits 0 with no stdout output, regardless of outcome. **Delivery is live-only (`persist: 0`), deliberately** — an earlier version of this spec/implementation used `persist: 1` (replay for late subscribers), but there is no completion tombstone for this event (`compact_boundary` arrives over the separate NDJSON stream, not WPS), so a replayed "started" ping is indistinguishable from a genuinely active one; a reconnecting pane already clears its own `compacting` flag on disconnect (§4.3/§6), so there was nothing worth replaying to begin with (Codex P1, round 2 of PR #2378's review).
- The frontend subscribes to `compaction_started` per-block (`useCompactionStream.ts`, mirroring `useToolChunkStream.ts`'s single-per-block-subscription contract) and flips the pane's live status to a distinct **"Compacting…"** state — see §4.3 and §7 Tier 1.

### 4.3 Frontend: real event is primary, heuristic is the cross-provider fallback

- `reducer.ts` handles two new commands: `CompactionStarted` (sets a `compacting` pane flag + `startedAt`, sourced from the `compaction_started` WPS event) and `CompactionBoundary` (real backend data — trigger/pre/post/duration — sourced from the `AgentEvent::CompactionBoundary` frame forwarded on the same raw stdout stream `useAgentStream.ts` already parses). `CompactionBoundary` clears the `compacting` flag, reconciles `lastContextTokens` to `postTokens`, and records `lastCompactionBoundaryAt` — a dedup guard the pre-existing `TokensIn` ≥50%-drop heuristic checks before firing its own synthetic `context-compacted` event, so a Claude session that just got a real boundary doesn't also get a duplicate heuristic-sourced one on the very next turn. The heuristic itself is NOT removed — codex/gemini/copilot have no equivalent structured signal, so it remains their only detection path; it's demoted from primary to backstop for Claude specifically.
- `ContextCompactedNode` (`types.ts`) carries `source: "real" | "heuristic"` plus optional `trigger`/`durationMs` (present only when `source === "real"`).
- A new, separate `CompactionStartedNode` type represents the in-progress announcement — deliberately not folded into `ContextCompactedNode`, which represents only the completed record. Conflating the two would make a still-running compaction look finished in the transcript.
- `DocumentRow.tsx` renders `context_compacted` with the trigger explicitly ("auto-compacted" vs "you ran /compact") and the real duration when `source === "real"`; the heuristic fallback shows neither (unknowable from an inferred drop alone). `compaction_started` renders as a distinct "Compacting conversation…" announcement.
- `AgentComposerStrip.tsx` shows a live "Compacting… Ns" readout (via the existing `useTick`/`fmtElapsed` pattern) in place of the normal turn/session stats while the `compacting` pane flag is set — a real `Date.now()`-delta stopwatch. Once the real `CompactionBoundary` event lands, the finalized transcript node's duration uses the backend's authoritative `durationMs`, not the live approximation.

### 4.4 Observability (not yet implemented)

- Log a structured line per compaction event (`identity`-prefix-style: `agent.compaction: trigger=… pre=… post=… duration_ms=…`) so compaction frequency/cost is greppable in the field — directly useful input to the existing `docs/analysis/TOKEN_TAX_ANALYSIS_2026_06_19.md` line of investigation already in this repo. Tracked as a follow-up; the data is already flowing through `AgentEvent::CompactionBoundary` so this is low-effort whenever prioritized.

## 5. What this does and doesn't fix

- **Fixes**: exact start/end time and duration of every compaction, real trigger reason, real token deltas, and — via the `PreCompact` hook — a genuine live "it's happening right now" signal instead of an after-the-fact inference. Directly answers "can we know when compaction is happening" for Claude Code: **yes, precisely**.
- **Doesn't fix**: other providers. Codex/Gemini/Copilot are not confirmed to expose an equivalent structured event or hook — this spec only closes the gap for the Claude Code provider path; the heuristic remains the only signal for the others unless/until each is individually investigated.
- **Confirmed during implementation**: `compact_boundary` is emitted on live piped `stream-json` stdout in the same shape as the on-disk session transcript — the translator changes in §4.1 are exercised by the real event shape, not just the transcript capture.
- **Open question, unaffected by this PR**: whether a `PostCompact` hook is available on the currently-pinned Claude Code CLI version. `PreCompact` + `compact_boundary` together are sufficient (start from the hook; end from the `compact_boundary` event) — `PostCompact` would only be a convenience, not a blocker, and isn't used here.

## 6. Rollout shape

Purely additive: a new `AgentEvent` variant, two new hook entries merged the same way the existing `PreToolUse` one is, a new field set on an existing frontend node type plus a new node type for the in-progress state, and a heuristic that goes from "primary" to "fallback" rather than being deleted. No existing event shapes changed in an incompatible way.

## 7. UX feature tiers

The detection plumbing in §4 is necessary but not sufficient — it's the data source, not the user-facing experience. Ranked by what's actually knowable, not just what's asked for.

### Tier 0 — already existed, underused

`AgentComposerStrip.tsx` already rendered a passive context-fill readout (`12.1k / 64k ctx`, color-banded `low`/`mid`/`high`/`critical` via `ctxBand()`) with a hover tooltip stating "Auto-compacts around N tokens" (`contextTitle()`, using the same `compactionThreshold()` from `context-window.ts` that §2 references). Real, already-shipped groundwork for the "before compaction" side of the experience — static text behind a hover, not an active countdown or a proactive warning.

### Tier 1 — "compaction is happening" message — SHIPPED

The `PreCompact` hook (§4.2) fires synchronously the moment compaction begins. On receipt: a "Compacting conversation…" node is pushed into the transcript and the pane's live-status chip shows a distinct "Compacting…" state instead of generic "Working". This is the direct fix for the AgentA incident — a user watching the pane now sees this the instant it starts, instead of a silent multi-minute gap.

### Tier 2 — count-up elapsed timer — SHIPPED

A stopwatch starts on the `PreCompact` signal and stops when the `compact_boundary` event (§2) arrives — real elapsed time, not an estimate, since both endpoints are genuine events. The live client-side reading is superseded by the backend's authoritative `durationMs` once the real event lands.

### Tier 3 — predictive count-down *before* compaction starts — SHIPPED (2026-08-22)

Frontend-only, exactly as scoped: `compactionThreshold(contextWindow) − currentTokens` (already computable every turn since Tier 0) is now surfaced as explicit countdown language, inline and hover-independent — `~4.2k to auto-compact`, rendered next to the existing `12.1k / 64k` context text in `AgentComposerStrip.tsx` once the fill level reaches the `mid` band or above (silent below that — not worth calling out yet). The hover tooltip (`contextTitle()`) also gained the same remaining-count line, so the full-precision and compact readings agree.

**Escalation at the `critical` band**, implemented as a strengthened inline treatment rather than a new standalone banner component: the countdown text gains a bold weight and a leading `⚠` glyph (`.agent-composer-strip-ctx-countdown--critical` in `_composer-strip.scss`), layered on top of the pulsing red color the `critical` band's `ctx` text already had. A genuinely separate full-width banner (matching `AgentDisconnectedBanner.tsx`'s pattern) was considered and deliberately not built — this strip's own file-header comment already documents its tiered-wrap responsive layout as fragile/hand-tuned, and a new mounted row would need its own container-query wiring to avoid fighting that; the strengthened-inline-text approach delivers the same "hard to miss once critical" outcome without that risk. Revisit if a real banner is wanted later.

**Labeled as auto-compaction-only, as required**: both the inline countdown's tooltip and the main tooltip's added line state explicitly that this predicts only the CLI's own auto-compact point — a manual `/compact` can happen at any fill level and is not predicted by this countdown.

Test coverage: `AgentComposerStrip.test.tsx` (band gating, critical escalation class, zero-clamp past the threshold, tooltip wording) — proved discriminating by temporarily stubbing `compactionCountdownText` to always return `null` and confirming 4 of 7 tests fail with the expected signature, then restoring.

### Tier 4 — percent-complete of the compaction operation itself — NOT YET IMPLEMENTED (follow-up)

**Not honestly deliverable as a real signal.** Compaction is a single opaque LLM call producing a summary; there is no intermediate progress event at the protocol level — Claude Code's own interactive TUI shows only a spinner + elapsed time during compaction, never a percentage, which is strong evidence no such signal exists to expose. The only way to approximate this is an *estimated* bar: keep a running average of observed `durationMs` from past `compact_boundary` events (per account or globally) and render `elapsed / averageDuration` as a fill fraction. This must be explicitly labeled as an estimate in the UI (e.g. "usually finishes in ~30s") since a slower-than-average run will overshoot 100%, which reads as broken if presented as a real progress bar. Recommend building this only if there's user demand after Tier 3 ships.

### Recommended build order

Tier 1 → Tier 2 → Tier 3 enhancement → Tier 4 (only if requested after the above ship). Tiers 1-2 required the backend plumbing in §4.1-4.2 and have both shipped; Tier 3's enhancement is frontend-only against data AgentMux already has; Tier 4 is the only tier that needs new historical-data tracking (a rolling average store) beyond what §4 already produces.

---

## Sources

- [Claude Code hooks reference (code.claude.com)](https://code.claude.com/docs/en/hooks.md)
- [PreCompact and PostCompact Hooks — Developers Digest](https://www.developersdigest.tech/guides/pre-post-compact-hook)
- [Claude Code Hooks: Complete Guide to All 30 Lifecycle Events](https://claudefa.st/blog/tools/hooks/hooks-guide)
- [anthropics/claude-code#14258 — PostCompact Hook Event feature request](https://github.com/anthropics/claude-code/issues/14258)
- In-repo: `agentmux-srv/src/agents/translator/claude.rs`, `agentmux-srv/src/agents/runner.rs`, `agentmux-srv/src/agents/types.rs`, `agentmux-srv/src/backend/agent_config.rs`, `agentmux-bashwrap/src/precompact.rs`, `agentmux-bashwrap/src/wps_client.rs`, `frontend/app/store/agent-pane-state/reducer.ts`, `frontend/app/store/agent-pane-state/types.ts`, `frontend/app/view/agent/useAgentStream.ts`, `frontend/app/view/agent/hooks/useCompactionStream.ts`, `frontend/app/view/agent/types.ts`, `frontend/app/view/agent/virtualization/DocumentRow.tsx`, `frontend/app/view/agent/components/AgentComposerStrip.tsx`
- Live evidence: `compact_boundary` event captured from `~/.claude/projects/C--Users-area54/67b0905c-f0f9-4ff3-8278-45ed8b40b926.jsonl` (2026-07-21 session)
