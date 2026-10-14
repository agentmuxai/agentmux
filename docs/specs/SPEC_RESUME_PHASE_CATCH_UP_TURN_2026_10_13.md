# SPEC: Resume phases: the "Given to the agent" card in its place, then the agent catches up and tells the user where it is

**Status:** active. Phase 1 (#4888: the catch-up turn for a fresh session and a manual compaction, and retiring the side-model note from #4707) is merged. Phase 2 (a catch-up when the user resumes an agent, the compose gate, the hidden-reinjection window ended by a message, the title kept across an account switch) is in the PR that edits this line; the ghost-text fix and phase 3 are not started. Verified by reading `main` @ `c9bcafb47`, 2026-10-13; not reproduced in a running app.
**Date:** 2026-10-13
**Related:**
- `SPEC_CONTEXT_DELIVERY_2026_09_30.md`: the card (CD1, CD2a/b shipped; CD4 "continuation packet" card never built).
- `SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md`: the hidden-turn fallback and its deferral (§3.3a).
- `SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md`: the SessionStart hook delivery (D10: `resume` gets nothing).
- `SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md`: when a resume is refused and a fresh session carries the record.
- `SPEC_AMBIENT_NARRATION_INLINE_AGENT_VOICE_2026_09_24.md`: ambient narration, settled as live-only (§ on persistence).
- `docs/investigations/RCA_FRESH_SESSION_RESUMED_BEFORE_FIRST_TURN_2026_10_11.md`: §4 leaves repeated "session continued" cards open.
- #4707 (`be4906ecb`): the pick-up note. #4708 (`3f7945a1c`): `supersededCard`.

Paths: `R/` = `crates/srv/src/`, `F/` = `frontend/app/view/agent/`.

## 0. The ask

From the operator, 2026-10-13, about an agent pane after resuming a conversation from My agents, switching the agent's account, or compacting:

> "We have a usual 'agent given such and such files…' now sometimes it comes through correct. Sometimes the conversation starts, we are going back and forth, and then spontaneously I see the 'Given to agent…' — that is the first bug. The 2nd bug, and its related, is that anytime an agent goes through a resume phase (like after compact or account switch or resume), after it finishes its catchup process, it should tell the user with something, like a short summary of where they are picking up; it shouldn't just end on a 'given to agent' section without any other explanation of state. Check the ambient. Finally, do a DRY check: these paths are very similar … we essentially have shared paths that end in similar parts."

And the expected sequence, from the operator the same day:

> "The 'Given to agent' card would appear just after the resume event, and the agent would be in progress chewing on it; when done, it prompts the user with a friendly summary and resume suggestions."

Three goals:
- **G1:** the card appears once, directly after the resume event it belongs to.
- **G2:** the agent then visibly works through what it was given (a **catch-up turn**), and ends by telling the user, in its own words, where it's picking up, with suggested next steps. The reply is an ordinary agent message, so it survives a reload.
- **G3:** one pipeline for all resume phases, so G1 and G2 hold on every path by construction rather than path by path.

## 1. The resume paths today

| Path | Trigger | What the agent gets | Turn after? | What the user sees at the end |
|---|---|---|---|---|
| **A. Native resume** (My agents reattach, reconnect, app restart; resume accepted) | `AgentPicker.tsx` `handleReattach` → controller `start()` → eager `--resume` with no message (`R/backend/blockcontroller/persistent/eager_resume.rs`) | Nothing: hook source `resume` gets no delivery (`R/backend/memory_delivery.rs` `Reason::from_hook_source`, D10) | No | Nothing. No card, and the "resumed" outcome row is hidden (`F/useAgentStream.ts` ~652). |
| **B. Fresh session onto history, hook delivered** (account switch → `ControllerResync forcerestart`; identity gate refused; `--resume` rejected) | `R/…/persistent/spawn.rs` `fresh_onto_history` → `carry_continuation()` | Hook `startup` memory (no turn); the `<agentmux-continuation>` packet rides on the **next** user message (`spawn.rs` ~706-717) | No | "Session continued" row, then "Given to the agent · new session". Then **maybe** the pick-up note (§2.2). |
| **C. Fresh session, frontend fallback** (eager resume rejected with nothing queued, or the hook didn't deliver) | `fresh` outcome → `memoryReinjectionController.trigger(…, "fresh_session")` (`F/useAgentStream.ts` ~672-677) | Hidden `AgentInput` carrying memory (+ packet); the reply is swallowed by the hiding queue | Yes, hidden | "Given to the agent · session continued" (`resume_fresh`, written in `R/server/agent_handlers/input.rs` ~1480-1494). Possibly the note. |
| **D. Manual `/compact`** | Sent to the CLI; `compact_boundary` | Hook `compact` memory + running summary | No | "Agent given a summary of the conversation (manual compact)", "Memory re-delivered after compaction". Nothing else. |
| **E. Auto-compact mid-turn** | Same frame, mid-turn | Same | The interrupted turn continues | The agent keeps working; never says where it is. |
| **F. Compaction fallback** (hook didn't deliver) | Controller, deferred to turn end (`F/memory-reinjection-controller.ts`) | Hidden turn: `compose_fallback` + running summary | Yes, hidden | The card only. |
| **G. `/clear`** | AgentMux's own command, frontend only (`F/../../app/commands/global/clear.ts`) | Nothing reaches the CLI | No | An empty document. Out of scope here. |

Only Claude runs as a persistent controller with the SessionStart hook; every other provider gets none of this (no packet, no hook, no compaction re-delivery). Out of scope, noted in §7.

## 2. Findings

### 2.1 Bug 1: why the card appears mid-conversation (verified in code unless marked)

The renderer doesn't misplace it. Both live and replay draw the card exactly where srv appended its `system/agentmux_memory_injected` frame to the transcript (`F/parseHistoryLines.ts`, `reducer.ts` `StreamFlush` appends at the end). Nothing anchors a card to the event it describes. srv appends it **late** in these cases, most likely first:

1. **The `fresh_session` fallback defers past the first exchange (path C).** `trigger()` defers while the pane is busy (`F/memory-reinjection-controller.ts` ~354-357), and re-defers after compose if a turn has started. It fires at a later `session_end`, and re-defers again if a queued message or a jekt keeps the pane busy. §3.3a of the reinjection spec expects this ("the pane is almost always busy at that exact instant"). When it finally fires, srv's `claim_event` stands down only if a **completed** hook delivery for `(block, Startup)` was composed after `event_at − 10 s` (`R/server/memory_delivery_handlers.rs` ~247-254). Otherwise it delivers, and "Given to the agent · session continued" plus its hidden turn land after one or more real turns. When the `fresh` outcome arrives while the pane is idle (an eager spawn after an account switch), it fires at once and the card is at the top: **the same mechanism explains "sometimes correct"**.
   - Ways the hook delivery fails to count (inferred): a part misses its 3 s `/part` or 1.5 s `/ack` timeout (`bashwrap/src/sessionstart.rs`), so `notice_sent` stays false and the delivery never enters `completed`; 8 parts compose in parallel with no in-flight lock, which adds load; the 60 s `CLAIM_WINDOW_MS` passes during a long first turn; the hook isn't installed (the 10-11 RCA §4 saw exactly this on a dev build); the `fresh` trigger has no provider gate.
2. **A later respawn without `--resume` (path B mid-session).** A lazy spawn triggered by a user message fires SessionStart `startup`, and the hook appends "new session" after that message. For generations after the first, `fresh_start_needs_disclosure` emits no session divider (`R/…/persistent/mod.rs` ~191-193), so nothing explains the card. `supersededCard` doesn't fold it, because the agent worked in between.
3. **Compaction (paths D–F).** `trigger()` claims as soon as the boundary arrives. If that claim beats the compact hook, the hook's parts come back empty, and the hidden turn then waits for the compacting turn to end: the card lands after that turn's answer, not under its "Context compacted" row.
4. **Another writer's lines (rarer).** The transcript cursor feeds lines another block or instance appended to the shared zone through the live sinks. Another block's card is appended at the current end, and another block's `fresh` outcome triggers **this** pane's fallback (`claim_event` has no origin check, unlike `claim_boundary`'s `boundary_origins`). `claim_boundary` never consults `completed`, so a boundary seen late can deliver twice.
5. **Duplicates on replay.** A fallback card whose hidden-message line is on an older history page renders twice (`memory-reinjected-replay-<ts>` plus the card, `F/parseHistoryLines.ts` ~285-296). `supersededCard` folds only cards already loaded.

To tell which one an operator hit: a mid-conversation frame with `"fallback":true,"reason":"resume_fresh"` right after a hidden "Your memory was reinjected…" line is (1); one with `session_id`, `parts` and `reason:"startup"` is (2); in the srv log, "memory delivery: complete, notice sent" is missing for that spawn.

### 2.2 Bug 2: nothing tells the user where the agent is picking up

- **One path has a line, and it's fragile.** #4707's pick-up note (`R/backend/continued_note.rs`, prompt in `R/ambient/prompt.rs` ~167-179: "open by saying you have picked up where you left off … End with exactly: How can I help?") is scheduled only when a fresh spawn carries the packet (path B/C). It needs: generated within or after a 20 s wait for the startup card, no SKIP from the model, the user hasn't typed, no note in the last 2 minutes.
- **It is live-only.** It's published as an `AmbientNarration` event with persist 0 (`continued_note.rs` ~155-156) and added to the live document only. Snapshots don't carry nodes and replay has no `ambient_narration` case. **After a reload, reopen or restart, or when no pane was subscribed when it was published, the pane ends on the card.** This is the most likely reason the operator never sees it.
- **No line at all** on A (native resume), D/E/F (compaction), a fresh session where `carry_continuation()` returns `None`, a second switch within 2 minutes, or a model SKIP/timeout/failure.
- **The agent itself is never asked.** The packet says "Continue as if uninterrupted: don't re-introduce yourself" (`R/…/continuity.rs` ~322-328), and it only arrives with the user's next message anyway. The hook and fallback texts give no reply instruction, and fallback replies are hidden. Adding "say where you're picking up" to the hook or packet text would **not** fix this: no turn follows those deliveries.
- **The fallback path always waits the full 20 s.** `startup_card_written_since` (`memory_delivery_handlers.rs` ~316) only sees hook `Startup` completions, never fallback cards.

### 2.3 The ambient layer around resume

Verified, from `R/ambient/` and `frontend/app/store/ambient-*.ts`:
- Ambient doesn't misread the card or the packet: the digest skips `system` frames, pane-side activity drops `context_delivery` and `ambient_narration` nodes, and hidden turns are skipped by the title hook, the running summary and the digest.
- **The running summary already holds what the note needs.** `continuity_state` (every 3 human turns or 10 min after a successful turn) writes sections including "Last user request" with a status, "In flight when last seen" and "Waiting on the user". It's fed to the model in the packet and the compaction re-delivery but never shown to the user.
- Bugs found along the way:
  - **The hidden-reinjection gate sticks.** `HIDDEN_REINJECTION_BLOCKS` (`R/…/digest.rs` ~31-53) is set by a hidden `AgentInput` and cleared only by a later non-hidden `AgentInput` (`input.rs` ~1438). Jekt and reactive deliveries go through `run_agent_turn` and never clear it. After any hidden re-delivery, every digest-based ambient call (title recovery, ghost text) returns `None` until a person types in the pane, so a jekt-driven agent can stay untitled indefinitely.
  - **Compaction wipes the ghost text.** The hidden re-delivery's `Submitting` clears the suggestion, and its end requests none (`useNextPromptSuggestion.ts` ~207-214).
  - **An account switch resets the goal title.** Any controller stop publishes `done`, and `useBlockActivity.ts` clears the title and turn count and moves the old title to `term:restored_summary`, though the conversation continues.

### 2.4 DRY check

The paths converge in a few places: `spawn_process` is the one place the packet is attached; `push_sections`, `global_entries` and `personal_entries` are shared; `claim_fallback` arbitrates hook vs fallback; one card component renders everything. Everywhere else they fork with near-copies:

1. **Message composition twice, on two sides.** srv `compose_items_with` (hook) and `compose_fallback` (fallback) have different headers; TS keeps `composeReinjectionMessage` (`F/memory-reinjection.ts` ~103) as a fallback for an older srv, byte-pinned by tests on both sides.
2. **The same clause and signature literals in 4–6 places:** the compaction clause (`memory_delivery.rs` ~66, ~263; `continuity_state.rs` ~240; `memory-reinjection.ts` ~83), the fresh-session clause (`memory_delivery.rs` ~265; `memory-reinjection.ts` ~85), `REINJECTION_SIGNATURE` (`memory_delivery.rs` ~239; `memory-reinjection.ts` ~142; a hand copy in `digest.rs` ~266).
3. **Two running-summary formats**: `## Running summary of the whole conversation …` in the packet (`continuity.rs` ~300-311) and `# Running summary of this conversation …` on compaction (`continuity_state.rs` ~276-284), with copy-pasted timestamp formatting.
4. **Two card frame builders**: `notice_frame` (~913-926) and `fallback_frame` (~592-613) in `memory_delivery_handlers.rs`, and the "Running summary" size entry twice (~567-582, ~871-886).
5. **"Append a frame to the transcript" pasted five times**: `memory_delivery_handlers.rs` ~394-402 and ~423-431, `input.rs` ~1482-1493, `resume_retry.rs` ~19-27 and ~60-68.
6. **Four reason vocabularies with mappings between them**: Rust `Reason {Startup, Clear, Compact}`, `FallbackReason {Compaction, FreshSession}`, `reason_for_fallback` (maps `fresh_session` → `Startup`), wire strings `startup | clear | compact | resume_fresh`, TS `REASON` and `ReinjectionReason`.
7. **"Is this our hidden turn?" decided four ways**: `digest.rs` ~266, `continuity.rs` ~44, `continued_note.rs` ~99, TS `isMemoryReinjectionMessage`.

Resulting inconsistencies the user can see:
- The same event gets two labels: a fresh session continued from the record reads "new session" when the hook delivered and "session continued" when the fallback did, under an outcome row that says "Session continued".
- The note fires on one path; the card fires on all but native resume; hidden turns run on fallback paths only.
- The outcome row says the session "was given AgentMux's record" before that's true (the record rides on the next message, `DocumentRow.tsx` ~667-668), and the packet itself has no card (CD4 types exist in `types.ts` and `ContextDeliveryCard.tsx`; nothing produces one).

## 3. Design

One idea carries all three goals: **a resume phase is an event with an identity, and everything it produces (card, note, claim) is keyed to that event and placed after it.**

### 3.1 The resume event (G3)

srv owns one module, `R/backend/resume_pipeline.rs`, and one type:

```rust
pub struct ResumeEvent {
    pub block_id: String,
    pub id: String,          // stable: <block>-<session_id>-<kind>[-<boundary uuid>]
    pub kind: ResumeKind,    // Resumed | Continued { record: bool } | Compacted { manual: bool } | Cleared
    pub session_id: String,
    pub anchor: String,      // the transcript node it belongs after: outcome row or compaction boundary
    pub at_ms: u64,
}
```

Every path in §1 creates exactly one `ResumeEvent` where it is detected today (eager-resume success arm, `fresh_onto_history`, the `fresh` outcome, the compaction boundary) and hands it to the pipeline, which runs the same four steps:

1. **Compose**: one composer for memory, summary and packet. Clause strings, the signature and the summary heading are defined once in Rust; the TS fallback composer is deleted once srv's `memorydelivery:compose` is the only path (CD2b already ships it), and the remaining TS literals that detect hidden turns are generated, not hand-copied.
2. **Card**: one `write_context_card(event, items)` that replaces the five pasted appends and the two frame builders. The frame carries `event_id` and `anchor`, and is written **when the event happens**, before the catch-up turn starts, so it's placed right after the event (§3.2).
3. **Deliver**: hook or catch-up message (§3.3). The hook mechanics don't change; the hidden fallback turn and the packet-on-next-message both become the catch-up turn.
4. **Catch-up turn**: `catch_up::start(event)`, §3.3: a turn whose input is hidden and whose reply is shown.

One reason vocabulary on the wire and in TS: `resumed | continued | compacted | cleared` (with `manual`/`auto` and `record` as fields, not separate reasons). `FallbackReason`, `reason_for_fallback`, `resume_fresh` and the TS `REASON` table go away; titles map from the one vocabulary (§3.2).

### 3.2 The card in its place, once (G1)

**Revised during implementation (2026-10-14).** The first draft anchored every card to the node of the event it describes and re-placed it in the document reducer. That is unnecessary once nothing is delivered late: a card is placed by where its frame sits in the transcript, live and on replay, so it lands in the right place if srv writes it when the event happens. The work is therefore to stop late delivery, not to re-order after it.

- **No delivery after the conversation has moved on.** A fresh session discovered while a turn is in flight (the user's message spawned it) keeps today's memory-only fallback, claimed against the hook's delivery. A fresh session discovered on a quiet pane, which is the account-switch case, no longer waits for anything: the catch-up turn (§3.3) is sent at once, so its card is written at once.
- **Claims by event.** Ask srv whether the event is this block's: a compaction this block's CLI did not write (another block of the agent reading the shared transcript zone) answers `foreign`, and nothing is delivered or caught up. A fresh-session outcome that is not recent (a re-read, a line another writer appended) is ignored, and one outcome seen twice starts one turn.
- **Make the hook reliable** (Phase 2): compose once per event with an in-flight lock so 8 parallel parts share one composition, and count a part that was fetched but not acked as delivered for claim purposes.
- **Titles from the one vocabulary** (Phase 3): "session continued" for every fresh session onto history, hook or fallback; "new session" only for a genuinely new conversation.
- **The packet's card (CD4)** (Phase 3): the record rides on the catch-up message, so it belongs to the same card.

### 3.3 The catch-up turn (G2)

The sequence the operator expects, for every resume phase:

1. The resume event's row ("Resumed", "Session continued", "Context compacted").
2. The "Given to the agent" card, directly under it (§3.2).
3. The agent visibly working: a **catch-up turn**, shown as the pane's normal busy state, labelled "Catching up…". Its input is hidden (the card stands for it); its tool calls and reply are shown like any turn.
4. The agent's reply: a short, friendly message saying where it's picking up, with suggested next steps. Then it stops and waits for the user.

**Mechanics (as built).**
- The frontend starts the turn, from the controller that already owns the hidden-turn flow (`memory-reinjection-controller.ts`, `catchUp`). The sources are the session outcome frame (`fresh`) and the compaction boundary (`manual`); it fires only on a quiet pane, because a user already talking to the agent has nothing to be caught up on. A manual compaction is found mid-way through its own turn, so it waits for that turn to end (`deferIfBusy`) and claims the memory event at the boundary, not at the end.
- srv composes the message (`memorydelivery:compose` with `catch_up`): the memory re-delivery if the claim says deliver (else the hook delivered it), then the catch-up block, in one `<system-reminder>`. The running summary of a compaction is inserted before the catch-up block. With no memory to deliver the block stands alone and there is no card.
- The input goes as a hidden `AgentInput`, so the pane shows no row for it; the stream parser knows it by `AgentMux catch-up:` and returns no node for it, live and on replay, but unlike a reinjection it does not hide what follows. The agent's reply is an ordinary agent message, so it survives a reload with no special persistence. The card, when there is one, is written by srv when it accepts the message, as for any fallback.
- The hidden-reinjection gate for ambient reads is not set for a catch-up, so titles and ghost text keep working while the reply streams.
- One catch-up per event. A message or jekt that arrives during it queues behind it as normal; Esc interrupts it like any turn.
- **If the resume was caused by a user message** (a lazy spawn that couldn't resume), there is no separate turn: the pane is busy, so the controller keeps today's memory-only fallback, and the record rides on the user's own message.

**What the turn carries, per kind.**

| `ResumeKind` | Input |
|---|---|
| `Continued { record: true }` (account switch, refused or rejected resume) | Whatever memory the hook didn't deliver, the `<agentmux-continuation>` record (no longer waiting for the next user message), the instruction |
| `Continued { record: false }` | Memory not already delivered, the instruction |
| `Resumed` (My agents, reconnect, restart) | The instruction only: the session's own context is intact and the startup file carries the memory (D10 stands) |
| `Compacted { manual: true }` | The compaction re-delivery if the hook didn't deliver it, the instruction |
| `Compacted { manual: false }` mid-turn | **No separate turn by default**: the interrupted turn is already the agent chewing, and its own reply ends it. Only if that turn ends with no assistant text does a catch-up turn follow (owner decision 3) |
| `Cleared` | None: a cleared conversation has nothing to pick up |

**The instruction** (one constant in `resume_pipeline.rs`; the packet's current "Continue as if uninterrupted: don't re-introduce yourself" is replaced by it):

> You are picking up this conversation after {an account switch | a resume | a compaction}. Read what you were just given. Before replying you may re-check real state read-only (git status, open PRs, files) to confirm what is still true; don't change anything yet. Then reply to the user in a few lines: where you're picking up (the last thing they asked and whether it was done), anything in flight or waiting on them, and two or three suggested next steps they can choose from. Then stop and wait for them.

**Suggestions.** The steps are part of the reply, as a short numbered list. The turn ends visibly, so the existing end-of-turn ghost-text suggestion (`next_prompt_suggestion`) runs as after any turn and pre-fills the composer with the likeliest one. Clickable suggestion chips are a possible follow-up, not part of this spec.

**What it replaces.**
- The deferred hidden fallback turn for `fresh_session` on a quiet pane, whose reply was swallowed, and a manual compaction's hidden turn.
- The ambient pick-up note from #4707 (`continued_note.rs`, purpose `session_continued_note`): the agent says it in its own voice now. Removed in Phase 1, with its 20 s wait and 2-minute de-dup.

**Cost.** One model turn per resume event. For `Continued` and `Compacted` the memory and record are sent today anyway, just in a hidden turn or on the next message. An unattended agent (driven by jekts) also takes the turn; its reply stays in the transcript for whoever looks next.

### 3.4 Fixes that ride along

- Clear `HIDDEN_REINJECTION_BLOCKS` when a hidden or catch-up turn ends (not on the next typed message), so jekt-driven agents keep titles and ghost text. The ambient digest skips the catch-up input but reads its reply.
- On an account switch that continues the conversation (`Continued { record: true }`), keep the goal title and turn count instead of resetting them.
- Compaction no longer leaves the ghost text cleared: the catch-up turn ends visibly and requests a new one.

## 4. Phases

| Phase | Scope | Fixes |
|---|---|---|
| **P1** (this PR) | The catch-up turn for a fresh session on a quiet pane and for a manual compaction; `foreign` claim answers; the catch-up message and its recognisers (srv and the stream parser); retire the note from #4707 | Bug 2 for the account-switch and manual-compact cases; bug 1's quiet-pane path |
| **P2** (built) | `Resumed`: a catch-up when the user resumes a conversation. The launch (`launchAgentDefinition`, which My agents and the launch modal's Continue both use) leaves a request on the pane's block meta (`agent:catchup`, `catch-up-request.ts`) and the pane's stream hook reads and clears it once running, so the picker needs no handle on the pane's controller; a fork is a new branch and asks for none. The compose gate: the 8 parts of a session start no longer each read and compose the memory, the first composes and the rest wait for it. The hidden-reinjection window for ambient reads ends on a jekt, cron or broadcast delivery, as it does on a person typing, so an agent only ever driven by messages gets its title and ghost text back. A fresh session that continues the conversation puts back the title the controller's stop moved to `term:restored_summary`. Not built: ghost text after a hidden re-delivery (an auto-compaction's memory-only fallback still clears it) | Bug 2 for resume; the ambient gaps |
| **P3** | `resume_pipeline.rs`: one composer, one card writer, one reason vocabulary, generated TS literals; delete the TS composer; the packet's card (§3.1, §2.4) | The DRY findings |

Each phase ships alone.

## 5. Tests

- Reducer: a card frame with an `anchor` that arrives after N later nodes is placed right after its anchor; replay of the same transcript gives the same order; an anchor on an unloaded page re-places on load.
- Hook and fallback for one event produce one card (same id); a `fresh` outcome from another block's line doesn't deliver in this block.
- For each `ResumeKind`: exactly one card, then exactly one catch-up turn (none for auto-compact with assistant text, none for `Cleared`); the input isn't shown and the reply is, live and after a reload.
- A user message sent during the catch-up turn queues and runs after it; a lazy spawn puts the catch-up content on the user's message and starts no second turn.
- Two account switches in a row give two cards and two catch-up replies.
- `HIDDEN_REINJECTION_BLOCKS` is clear after a catch-up turn followed by a jekt turn; the goal title survives an account switch.
- Snapshot the wire vocabulary and the instruction text so TS and Rust can't drift.

## 6. Owner decisions

Settled by the operator, 2026-10-13/14: the agent itself writes the pick-up message, in a visible catch-up turn after the card, ending with suggested next steps (§0), and "use your recommendations" for the rest:
1. **Read-only checks during catch-up:** allowed. The instruction lets the agent confirm real state (git status, open PRs, files) before replying, and forbids changing anything.
2. **Native resume (`Resumed`):** a catch-up turn. Scoped to a resume the user starts from My agents (Phase 2), not to the eager resumes AgentMux does by itself on restart or reconnect, which would otherwise start a turn in every pane at once.
3. **Auto-compact mid-turn:** no separate catch-up; the interrupted turn's own reply ends it. Only a manual compaction gets a catch-up turn.

## 7. Out of scope

- Non-Claude providers (no hook, no packet, no compaction re-delivery). Worth its own spec once this pipeline exists; the `ResumeEvent` shape is provider-neutral on purpose.
- `/clear`'s srv path, which is unreachable from the composer today.
- Clickable suggestion chips.
