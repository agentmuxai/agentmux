# Tool-call display: two implementations, one good, and the swarm can only reach the bad one

**Status:** active
**Author:** Posa
**Date:** 2026-09-06
**Investigated at:** `6601c7b22` (v0.55.36)
**Trigger:** the Swarm pane's expanded Agent-tool row shows raw JSON. That symptom is a
one-line fix; the reason it exists is architectural, which is what this report is about.
**Related:** `SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md` (the renderer registry the
swarm path bypasses), `REPORT_SWARM_PANE_VS_ACTUAL_AGENT_CALLS_2026_09_05.md` (where the
symptom surfaced), `REPORT_SUBAGENT_COMPLETION_NEVER_DETECTED_2026_09_05.md` (§5 below
depends on its finding)

---

## 1. The symptom

Expanding an Agent-tool row in Swarm shows this, verbatim:

```
{"command":"ls /c/Users/area54/ 2>/dev/null | head -50; echo \"---\"; ls /c/User…","description":"List home directory contents"}...
```

Escaped quotes, raw braces, hard-cut mid-token. What it should show is what the agent pane
already shows for the identical call: `ls /c/Users/area54/ 2>/dev/null | head -50`.

## 2. Two implementations of the same idea

**A — `frontend/app/view/agent/stream-parser.ts:117`, `extractToolDetail`.** Typed,
per-tool, covers 10 kinds, and picks the field a human actually wants:

```ts
case "Bash":  return params.command || "";
case "Read": case "Edit": case "Write":  return params.file_path || "";
case "Grep": case "Glob":  return params.pattern || "";
case "Agent":  return params.description || params.prompt || "";
case "WebFetch": …  return u.host + (u.pathname === "/" ? "" : u.pathname);
```

Three consumers: `ToolBlock.tsx:316`, `activity/tool-adapter.ts:202`, and `stream-parser`
itself at `:734`. This is the codebase's real answer to "summarise a tool call."

**B — `agentmux-srv/src/backend/subagent_watcher/parse.rs:261`.** `serde_json::Value::to_string()`
— a compact JSON dump — truncated at a bare `200`:

```rust
let input_summary = value.get("input").map(|v| {
    let s = v.to_string();
    if s.len() > 200 { … format!("{}...", &s[..end]) } else { s }
})
```

Same operation, opposite quality, opposite side of the process boundary. `tool_result`'s
`preview` (`:283`) is the same shape at `500`.

## 3. The actual architectural fault

Not "B is sloppy" — **B formats too early, at the wrong layer, and destroys the input.**

Summarisation happens at *parse* time on the backend. The structured `input` object is
reduced to one display string and thrown away; `SubagentEvent` carries only
`input_summary: String`. So the frontend **cannot** apply `extractToolDetail` to a subagent
tool call even though the function is sitting right there — the data it needs never crosses
the boundary.

That is why the swarm feed also bypasses the whole renderer registry
(`components/tool-renderers/registry.ts`, `SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md`)
— a priority-ordered, shape-matching system with rich per-tool UIs (`RecordTable`,
`SearchResults`, `WebFetchResult`) built precisely so "the open-ended tool universe … can be
routed by its real name or result shape." The swarm feed can't route anything: it receives a
pre-flattened string and renders it through `AnsiText` as monospace lines.

**The layering rule this breaks:** the backend's job is to observe and transport; deciding
what a human should read is a view concern. Every consumer that comes along later inherits
B's choices permanently, because the alternative was discarded upstream.

## 4. Secondary findings in the same area

**4.1 The magic numbers mix units.** `200`/`500` are inline and unnamed, and each pairs a
**byte** length test with a **char** cut:

```rust
if s.len() > 200 {                                   // bytes
    let end = s.char_indices().nth(200).map_or(s.len(), |(i, _)| i);   // chars
```

For ASCII these agree. For multi-byte content they don't: a string over 200 bytes but under
200 chars takes the truncation branch, finds no 200th char, and falls back to `s.len()` — so
it silently isn't truncated at all. Harmless today, but it is two different notions of
"length" in one expression, and the repo already has `bounded_instructions_preview`
(`bundle_import.rs:129`) as a named, tested precedent for exactly this.

**4.2 The truncation cuts mid-token and appends `...` to broken JSON.** With B's output being
JSON, a 200-byte cut routinely lands inside a string literal or before a closing brace, so the
result is neither valid JSON nor readable prose. Fixing §3 dissolves this: truncating a plain
command string is safe in a way truncating serialised JSON isn't.

**4.3 Types are hand-mirrored across the boundary, twice, with no check.**
`frontend/types/gotypes.d.ts` opens with *"Hand-maintained type bindings. Keep in sync with
agentmux-srv … The original Go generator was removed with the Go backend."* But
`SubagentEventType` isn't in that file at all — it's mirrored a second way, inline in
`swarm-model.ts:140-146`, against `types.rs:105-115`. Two mirroring conventions for one
boundary, neither enforced by anything.

**4.4 A now-dead variant and a dead detector, with a comment that argues for keeping them.**
`SubagentEventType::Result` exists so that "completion detection can key off the discriminant
directly" (`types.rs:110-114`), and `jsonl.rs:243` still tests for it. Per
`REPORT_SUBAGENT_COMPLETION_NEVER_DETECTED_2026_09_05.md`, **no AgentMux transcript contains a
`"type":"result"` line** — 0 of 11 on this machine, across two CLI versions — so that branch
is unreachable, and #3007 replaced its job with parent-`tool_result` correlation. Two
completion mechanisms now exist, one provably dead, and its doc comment still reads as
live justification. Whoever touches this next will believe it works.

## 5. Proposed cleanup

Ordered so each phase stands alone and none blocks the next.

### Phase 1 — stop dumping JSON (small, self-contained, fixes the reported symptom)

Replace `Value::to_string()` in `parse.rs` with a Rust port of `extractToolDetail`'s field
selection, plus a named constant to replace `200`/`500` with a single unit (chars). Two tools
cover most real traffic: `Bash → command`, `Agent → description`.

Buys the visible fix immediately, and is revertible on its own. It does **not** fix §3 — it
duplicates the mapping in a second language, which is a deliberate down-payment, not the
destination. Phase 2 removes the duplicate.

### Phase 2 — move formatting to the edge (the actual fix)

Carry the structured input across the boundary — `input: serde_json::Value` (bounded) instead
of, or alongside, the flattened string — and let the frontend call the `extractToolDetail`
it already owns. The swarm feed then becomes eligible for the renderer registry for free,
which is the real prize: `RecordTable`/`SearchResults`/`WebFetchResult` start working in
Swarm with no new per-tool code.

**The honest tension:** B's truncation is presumably there to bound WS payload size, and
raw inputs are unbounded (an `Agent` prompt can be tens of KB). So this needs a size budget
at the transport layer — bound the *payload*, not the *meaning* — rather than removing
limits. Worth measuring real input sizes before committing to a number; I have not.

### Phase 3 — one binding surface

Fold `SubagentEventType` into `gotypes.d.ts` so there's one mirroring convention rather than
two, and add a check that fails when the Rust and TS shapes diverge. A generator is the
better answer but a far larger change; a diff-check is cheap and closes the silent-drift hole.

### Phase 4 — delete the dead completion path

Remove `SubagentEventType::Result`'s detector at `jsonl.rs:243` and either drop the variant or
re-document it as parse-only. Keeping a second, unreachable completion mechanism next to the
working one is how the original bug survived a refactor: #2283 replaced a heuristic that
"almost never fired" with one that fires never, and the two look identical from outside.

## 6. Recommendation

**Phase 1 now, Phase 4 alongside it** — both are small, and Phase 4 removes a live trap for
the next reader. Phase 2 is the one worth doing properly and shouldn't be rushed into the same
change; it needs the payload-budget question answered with measurements. Phase 3 is
independent housekeeping that can wait for someone touching those types anyway.

## 7. What I did not verify

- Real-world size distribution of tool `input` payloads. Phase 2's budget depends on it and I
  am not guessing at a number.
- Whether any consumer outside Swarm reads `input_summary`. I found none in
  `frontend/` (only `swarm-model.ts`/`swarm-view.tsx`), but I did not audit MCP or external
  RPC callers, which could make its shape a compatibility surface.
- The blank-lines half of the original report. That is a rendering question in `AnsiText`, not
  a formatting-layer one, and is deliberately out of scope here.

---

## 8. Shipped (2026-09-06)

### Phase 1 — done as proposed

`parse.rs` gains `tool_input_detail` (a Rust mirror of `extractToolDetail`),
`result_content_text`, and `truncate_chars`, replacing both `Value::to_string()` dumps.
`MAX_INPUT_SUMMARY_CHARS`/`MAX_RESULT_PREVIEW_CHARS` replace the bare `200`/`500` and are
CHARS throughout, closing §4.1's byte-test/char-cut mismatch.

The Bash case from §1 now renders as `ls /c/Users/area54/ 2>/dev/null | head -50`.

One deliberate divergence from the frontend original: an **unknown tool falls back to the raw
JSON object** rather than `""`. `extractToolDetail` can return empty because its callers still
hold the full node; here there is nothing else to show, and `mcp__*`/provider-specific names
are open-ended by design. A known tool whose expected field is missing or empty takes the same
fallback.

11 tests, mutation-checked — forcing the JSON-dump fallback fails 5 of them, including the
flagship Bash case.

### Phase 4 — NOT done as proposed; re-documented instead of deleted

The proposal above said to delete the dead detector. **I didn't, and the reasoning is worth
recording rather than quietly narrowing the scope.**

Deleting `jsonl.rs`'s `Result`-discriminant check would strand `PendingCompletion`, the
`pending_activity` coalescing buffer's `completed` vector, and two broadcast shapes — all of
it *correct* code. Its only fault is being unreachable in a transcript format we don't
control and which could change back; if `"type":"result"` lines ever appear it would resume
being the **faster** completion signal (immediate, versus waiting for the parent turn to end).
Deleting correct handling for an external format we don't own trades a real capability for
tidiness.

What actually caused harm was the doc comment asserting it works. Both sites now say
plainly that it never fires, cite the evidence, point at `completion.rs` as the real path,
and record the #2283 history — a placeholder match that "almost never fired" replaced by a
discriminant check that fires *never*, indistinguishable from outside, which is precisely how
the original bug survived a refactor.

If a future reader disagrees and wants the deletion, §4.4 and this section are the argument
either way; the trap (believing it works) is gone regardless.

**Phases 2 and 3 remain open**, unchanged.

**Verified:** full `agentmux-srv` suite 3030 passed / 0 failed. Not verified in a running app
— the string this produces is unit-tested, but I have not watched a Swarm row render it.
