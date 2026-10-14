# SPEC: Context deliveries — what the agent was given, one item at a time

**Status:** active — CD1 (#4037), CD2a (#4041) and CD2b (#4130) implemented; startup-file items implemented under the launch-context spec (#4129, #4131); CD3 proposed (awaits the owner's decision on storing bodies); CD4 moved to the resume-phase spec (§6); CD5 narrowed to the narration tag. B1 (#4901), B2 (#4909), C1 (#4915) and C3 fixed; checked live on 2026-10-14 (§5.2), which found C1–C3 (§9).
**Date:** 2026-09-30. **Revised:** 2026-10-14 (§0.1, §1.3, the §2 note, §3–§9; live results §5.2).
**Verified against:** `agentmux` `main` @ `5c0d603ec` (2026-10-14), by reading code,
and #4888 at `40623c1ba` for what it changes (§1.3, §3.5, §9). §2 is the
2026-09-30 baseline: its line numbers refer to the tree before the October
history rewrite. Elsewhere code is cited by file and symbol. Paths are relative
to the repo root; `frontend/…/agent/` means [`frontend/app/view/agent/`](../../frontend/app/view/agent).
**Related:**
- `SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md` (#4888): **owns resume
  phases**: where a card lands, the catch-up turn after it, one reason
  vocabulary, and the continuation-packet card (formerly CD4).
- `SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md`:
  the `startup_file` items on a new session's card, and why the hook leaves
  Global Memory out when a startup file carries it (§3.7).
- `docs/reports/REPORT_FRESH_SESSION_MEMORY_REINJECTION_DUPLICATE_2026_10_05.md`:
  a fresh session's memory delivered twice, fixed by dated claims (#4364).
- `SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md`: the delivery paths this
  spec changes the display of. D6 there ("the user sees a label only") is
  revisited in §3.4.
- `SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md`: the
  hidden-turn primitive and the current notice.
- `SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`: what "ambient"
  means (§1).
- `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`: the
  `context_compacted` row.

## 0. The ask

From the owner, 2026-09-30:

> "After auto-compact or resume, we want to see an item of each memory that
> is being injected (currently we just get a summary like 3 injections) —
> we want to know more about each one, file etc. Second, when manually
> clicking compact, we see the raw input to the agent (a full summary
> getting injected as if it were a user message) … the user sees something
> like 'Agent injected with summary: key points:…' and something very
> short. Also, is ambient the correct name? … We may need a new term since
> ambient may be used to mean two different things."

### 0.1 Where this stands (2026-10-14)

| Ask | State |
|---|---|
| See each memory item, not "3 injections" | **Built.** One row per item on every hook and fallback delivery (CD2a, CD2b), plus each startup file the CLI read on a new session (launch-context LC2, LC3). Rows carry no text yet (CD3). |
| The compaction summary isn't a user message | **Built.** A card with an excerpt, pinnable to read the whole summary (CD1). |
| Is "ambient" the right word? | **Decided** (§1) and in the public glossary. Only the narration tag still shows "ambient" (CD5). |
| *Later (2026-10-13):* the card right after a resume, account switch or compaction, then the agent saying where it is picking up | **Owned by** the resume-phase spec (#4888), which also found that cards can land late (§9). |

Checked live on 2026-10-14 (§5.2): the cards work, and the run found three
more places where a delivery still reads as something else (§9, C1–C3).

## 1. Terms

### 1.1 The decision

- **Ambient** means one thing: **a model call AgentMux makes on its own**,
  not driven by the user. Titles, activity summaries, ghost text,
  narration and the running summary are all ambient calls. This is the
  existing meaning (`crates/srv/src/ambient/mod.rs`, the AMC
  framework). "Ambient" says **how** something was produced, never where
  it goes.
- **Context delivery** (new) means **content the agent is given without
  the user typing it**. Memory, the compaction summary and the
  continuation packet are all context deliveries. A context delivery is
  made of **items**. The pane shows one **delivery card** per delivery,
  with one row per item, and each row expands to the full text. "Context
  delivery" says **where** something goes: into the agent's context.

The two can overlap. The running summary is an ambient call whose output
becomes an item in a context delivery (`backend/continuity_state.rs`,
`running_summary_section`).

### 1.2 Why a new term

"Ambient" already has three unrelated meanings in code:

| Meaning | Where |
|---|---|
| model calls AgentMux makes itself | `ambient/`, `term:ambient_summary`, the `__ambient__` token bucket |
| the host's own CLI login and home dir | `use_ambient_login` (189 occurrences), `AmbientHomeDirNotAllowed` (`identity/resolver/errors.rs`) |
| a sound | "looping ambient tone" ([`frontend/app/view/settings/sections/sounds-section.tsx`](../../frontend/app/view/settings/sections/sounds-section.tsx)) |

None of the mechanisms that send content to the agent is called ambient
today. Between them they use four names instead: "hidden turn",
"delivery", "continuation packet" and "jekt".

The pane already shows users the word **"ambient"** as a tag on narration,
which is text that **never** reaches the agent
(`frontend/…/agent/components/AmbientNarrationBlock.tsx`, `AMBIENT_TAG_LABEL`). Using the same
word for text that **does** reach the agent would give one visible word
opposite meanings.

**Other candidates, and why not:**
- **Taken:** "hidden" (the hidden turn, and agent visibility
  `display_hidden`), "background" (background tasks), "sidecar" (the srv
  process), "whisper" (whisper.cpp speech-to-text) and "out-of-band"
  (memory drift detection).
- **Free but rejected:**
  - "backchannel" sounds covert, which is the opposite of the point: the
    card exists so the user can see everything the agent was given.
  - "system note" undersells memory and summaries.
- **Why "context delivery":** it builds on the existing
  `memory_delivery` wording (`backend/memory_delivery.rs`), and it's
  free in code and specs.

### 1.3 Renames that follow

| Was | Now, or where it went |
|---|---|
| "🧠 Memory reinjected — …" for every reason | **Done** (CD2a). Every delivery is a `ContextDeliveryNode` with a title per reason (§3.2); "re-delivered" only after a compaction. |
| `MemoryReinjectionNode` / `"memory_reinjection"` | **Kept, not aliased.** It is still produced for transcripts from before CD2b (their hidden fallback line has no card) and by the frontend's own composer when srv is older than CD2b (`memory-reinjection.ts`). The resume-phase spec's Phase 3 deletes that composer; the replay reader stays for old transcripts. |
| reasons `startup` / `clear` / `compaction` / `resume_fresh`, and titles "new session" / "session continued" | **Moved** to the resume-phase spec's Phase 3: one vocabulary (`resumed`, `continued`, `compacted`, `cleared`) and "session continued" for every fresh session onto history, so there is one rename, not two. |
| the narration tag "ambient", shown to users (`AMBIENT_TAG_LABEL`, `AmbientNarrationBlock.tsx`) | "AgentMux". The tooltip "Generated by AgentMux, not by the model" stays. **CD5, not done.** |
| persisted frame `system/agentmux_memory_injected` | **Unchanged** on disk; hook and fallback deliveries both write it. |
| `use_ambient_login` and "ambient home dir" | Out of scope; a follow-up (§7, Q4). |

## 2. What happened before this spec (verified 2026-09-30)

> [!NOTE]
> This section is the baseline the design answers, kept as written. Most of
> it no longer describes the code: the notice, the fallback and the
> summary frame are now cards (§3). Its line numbers refer to the tree
> before the October history rewrite. Two facts changed under it: the CLI
> writes the boundary in snake_case (`compact_metadata`), which the
> parsers missed until #4303. Until 2026-10-03 the live pane never saw a
> boundary, so the fallback's compaction trigger never ran, and the summary
> card was found by its two signals alone (§3.3). Since #4303 a
> compaction's claim names its boundary's `uuid`.

### 2.1 Memory deliveries

| Event | Startup file | SessionStart hook | Frontend fallback (hidden turn) |
|---|---|---|---|
| new session | written | `startup` | — |
| `/clear` | — | `clear` | — |
| auto-compact | — | `compact` (not yet verified live for auto) | yes, deferred while busy, then claims |
| manual `/compact` | — | `compact` | yes |
| normal resume or fork | written | none (D10) | — |
| fresh session after a failed resume | written, plus continuation packet | `startup` | `fresh_session` |

Sources: `memory_delivery.rs:45-52`; `memory_delivery_handlers.rs:184-301`;
`useAgentStream.ts:530-564,621-622`; `spawn.rs:368-389,697-708`.

**The notice:**
- **Hook path:** srv builds a `system/agentmux_memory_injected` frame
  (`memory_delivery_handlers.rs:404-417`) with `entries[{label, source,
  size_bytes, tokens}]`, and appends it to the block output, so it is
  persisted. The frontend (`memory-injected.ts:36-72`) turns it into a
  `MemoryReinjectionNode` and drops `reason`, `summary_bytes` and
  `parts`.
- **Fallback path:** the frontend builds the node itself
  (`memory-reinjection.ts:311-355`), and nothing is persisted. Replay
  rebuilds it from the hidden stdin line, so names become "Entry N" and
  sizes are approximate (`memory-reinjection.ts:183-290`).
- **Rendering** (`frontend/…/agent/virtualization/DocumentRow.tsx:433-495`): one line,
  "🧠 Memory reinjected — N global, M personal (~T tok, est.)". The
  entries appear only in a hover tooltip. The row can't be expanded
  (`frontend/…/agent/virtualization/disclosure.ts:98-104`, FIXED_OPEN).
- **Data available when the notice is built:**
  - **Global:** `name`, `is_system`, `size_bytes` and `text`
    (`bundles.rs:206,221-238`). The bundle id is not carried, and
    `is_system` is folded into the label as a prefix
    (`memory_delivery_handlers.rs:347`).
  - **Personal:** the file name and text (`:361-377`); the directory is
    known but dropped.
  - **Bodies are never stored**, by design (D6).
- **Personal Memory** is included only when the hook call carries an agent
  token (`memory_delivery.rs:20-22`, `:321-323`). Whether "0 personal"
  notices come from a missing token is not verified.

### 2.2 The Compact button

- **AgentMux doesn't send a summary.** The button sends the literal user
  turn `/compact` (`frontend/…/agent/agent-view.tsx:2881`).
- **Claude Code writes its own summary** and echoes it on stdout as a
  plain-string user frame: `{"type":"user","message":{"role":"user",
  "content":"This session is being continued from a previous
  conversation…"}}`. The transcript store has five of these
  (2026-09-15 to 09-21), each right after a `compact_boundary`, and none
  carries `isCompactSummary`.
- **The pane renders it as a user message.** `frontend/…/agent/providers/claude-translator.ts:320-322`
  turns any string-content user frame into a `user_message`, which
  `DocumentRow.tsx:253-259` renders; replay does the same
  (`parseHistoryLines.ts:306`). That is the owner's "raw input as if it
  were a user message".
- **It isn't a digest leak.** The ambient digest reader only reads array
  content in user frames (`server/app_api/session.rs:1489-1493`), so it
  already skips this frame.
- **Auto-compact** produces the same frame; it's rarer to notice because
  it happens mid-work.
- The `compact_boundary` frame carries `compactMetadata.trigger`
  (`manual`/`auto`), `preTokens`, `postTokens` and `durationMs`, parsed by
  `compact-boundary.ts:52-72`. It is already rendered as a
  `context_compacted` row (`types.ts:445-458`).

### 2.3 Other injected content

- **Continuation packet** (`backend/continuity.rs`, tag
  `<agentmux-continuation>`): hidden. The pane shows a label that can't be
  expanded, "Session continued · reconstructed context"
  (`DocumentRow.tsx:630-650`).
- **Jekts** already have their own collapsed bubble (`JektBubble.tsx`), so
  they are out of scope here.
- **Dead-air re-delivery lines** (`persistent/input.rs:602-1020`) replay
  as user messages written by the system. That's a follow-up (§7).

## 3. Design

### 3.1 One node, one card

As built (`frontend/…/agent/types.ts`, `ContextDeliveryNode`, `ContextItem`):

```ts
interface ContextDeliveryNode {
  type: "context_delivery";
  id: string;               // the frame's own id; for a summary, its uuid (§3.3)
  reason: "startup" | "clear" | "compaction" | "resume_fresh";  // renamed by the resume-phase spec, Phase 3
  trigger?: "manual" | "auto";   // compaction only
  items: ContextItem[];
  timestamp: number;
  sizeBand?: "low" | "mid" | "high" | "critical";  // Personal Memory against the context window
}

interface ContextItem {
  kind: "global_memory" | "personal_memory" | "running_summary"
      | "compaction_summary" | "continuation_packet" | "startup_file";
  name: string;             // entry name, file name, or a fixed label
  tier?: "system" | "workspace";   // global memory only
  path?: string;            // personal memory and startup files: the file
  bundleId?: string;        // global memory: entry id
  delivered?: "full" | "partial" | "omitted";  // §3.4 step 3; absent = full
  sizeBytes: number;        // of what was delivered
  sourceTokens?: number;    // memory items: the whole entry, whatever was delivered
  tokens: number;           // estimated
  excerpt?: string;         // ≤160 chars, for the collapsed card (summary only)
  body?: string;            // inline only for compaction_summary (§3.3)
  // startup_file only (§3.7):
  role?: string;            // user_instructions | instructions | instructions_import | skills | mcp_servers
  owner?: "agentmux" | "user" | "external";
  count?: number;           // skills and MCP servers: how many are listed
  contains?: string[];      // AgentMux sections the file carries: global_memory, skills_index
  via?: "startup_file";     // a Global entry the agent got through a startup file (size 0)
}
```

`bodyRef`, in the first draft, is not built; CD3 decides where bodies
live. `continuation_packet` has a type and an icon, but nothing produces
it: the resume-phase spec builds that card (Phase 3).

On the wire each memory, summary and startup item is one entry of the
`agentmux_memory_injected` frame (`EntrySize` in
`crates/srv/src/server/memory_delivery_handlers.rs`), in snake case, plus
`label` and `source` for builds older than CD2a. The frontend reads it in
`memory-injected.ts` (`buildMemoryInjectedNode`).

### 3.2 How the card looks

As built (`components/ContextDeliveryCard.tsx`; titles in
`context-delivery.ts`, `contextDeliveryTitle`):

- **Header:** an icon, the title, then the total as "~N tok (est.)".
  Titles:
  - "Given to the agent · new session · 4 items"
  - "Given to the agent · after /clear · 2 items"
  - "Memory re-delivered after compaction · 3 items", when every item is
    memory or the running summary; otherwise "Given to the agent · after
    compaction · N items"
  - "Given to the agent · session continued · 3 items" (a fallback after
    a fresh session)
  - "· 1 cut" is added when the part cap cut items (§3.4 step 3)
  - "Agent given a summary of the conversation (manual compact)" for the
    summary card (§3.3)
- **Items:** **one row per item, always visible**, which is the owner's
  first ask:
  - an icon for the kind;
  - the name, with the file as its tooltip;
  - a chip: "AgentMux system", "Workspace", "Personal", or for a startup
    file its owner ("AgentMux", "Yours", "Hand-maintained");
  - "+ Global Memory" on a startup file that carries it, marked as a
    duplicate when the agent got it twice (§3.7);
  - "cut" or "not sent" when the part cap cut it;
  - the size, "N listed" for skills and MCP servers, or "in startup file".
- **Size advice:** when Personal Memory is large against the context
  window (`sizeBand` high or critical), the card suggests consolidating it.
- **Opening the card:** only a card with text to show can be pinned open,
  which today is the summary card. A memory card has no chevron until CD3
  gives its rows text (`virtualization/disclosure.ts`, `context_delivery`).
- **The `context_compacted` row stays** ("Context compacted 120k → 8k").
  The summary card renders right below it.
- **A repeated new-session card replaces the earlier one** (#4708,
  `supersededCard` in `memory-injected.ts`): when a session is restarted
  before its first turn (a model change, an account switch), what the
  first session was given never reached the conversation, so the new card
  takes the old one's place instead of stacking under it. Live and replay
  use the same rule.

### 3.3 The compaction summary (the owner's second ask): render-only

**Nothing new is sent.** The summary is already in the model's context,
so sending it again would deliver it twice and cost a turn.

- **Recognise** Claude Code's frame, a string-content user frame, by two
  signals: it carries `isSynthetic: true`, and it starts with `This
  session is being continued from a previous conversation`.
  - **Directly after a `compact_boundary`** (before any assistant or
    other user-text frame), either signal is enough.
  - **With no boundary in view,** both are required. History pagination
    parses each page separately, newest first (`useHistoryPagination.ts`),
    so a summary at the top of a page can't see the boundary that ends the
    older page.
  - Otherwise the frame stays a normal user message.

  Every summary frame in the local transcript store (3 of 3) has
  `isSynthetic: true` and directly follows a boundary, and no other
  string-content user frame is marked synthetic. With a boundary, the
  rule survives a change to either the wording or the flag.
- **Emit** a `context_delivery` with `reason: "compaction"`, the boundary's
  `trigger` when there is one, and one item:
  - `kind: "compaction_summary"`, name "Conversation summary (written by
    Claude Code)";
  - an excerpt taken from the first non-heading line of the summary (its
    "Primary Request and Intent" section when present);
  - the full text inline as `body`. It's already in the transcript, so
    nothing new is stored.
- **Header:** "Agent given a summary of the conversation (manual compact)
  · ~2.3k tok", with the excerpt under it. This is the owner's "Agent
  injected with summary: key points…".
- **Id:** the frame's own `uuid`, so the live copy and every replayed copy
  share one node, whichever page it's parsed on. Without a `uuid`, the id
  falls back to the boundary's id, then to the frame's timestamp and
  size. **Time:** the frame's own `timestamp`, then the boundary's.
- **It must not end a hidden-turn window,** and it's never treated as the
  user's own message. Neither the activity summary nor "last user
  message" logic should use it. As built, `CompactionSummaryTracker`
  (`context-delivery.ts`) runs in `interceptFrame` (`frame-intercepts.ts`)
  before the translator, for the live stream and replay alike, so the frame
  never reaches the stream parser.
- **The `/compact` line** the user sent stays as the user's own row. It's
  a real action, and the one they clicked.
- **Old transcripts** get the new rendering automatically, because this
  is render-only.

### 3.4 Memory deliveries (the owner's first ask)

Steps 1, 2, 3 and 5 are built (CD2a #4041, CD2b #4130). Step 4 is CD3,
not built. Each step below keeps its design and says how it was built.

1. **srv carries per-item fields.** `memory_delivery.rs` `Entry` and the
   handlers' `EntrySize`/`notice_frame` gain `kind`, `name`, `tier`,
   `bundle_id`, `path` and `excerpt`. `GlobalMemorySection` carries the
   bundle id. The running summary becomes its own item when it's
   included. The frame keeps its subtype; the new fields are additions.
   - *As built:* all but `excerpt`, which memory items don't carry, plus
     `delivered`, `source_size_bytes` and `source_tokens` (step 3), and the
     startup-file fields (§3.7).
2. **One producer for both paths: srv composes the fallback too.** Before
   CD2b the frontend composed the fallback's text itself and sent srv only
   the flattened string. srv then couldn't know which items, bundle ids or
   paths went out, and re-reading memory later could report a newer
   version than the one sent.
   - *As built:* `memorydelivery:compose {block_id, reason}` returns
     `{delivery_id, text, frame}`: the hidden message and the finished
     card frame (`compose_fallback_delivery`). srv keeps the frame in
     memory under `delivery_id` for 10 minutes (`FALLBACK_TTL_MS`). The
     hidden `AgentInput` carries `delivery_id`; once srv has passed the
     message to the agent it appends the frame, with `fallback: true`, to
     the block's output (`agent_handlers/input.rs`, `take_fallback_frame`).
     A compaction's running summary is listed as an item and appended to
     the message by the input handler (`with_state_after_compaction`), as
     before.
   - *As built, frontend:* the controller asks srv first and composes
     itself only when the RPC fails (an srv older than CD2b). Live, the
     card frame arrives while the hidden turn is being hidden, so it's
     skipped there and the controller shows the same node (same id) when
     the turn ends. On replay, the card replaces the row its hidden line
     produced, or, when the card comes first, marks that line as its echo
     (`noteFallbackCard`). §9 B1 is the case this misses.
   - **The fallback's wire format doesn't change** (an invariant, kept).
     The message is the same `<system-reminder>` envelope with
     `REINJECTION_SIGNATURE` first (`compose_fallback` in
     `memory_delivery.rs`, byte for byte the frontend's
     `composeReinjectionMessage`). Replay and srv's digest recognise a
     hidden turn's echo by it (`isMemoryReinjectionMessage`,
     `is_hidden_reinjection_text`), so it must stay. Only the item
     collection is shared with the hook, never its composed text.
3. **Record what was delivered, not the source.** The hook caps a
   delivery at eight 9,000-character parts and cuts the rest with a
   notice (`memory_delivery.rs`, `MAX_PART_CHARS`, `HOOK_PARTS`,
   `split_into_parts_counted`). Items are projected onto the text
   actually delivered (`Delivered::of`, from each item's `ItemSpan`):
   - an item wholly inside the delivered text is `full`;
   - one cut by the cap is `partial`: its size, tokens and body are those
     of the delivered slice;
   - one entirely past the cap is `omitted`, with no body and size 0.

   The card shows partial and omitted items with a "cut" mark, and the
   header says how many ("3 items · 1 cut"). The fallback path, which
   has no cap, records every item as `full`.
4. **Bodies on expand.** This revisits D6. D6 said "the user sees a label
   only", and the owner now wants to see each item.
   - Each delivery's delivered text (step 3) is stored per item under the notice id,
     in the agent's transcript zone. It's retained exactly as long as the
     transcript, and nothing new leaves the machine.
   - A new RPC `contextdelivery:get {block_id, delivery_id, item}` returns
     one body. It's authorized by **transcript**, not by block: srv
     resolves the requesting block to its agent's transcript zone
     (`agent:<defId>:current`) and serves the body only if the delivery
     was stored in that zone. A card replayed in a later pane of the same
     agent, which `useHistoryPagination.ts` does on a cross-block
     continuation, can still expand; a block of another agent can't read
     it.
   - Deliveries from before this change have no stored body. Their rows
     say "Content from before CD3 isn't kept." For Global Memory they
     offer "Open the current version in Memory" via `bundleId` (the
     Armory's Memory view is now the Memory pane, #4425).
   - Startup files (§3.7) would be stored the same way, except
     `.mcp.json`, whose text never leaves `startup_files.rs` because it
     holds the agent's signing keys. Its row stays names-only.
5. **Header per reason** (§3.2). "Re-delivered" appears only when the
   reason is compaction. A new session reads "Given to the agent · new
   session". *As built;* the resume-phase spec's Phase 3 renames the
   reasons and makes every fresh session onto history read "session
   continued".

### 3.5 The continuation packet: moved

This was CD4: a card for the `<agentmux-continuation>` record a fresh
session gets after a failed resume. It is now part of the resume-phase
spec (`SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md`, Phase 3), because
#4888 sends the record with the catch-up turn instead of the user's next
message, so the record belongs on that event's card. The card follows
this spec's model: one `continuation_packet` item (and the running
summary when it rides along), sized, with its body under CD3's rules.

### 3.6 What a delivery card must never do

These rules are about the card and its bookkeeping. Deciding what the
agent receives, and whether a turn follows, belongs to the delivery paths
(the SessionStart hook, the fallback, and the resume-phase spec's catch-up
turn), not to the card.

- **Reach the digest.** A card's frame and a hidden delivery's text are
  never read as user or assistant text by the digest, activity summary,
  title or next-prompt readers (`crates/srv/src/ambient/digest.rs` skips
  `system` frames and hidden turns; `server/app_api/session.rs`). The
  catch-up turn's reply is the exception by design: it is the agent's
  own message.
- **Leave the machine.** Bodies are stored only in the local transcript
  zone (CD3).
- **Change what was delivered.** The card describes what was sent; it
  never sends anything itself. A card for content that wasn't delivered
  is a bug (an unacknowledged hook part writes no card,
  `handle_session_start_ack`).

### 3.7 Startup files on a new session's card

Built under the launch-context spec (LC2 #4129, LC3 #4131), recorded here
because they are items of the same card:

- On a new session (hook `source=startup`) the card lists every file the
  CLI read by itself, first, then the hook's memory items. srv lists them
  from the `cwd` and `CLAUDE_CONFIG_DIR` the hook sends
  (`backend/startup_files.rs`, `claude_startup_files`). A hook binary
  older than srv sends no `cwd`, and the card shows no files.
- When a startup file AgentMux wrote carries every current Global Memory
  section (`global_memory_in_startup_files`), the hook doesn't send Global
  Memory again. The card still lists each entry with `via:
  "startup_file"` ("in startup file", size 0).
- A delivery left with nothing for the hook to carry still gets its card,
  written on the first part request (`send_partless_notice`).
- Claude only: other providers have no SessionStart hook and get no card
  (§7 Q8).

## 4. Replay and compatibility

| Transcript content | Renders as | Built? |
|---|---|---|
| Hook frames from CD2a on | a full card | yes |
| Old hook frames (no `kind`/`name` fields) | a card whose names come from the label prefix "[AgentMux System] X" / "[Workspace] X"; no bodies | yes (`toItem` in `memory-injected.ts`) |
| A fallback card (`fallback: true`) next to its hidden line | one card; the hidden line adds no row, and the reply stays hidden | yes, both orders (card first, or line first) |
| Old fallback hidden lines (before CD2b, no card) | planned: a card with names from the `# [AgentMux System] X` headings. **As built:** the old one-line "🧠 Memory reinjected" row, with entries named "Entry N" (`buildMemoryReinjectionNodeFromReplay`) | no |
| Old compaction summaries | the new compaction card, with the full body, because the text is in the transcript | yes |
| A summary at the top of a history page, its boundary on the older page | the compaction card, recognised by both signals (§3.3); no trigger shown | yes |
| A fallback card on one history page and its hidden line on the older one | should be one card. **As built:** the card, plus an "Entry N" row from the hidden line (§9 B1) | bug |
| A new-session card after an unanswered earlier one | one card in the earlier one's place (§3.2) | yes |

The parser alias planned here (`memory_reinjection` read as
`context_delivery`) was not built; both node types exist (§1.3).

## 5. Tests

- **Compaction card:**
  - a boundary followed by the summary frame gives one card, and no user
    message, live and on replay;
  - the same text without a preceding boundary stays a user message;
  - replaying twice gives one node (stable id);
  - a summary alone at the top of a page, flagged and worded, becomes the
    same card (same id) as when its boundary is on the same page;
  - the frame doesn't end a hidden window.
- **Memory card:**
  - the hook frame with new fields gives one row per item, with the right
    tier, path and size;
  - the fallback path produces the same persisted frame, built from the
    item list stored under its `delivery_id`, even if memory changes
    between compose and send (srv test in `input.rs`);
  - a delivery over the part cap marks the cut item `partial` with the
    delivered slice's size, and later items `omitted`;
  - old frames and old hidden lines render with the names in §4.
- **Bodies:** `contextdelivery:get` returns the stored text to any
  block of the same agent (same transcript zone), including a later pane
  on a cross-block continuation, and refuses blocks of other agents.
- **Fallback wire format:** the fallback's message from
  `memorydelivery:compose` still carries the `<system-reminder>`
  envelope and signature, and replay still hides its echo.
- **Digest:** a transcript with each delivery kind yields the same digest
  as one without them.
- **Live check (as for P2):** new session, `/clear`, manual `/compact`,
  auto-compact and a fresh-after-failed-resume each show one card with
  correct items, and nothing appears as a user message.

**What exists (2026-10-14):** the compaction card's cases
(`context-delivery.test.ts`, `parseHistoryLines.test.ts`); the hook
frame's items, old-label recovery, size bands, the fallback card's
replay in either order around its hidden line, and the superseded card
(`memory-injected.test.ts`); claims,
acknowledgements, the cut and the fallback card's id and ownership
(`memory_delivery_handlers.rs` tests); the fallback's wire format
(`compose_fallback_writes_the_frontends_exact_message`); hidden-turn
recognition in the digest (`digest.rs` tests).

**Missing:**
- replay of a fallback card and its hidden line on different history
  pages (§9 B1);
- the fallback card built from the stored frame when memory changes
  between compose and send (the frame is stored, so this should hold,
  but nothing pins it);
- a digest test with each card kind in the transcript;
- **the live check**, never run.

### 5.1 The live check

Runs on a dev build of `main` with #4888, on Windows, together with
#4888's own check (an account switch, a manual `/compact`, a reload after
each, a `/compact` mid-turn). For each of these: a new session, `/clear`,
a manual `/compact`, an auto-compact, an account switch (fresh session
onto history):
- exactly one card, directly under its event, with the right title and
  items (names, chips, sizes, startup files on a new session only);
- nothing shown as a user message, and no "Entry N" row;
- the same after a reload, and after scrolling the card's page out and
  back in;
- the hook delivered, or the fallback did, not both: the srv log has
  "memory delivery: complete, notice sent" or "fallback delivers", once
  per event;
- in the transcript, whether a fallback card sits before or after its
  hidden line. Replay handles both; the answer says which one B1's fix
  must expect in practice.

### 5.2 Live results (2026-10-14)

A local build at `40623c1ba` (#4888's merged head), Windows, CEF 154,
driven over CDP. Each pane's document state was read from the store, so
the results don't depend on scrolling. The agent was a throwaway, deleted
afterwards. Details: #4888, issuecomment-6174342680.

| Event | Result |
|---|---|
| New session | One card: 5 startup files, 11 Global Memory entries "in startup file". Correct. |
| Manual `/compact` | Compacted row, summary card, memory card, then #4888's catch-up reply. The hook's card landed **above** the compacted row (C2). The CLI's `<local-command-stdout>` line showed as a user message (C1). |
| Reload | Same nodes, no duplicates, the catch-up reply kept. |
| `/compact` queued behind a turn | As above, but the **fallback** delivered, so the memory card landed after the summary card (C2). |
| Fresh session onto history | Not reached: stopping the CLI and moving its session file aside starts a brand-new session, not a rejected resume. A real account switch needs a second account. |
| Transcript order | A fallback card is written **after** its hidden message (B1's fix handles both orders). |

The My Agents tile and the Swarm row showed delivered text as the agent's
last prompt (C3).

## 6. Phases

| Phase | What ships | State |
|---|---|---|
| **CD1** | Compaction summary as a card (§3.3). Frontend only, render-only. | **implemented** (#4037); docs #139 (agentmux-docs) |
| **CD2a** | Per-item memory card (§3.1, §3.2, §3.4 steps 1, 3, 5): srv fields, delivered-slice items, frontend rows. | **implemented** (#4041); docs owed (§8) |
| **CD2b** | srv composes the fallback and writes its card (§3.4 step 2). | **implemented** (#4130) |
| — | Startup files on a new session's card (§3.7). | **implemented** under the launch-context spec (#4129, #4131) |
| **CD3** | Bodies on expand (§3.4 step 4): storage, RPC, expanded rows. | proposed; needs the owner's decision (§7 Q1) |
| ~~CD4~~ | Continuation-packet card. | **moved** to the resume-phase spec, Phase 3 (§3.5) |
| **CD5** | The narration tag "ambient" → "AgentMux" (§1.3), and the ambient framework spec's note (§8). The reason and title renames moved to the resume-phase spec, Phase 3. | proposed |
| **CD6** | Fix the card bugs in §9 that this spec owns, add the missing tests (§5), run the live check (§5.1). | B1, B2, C1, C3 fixed; live check run (§5.2); C2 next |

**Every phase ships its docs with it** (§8). CD2a and CD2b did not: the
public memory page still describes the old one-line notice. That is owed
now, with CD6.

## 7. Open questions

1. **Storing bodies (CD3) reverses D6.** Store per item in the local
   transcript zone, as recommended? Or show names and sizes only?
   **Still open.**
2. **Card open or collapsed by default?** Recommended: item rows visible,
   bodies collapsed. **Built that way.**
3. **Group adjacent cards?** After a compaction, the summary card and the
   memory card appear next to each other. Recommended: keep them as two
   cards, because they arrive at different times and come from different
   sources. **Built that way.** The resume-phase spec places each card
   under its event.
4. **Rename "ambient login" and "ambient home dir"** to "host login" and
   "host home dir"? That's 189+ occurrences plus two migrations, so it
   belongs in a separate change. Recommended, as a follow-up.
5. **Dead-air re-delivery lines** replay as user messages written by the
   system. Should they become a card too ("Re-sent your message")? Not a
   context delivery in this spec's sense, so it's a follow-up.
6. **"0 personal" notices.** Check whether they come from hook calls
   without an agent token (§2.1). **Still open;** part of the live check
   (§5.1). The fallback path doesn't depend on the token: it reads the
   agent from the block (`persisted_agent_identity`).
7. **Cards that land late.** The resume-phase spec found that a card is
   drawn where its frame sits in the transcript, and srv sometimes writes
   it after the conversation has moved on (its §2.1). That spec owns the
   fix; this one only requires that a card describe what was sent.
8. **Other providers.** Codex, Gemini, Qwen and Pi have no SessionStart
   hook and get no memory card, no compaction card and no startup files.
   Worth its own spec once the resume pipeline exists (resume-phase spec
   §7).

## 8. Docs to update

User docs live in the public `agentmuxai/agentmux-docs` repo
(docs.agentmux.ai), at `src/content/docs/`.

| Doc | Change | Phase | State |
|---|---|---|---|
| `internals/conversation-overhead.md` | Say what triggers compaction, and what the pane shows afterwards: the compacted row and the summary card. | CD1 | **done** (agentmux-docs #139) |
| `glossary.md` | Add **ambient** and **context delivery**. | CD1 | **done** (#139). Its context-delivery entry names only the summary card; widen it to memory and startup files. |
| `memory.md`, the SessionStart paragraph | It still says the pane shows "🧠 Memory reinjected — 3 global, 2 personal …; hover it for each entry's size". Describe the card: a row per item with its chip and size, startup files on a new session, "cut" marks, and the fallback after a failed resume. | CD2 | **owed** (CD6) |
| `armory.md` | "Ambient provider credentials" uses the third meaning of ambient (§1.2). Leave it until the follow-up rename (§7, Q4), then say "host login". | follow-up | open |
| `agentmux` [`docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md`](SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md) | Point its notice section at the card; amend D6 ("label only") once CD3 lands. | CD2, CD3 | owed (CD6) |
| `agentmux` [`docs/specs/SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md`](SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md) | Point its label-node section at the card. | CD2 | owed (CD6) |
| `agentmux` [`docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`](SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md) | State that "ambient" names how a call is made, and that content sent to the agent is a context delivery. | CD5 | open |

## 9. Known bugs (2026-10-14)

Found by reading code; none reproduced in a running app yet. B1 and B2
come from the resume-phase spec's §2.1 and were handed to this spec by
its author (agreed on #4888).

| # | Bug | Owner |
|---|---|---|
| **B1** (fixed, #4901) | **A fallback card shows twice on replay** when its hidden line and the card fall on different history pages. Each page is parsed separately, so the page with the hidden line makes an "Entry N" row (`buildMemoryReinjectionNodeFromReplay`) and the other page makes the card. Fix: when pages are joined, drop a `memory_reinjection` row that sits next to a `fallback` card with no agent work between them (the reply is hidden, so nothing should be). | this spec, CD6 |
| **B2** (fixed, #4909) | **A compaction seen late can re-deliver memory the hook already delivered.** `claim_boundary` looks only at hook deliveries from the last 60 s (`CLAIM_WINDOW_MS`), never at `completed`. A boundary that reaches the pane more than a minute after the CLI wrote it (a reconnect, a remounted pane reading the gap) gets "deliver", and the agent gets its memory twice in a hidden turn. Fix: date the claim by the boundary's own arrival on srv (`boundary_origins` already records it) and stand down when `completed` has a hook delivery after it, as `claim_event` does for a fresh session. | this spec, CD6 |
| B3 | Cards written after the conversation moved on (late fallback, a mid-session respawn, a compaction claim racing the hook). | resume-phase spec, §2.1 (1–3) |
| B4 | Another block's lines in the shared transcript zone can trigger this pane's fallback (`fresh` outcome). | resume-phase spec, Phase 1 (`foreign`, recency) |
| **C1** (fixed, this PR) | **The CLI's `<local-command-stdout>` frame showed as a user message.** After `/compact` (and other local slash commands) Claude Code echoes a plain-string user frame wrapped in `<local-command-stdout>…</local-command-stdout>`, here "Compacted PreCompact [agentmux-bashwrap precompact --trigger=manual] completed successfully". Nothing recognised the tag, so the translator made it a `user_message`, live and on replay. Now `commandOutputNode` (`context-delivery.ts`) turns it into a one-item card, "Claude Code command output", with an excerpt that opens to the text, in `interceptFrame` for live and replay alike; an empty one is dropped. A text card also stops `supersededCard`, so a later new-session card never replaces it. | this spec, CD6 |
| **C2** | **The memory card's place depends on which path delivered.** The hook's card is written when its parts are acknowledged, which can be before the stream reaches the compaction boundary, so it lands above "Context compacted". The fallback's card lands after the summary card. | this spec, CD6 (handed over on #4888) |
| **C3** (fixed, this PR) | **Readers outside the pane took deliveries for the user's prompt.** The My Agents tile showed Claude Code's compaction summary ("This session is being continued…") as the last prompt, and the Swarm row showed the "# Session Context" startup message as the activity line. §3.6 covered the digest, titles and ghost text, but not these two. Now the tile's reader (`history_summary.rs`, `user_prompt_in_line`) skips `isSynthetic` user frames and the summary's opening words, and the pane (`useAgentActivitySummary.ts`) neither stores the startup message as `term:last_prompt` nor sends it to the title model. | this spec, CD6 |
