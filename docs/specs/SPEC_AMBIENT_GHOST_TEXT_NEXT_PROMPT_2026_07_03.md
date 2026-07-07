# SPEC: Ghost-Text Next-Prompt Suggestion — a Second Ambient Model Call Gateway Bind Point

**Date:** 2026-07-03
**Status:** Draft — investigation complete, design proposed, not yet implemented
**Related:** `agentmux-srv/src/ambient/mod.rs`, `agentmux-srv/src/server/app_api/session.rs`,
`frontend/app/view/agent/hooks/useAgentActivitySummary.ts`,
`frontend/app/view/agent/components/AgentFooter.tsx`,
`docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`

---

## 0. TL;DR

Claude Code CLI has a real, shipping "ghost text" feature: after a turn
finishes, the interactive TUI shows a dimmed, predicted **next user prompt**
in the empty input box (e.g. "Try running the tests"), accept it with Tab,
or ignore it and type your own message. Internally it's a `prompt_suggestion`
stream-json event with schema `{ type: "prompt_suggestion", suggestion:
string, uuid: string, session_id: string }` (confirmed by inspecting the
installed CLI binary directly, §1).

**The critical finding: this native mechanism is unreachable from AgentMux.**
It is unconditionally suppressed whenever Claude Code runs non-interactively
(`-p`/`--print`) — which is how AgentMux spawns every agent CLI, with no
exception. Confirmed empirically (§2): even with the CLI's own enabling env
var set, a real `-p --output-format stream-json --verbose` invocation never
emits a `prompt_suggestion` event.

So AgentMux cannot "turn on" Claude's ghost text — it has to build its own
equivalent. That turns out to be exactly what the Ambient Model Call (AMC)
framework (`docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`) is
for: a second ambient call, `next_prompt_suggestion`, built the same shape as
the existing `activity_summary` one — triggered on `Done`, gated through the
AMC gateway for coalescing/cancellation, tagged for token accounting — whose
result is rendered as real ghost text in the composer (§4), a genuinely new
UI surface, not the existing static placeholder text (§3).

---

## 1. What Claude Code's native feature actually is (ground truth)

Investigated by inspecting the installed CLI binary (`claude` v2.1.112,
`/c/Users/asafe/.local/bin/claude`) directly for the literal strings its
compiled bundle contains — not blog posts, which turned out to be
unreliable for this (§1.3).

### 1.1 Wire schema

A Zod-style schema literal found in the bundle:

```
h.object({ type: h.literal("prompt_suggestion"), suggestion: h.string(),
           uuid: D5(), session_id: h.string() })
```

i.e. the stream-json event shape is:

```json
{ "type": "prompt_suggestion", "suggestion": "...", "uuid": "...", "session_id": "..." }
```

A nearby doc string: `"Predicted next user prompt, emitted as..."` — this
predicts the **user's** likely next message, not a preview of what Claude
would say next. "Ghost text" here means: an editable, acceptable suggestion
for what *you* might type next, shown once Claude's own turn is fully done.

### 1.2 Gating — four independent suppression checks

Found in sequence in the bundle (each an early-return that disables the
feature, source-tagged for telemetry):

| Check | Source tag | Meaning |
|---|---|---|
| `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION` env var | `"env"` | explicit env opt-out |
| A GrowthBook remote feature flag | `"growthbook"` | staged rollout — explains inconsistent reports across users/versions online |
| **Non-interactive session** (`-p`/print mode) | `"non_interactive"` | **unconditional** — see §2 |
| Running as a "swarm teammate" (Claude Code's own internal multi-agent concept — unrelated to AgentMux's "Swarm" pane; naming collision, not a technical one) | `"swarm_teammate"` | suppressed for sub-agent/teammate sessions |
| (only past all the above) `promptSuggestionEnabled !== false` | `"setting"` | final local settings.json toggle |

The GitHub issue that prompted this investigation
([anthropics/claude-code#34211](https://github.com/anthropics/claude-code/issues/34211))
reported the feature "silently disappeared" and that its old settings key
(`showSuggestedNextSteps`) is now a dead key in the compiled CLI — confirmed:
that exact string has **zero** occurrences in the current binary, while
`prompt_suggestion` / `promptSuggestionEnabled` have ~50. The feature was
renamed/reworked, not removed — but remains gated by the checks above,
independent of that renaming.

### 1.3 A note on research quality

Initial web search results included several SEO "Claude cheat codes" blog
posts confidently describing a `--prompt-suggestions` CLI flag. That flag
**does not exist** — confirmed by running the real installed CLI:

```
$ claude --prompt-suggestions -p "say hi"
error: unknown option '--prompt-suggestions'
```

The only reliable sources were (a) the GitHub issue, a first-party bug
report referencing real compiled-output symbol names, and (b) directly
grepping the installed binary. Anthropic's own docs page on interactive mode
corroborates the *existence* of post-response suggestions but not the
fabricated flag. Lesson for future ghost-text-adjacent research: verify
CLI-flag claims against `--help` / the real binary before trusting them.

---

## 2. Empirical proof it's unreachable in AgentMux's spawn mode

AgentMux always drives Claude Code via `-p --output-format stream-json
--verbose` (see `agentmux-srv/src/server/app_api/session.rs`'s
`invoke_cli_for_activity`, and the main turn pipeline in
`agents/translator/claude.rs`) — never as an interactive TTY session.

Direct test, with the CLI's own enabling env var explicitly set:

```
$ CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=1 claude -p "What is 2+2? Answer in one word." \
    --output-format stream-json --verbose
```

Event types observed in the output: `system`, `assistant`, `message`, `text`,
`rate_limit_event`, `result`. **No `prompt_suggestion` event, ever** —
confirming the `"non_interactive"` gate (§1.2) has no override; the env var
only clears the *first* of four sequential checks, and non-interactive mode
is checked independently and unconditionally.

**Conclusion: there is no flag, env var, or setting combination that makes
Claude Code emit `prompt_suggestion` in `-p` mode.** AgentMux must generate
this signal itself if it wants the UX.

---

## 3. What AgentMux has today (and why it isn't this feature)

`frontend/app/view/agent/components/AgentFooter.tsx`'s composer already has
something informally called "ghost text" in its own comments (line ~316),
but it's the textarea's static HTML `placeholder` — a constant string
("Send message to Claude...", or "Speak to Claude..." while voice input is
listening), not a dynamic, per-turn, model-generated suggestion. It's a
different, much simpler mechanism than what this spec proposes, and the two
are compatible: the new suggestion can simply *become* the placeholder text
when present (§4.2), no conflict.

---

## 4. Proposed design: `next_prompt_suggestion`, a second AMC purpose

### 4.1 Backend — mirrors `activity_summary` exactly

New RPC, `session:next_prompt_suggestion`, structurally identical to
`session:activity_summary` (`session.rs`):

- Same trigger: `TurnPhase.kind === "Done"`.
- Same gateway usage: `ambient::gateway().admit(AmbientCallKey::new(block_id,
  "next_prompt_suggestion"), generation)` — coalesced, cancelled-on-supersede,
  rejected-stale-on-arrival, exactly like activity_summary's admission.
- Same CLI-invocation shape: spawn Claude with `-p --output-format
  stream-json --model claude-haiku-4-5-20251001`, prompt built from the
  recent conversation tail, asking it to predict a plausible, short next
  user message (e.g. "Summarize the last N turns; suggest one short, natural
  next thing the user might ask, as a single sentence with no quotes").
- Same token capture: parse the `result` event's `usage` via the existing
  `agents::translator::claude::parse_usage`, tag as
  `"ambient:next_prompt_suggestion"` in `token-usage.ts` (a new, distinct
  bucket — same pattern as `"ambient:activity_summary"`).
- New meta key: `term:next_prompt_suggestion`, generation-stamped the same
  way `term:ambient_summary` is (§4.3 covers why this matters more here, not
  less).

This is a clean second consumer of the AMC gateway with zero changes needed
to `agentmux-srv/src/ambient/mod.rs` itself — the gateway was built
key-and-purpose-generic from the start (`docs/specs/
SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md` §3.2), so this is the
validation that it actually generalizes past its first (only) caller.

### 4.2 Frontend — real ghost text in the composer

Unlike the activity summary (read-only pane-header/Swarm text), this
suggestion is **interactive**:

- Rendered as dimmed text in the composer textarea, shown only when the
  textarea is empty (matching Claude's own UX — a suggestion, not an
  overlay on text the user is actively typing) and the current turn is
  `Done`.
- **Tab accepts** the suggestion into the real input (matching Claude Code's
  own terminal UX exactly, so users familiar with the CLI get a consistent
  mental model). **Right Arrow also accepts it** — verified 2026-07-07 by
  inspecting the Claude Code CLI binary directly (`strings` + grep for every
  `name==="rightArrow"`/`name==="right"` key check): the real CLI has no
  right-arrow path anywhere near the prompt-suggestion logic, only `tab`. So
  this is a deliberate AgentMux-only addition layered on top of the CLI-parity
  baseline — the fish-shell/zsh-autosuggestions/Copilot convention of
  accepting an inline suggestion with the "keep going right" key, so the user
  can Right-Arrow-then-Enter without reaching for Tab. Safe to add: with the
  textarea empty (the only state the suggestion renders in) there's no cursor
  to move, so Right Arrow is otherwise a no-op there. Any other keystroke
  dismisses it — normal typing takes over immediately, no different from
  typing over a placeholder.
- Precedence over the existing static placeholder (§3): show
  `term:next_prompt_suggestion` if present and current-generation; else the
  existing static placeholder text. Same "prefer the richer ambient signal,
  fall back to the free one" shape as `readActivitySummary()`'s precedence
  for `term:ambient_summary` / `term:osc_title` — worth implementing as a
  sibling helper (e.g. `readNextPromptSuggestion()`) in
  `frontend/app/store/activitySummary.ts` or a co-located new file, for the
  same reason: one place owning precedence so it can't drift.

### 4.3 Binding-lifecycle discipline — the sharpest edge of this feature

The user's original framing ("the framework's responsibility is keeping
track of these bindings so they don't get lost") matters *more* here than
for the read-only activity summary, because a wrong ghost-text binding isn't
just a stale label — it can put words in the user's mouth:

- **Must clear the instant a new turn starts** (`Submitting`), not persist
  like `term:ambient_summary` does. A suggestion from turn N is meaningless
  (and actively misleading) once turn N+1 has begun — showing "you might
  want to ask about tests" while the agent is mid-way through a *different*,
  already-in-progress turn is a real correctness bug, not a cosmetic one.
- **Must clear the instant the user starts typing their own message** —
  independent of turn phase. If the suggestion RPC (1-3s round trip) resolves
  *after* the user has already started typing, the write must be dropped
  entirely (check "is the composer still empty" at write time, not just "is
  this still the current generation").
- **Must never partially apply** — if accepted via Tab, the full suggestion
  text goes in atomically; there's no "accept word by word" mode to design
  for (unlike Copilot-style completions), which keeps the write-and-accept
  path simple.
- Reuses the AMC gateway's existing generation-cancellation for the
  *backend* half of this (killing a stale in-flight subprocess) — the
  *frontend* half (checking "is the composer still empty") is new and
  specific to this feature; the gateway's generation guard alone is not
  sufficient here the way it is for the read-only activity summary.

---

## 5. Open questions

1. ~~Predict the next user prompt vs. predict the next agent action~~ —
   **decided: next user prompt**, matching Claude Code's own semantic
   (§1.1). Agent-action prediction (a Swarm-oriented, AgentMux-specific
   idea) is out of scope for this spec — a distinct future idea, not a
   variant of this one.
2. **Should this be opt-in?** Unlike activity_summary (which only writes a
   header label), this is a second Haiku call *and* a UI feature that puts
   suggested text in front of every user after every turn. Worth a
   settings toggle (mirroring Claude Code's own `promptSuggestionEnabled`)
   rather than always-on, given the added per-turn cost.
3. **Prompt design** — how many turns of context, what word/length target,
   whether to bias toward actionable next steps (Claude's own docs mention
   things like "run the tests") vs. open-ended continuations. Needs
   iteration once a first version is running, not fully specifiable up
   front.
