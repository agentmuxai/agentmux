# Report: incoming jekts in a collapsible panel on the right of the agent pane

**Task:** #23219 (research and design); the build is #23220 and waits on the operator's approval.
**Asked by the operator (2026-11-04):** "can we have the jekt messages appear in a collapsible panel on the right, similar to the kanban's task description. so everything would work the same, but the jekt lines would run in the detached panel. lets research and design a best practice doc."
**Author:** Naki-2@narko. **Read on:** `origin/main` at `a158acd8c` (2026-11-04), AgentMux 0.60.14. **Constraints from AgentX** (2026-11-04) are in section 6 and shape the design; where something below differs from what AgentX assumed, it says so.
**Status:** proposed. AgentX approved it for the operator (2026-11-04) and agreed with the recommendation on all nine questions in section 8; his four corrections are folded in below (marked "AgentX"). The operator's approval is still to come. No code.

---

## 1. In one paragraph

Today an incoming jekt is one row in the transcript, between the person's turns and the agent's replies, and an agent that gets many jekts (a leader's pane gets one every few minutes while it is active) has a transcript that is half other agents. The design moves **the body and the metadata of an incoming jekt to a panel on the right of the pane** and leaves **a one-line row in the transcript** where it arrived, so the timeline still reads. Nothing about delivery, the marker, the history file or what the agent reads changes: it is a second view over the same document nodes, not a second store. The panel docks beside the transcript in a wide pane and becomes an overlay in a narrow one. A message that waits on the person never hides in it (it keeps a card and a waiting-for-you entry; today it has neither, see 2.4). Recommended: ship it as a per-pane setting, build in four PRs, no reply box in the first version.

## 2. How it works today (verified in the code, 2026-11-04)

### 2.1 From marker to row

- Every jekt arrives as `[JEKT:FROM=… TO=… TIER=… DELIVERY=… TRUST=… MSGID=… PRIORITY=… TS=…]` (plus `HELD_FOR`, `ESCALATE`, `FILES`, `SIG`, `INSTANCE`, `INSTANCE_STATUS` when they apply) and a body, written by srv (`crates/srv/src/backend/reactive/sanitize.rs`, `wrap_jekt_message_with_files`) into the agent's input stream. The sender gets an echo with the same marker (`crates/srv/src/server/jekt_echo.rs`), which is how the pane tells outgoing from incoming (`FROM` equals this pane's agent).
- The window parses it in `frontend/app/view/agent/stream-parser.ts` (`tryParseJekt`, a `[JEKT:` block regex) into a `JektMessageNode` (`frontend/app/agent-model/node-types.ts`): `from`, `to`, `message` (the body with srv's scaffolding stripped), `raw` (the whole block), `tier`, `deliveryTier`, `trust`, `msgId`, `priority`, `direction`, `timestamp` (the sender's `TS`), `heldForSecs`, `attachments`. A jekt that arrives while a text or thinking block is open is held and released when the block ends.
- Since #6059 (Maop) there is a second shape: a `SystemNoteNode` with an `origin` (`MessageOrigin`, generated from srv): `kind`, `from`, `trust`, `tier`, `delivery`, `sig`, `msg_id`, `sent_ms`, `delivered_ms`, `held_for_s`. It carries the messages that are not a jekt marker: a schedule, a task, a supervisor turn, a review or CI notice. The container is `OriginContainer` (`frontend/app/view/agent/components/OriginContainer.tsx`); its jekt variant (`JektVariant`) shows a header (direction arrow, "Jekt from X", tier tag, delivery tag), the body with links, a metadata row (From, To, MSGID, Trust, Priority, time, "Held for …") and a `Raw payload` details block. `DocumentRow.tsx` dispatches to it.
- **Finding:** `ESCALATE` is **not carried to the window at all**: it is in neither `JektMessageNode` nor `MessageOrigin` (searched `frontend/app` and `frontend/types/rpc`). AgentX asked the panel to show TRUST, TIER and ESCALATE from the same data as the transcript container; for ESCALATE that data does not exist yet and srv has to add it (PR 2 below).

### 2.2 Which message kinds there are, and what each does now

| Kind | Today | Panel? (recommended) |
|---|---|---|
| Incoming jekt from an agent (`jekt_message`, incoming) | One row, `JektVariant`, collapsed or open by the `jekts` switch | **Yes** |
| Review / CI notice (github-consumer), a bulletin, a message from `cron` | A `system_note` with an `origin` whose kind is not `system`; it follows the **same `jekts` switch** (`verbosity.ts`) | **Yes** when `origin.from` is another sender (agent, github-consumer, bulletin); **no** for the pane's own triggers (schedule, task, supervisor turn) |
| The agent's own outgoing jekt (echo, `direction: "outgoing"`) | A row like an incoming one, with an up arrow | **No**: it belongs to what the agent did, next to its `Message` tool call (AgentX 6: replies read the same) |
| Swarm broadcast (`[BROADCAST:FROM=user VIA=swarm …]`) | The person's own words, a user message with broadcast metadata | **No**: it is the operator's voice |
| `[AgentMux] …` system notes | `NoteVariant` (flat), the `notes` switch | **No**: they are about the pane and the session (an upgrade, an overlap), not a message from someone |
| `[JEKT-QUOTED:` / `[BROADCAST-QUOTED:` | Plain text, never a marker | n/a |
| Held / late (`HELD_FOR`, `(sent 14:30, delivered 14:41)`) | Shown in the metadata row; the time is the sender's `TS` | Same fields in the panel row |

### 2.3 Rules the panel must keep

- **Verbosity (#5996):** `verbosity.ts` has five switches; `jekts` covers jekt rows and non-system origins. Values `auto | expanded | collapsed | hidden`; presets Full (auto), Standard (jekts collapsed), Replies (jekts hidden). **Never hidden, whatever the setting:** a sensitive jekt, or one whose trust is not `*-verified` (`forced`). That rule stays.
- **Virtualisation:** a collapsed jekt row is estimated at 32 px (`COLLAPSED_MESSAGE_PX`); an open one up to about 290 px (`estimateExpandedJekt`). The collapsed row is already the "stub" the operator asked for.
- **Resumed history** re-parses the stored lines with `isReplay: true` into the same nodes. A panel computed from the document gets history for free.
- **Existing tests that pin the rendering:** `OriginContainer.jekt.test.tsx`, `OriginContainer.test.tsx`, `stream-parser.test.ts`, `stream-parser.origin.test.ts`, `virtualization/renderers.test.ts`, `virtualization/disclosure.test.ts`. They keep passing in inline mode.

### 2.4 What waits on the person

- srv already raises an OS notification for a jekt with `ESCALATE=required` (`NotifyKind::MessageNeedsReview`, in `backend/notify/policy.rs`; fixed text, never the message).
- In the window **there is no card and no waiting-for-you entry for it**: `frontend/app/notification/waiting-for-you.ts` registers only a question, a tool permission, and what srv announces through `userattention` (browser hand-off, SSH consent, widget install). The sensitive jekt is "open by default" in the transcript and that is all. AgentX's constraint 3 ("keeps the card in the dock and registers with waiting-for-you") is therefore **new work, not preservation**, and has to land before or with the panel, or a collapsed panel would make today's weaker guarantee worse.
- The docked area under the transcript is `AgentBottomPanels.tsx` (login, decision and question panels, queue, banners, activity dock). Each panel must stay a direct flex child of `.agent-view-zoomed` (its header comment: the transcript's re-pin relies on it). The request stack (permission, question, cards) shares one 40% cap. A "needs you" jekt card belongs in that stack.

### 2.5 Layout and scroll

- `.agent-view` (a column, `container-type: inline-size`, name `agent-pane`) holds `.agent-view-zoomed` (the transcript and the docked panels, also an `agent-pane` container, carries the per-pane zoom) and, as a sibling, the shell drawer. There is **no side area** in the agent pane today.
- **One writer of the transcript's scroll position:** `scroll/one-way-flow.ts` (exports `scrollTranscriptTo` and `scrollTranscriptBy`); `scripts/check-transcript-scroll-writers.mjs` fails CI on any other `scrollTop =`, `scrollTo(`, `scrollBy(` or `scrollIntoView(` under `frontend/app/view/agent/`. Exempt files are listed with what they scroll (two tool preview boxes). `scroll/follower.ts` holds the follow logic. A second scroller (the panel's list) cannot have its own scroll code; see 5.5.
- Per-pane view state lives in block meta (the verbosity switches are stored there); the panel's state uses the same mechanism.

### 2.6 The Kanban's description panel (the model the operator named)

It is `KanbanDrawer` (`frontend/app/view/kanban/kanban-drawer.tsx`, opened from `kanban-section.tsx`): an **overlay**, not a docked split. `.kanban-drawer` is `position: absolute; top/right/bottom: 0; width: min(440px, 92%); z-index: var(--z-pane-overlay); border-left; box-shadow`, with a head (title, facts, close), sections, and a body that scrolls itself. It is specific to the Kanban (it knows tasks, events, moves), so "reuse" means **reuse its shell and tokens, not the component**: PR 1 below extracts the shell into a kit `SidePanel` that both use.

## 3. Best practice in comparable products

Written from what I know of these products; I did not re-check their current behaviour for this report, so treat the specifics as the pattern, not a quotation.

| Product | What it does | What to take |
|---|---|---|
| **Slack** (threads, Activity, Later) | A thread opens in a right pane beside the channel, and the channel keeps a one-line "N replies" stub; unread is a bold name plus a count badge; the Activity view lists mentions and DMs in time order with filters; a message links back to its place in the channel. | The right pane plus a stub in the main flow; a count on the collapsed handle; click jumps back |
| **Discord** (Inbox, pinned messages, threads) | A right sidebar for members/threads, an Inbox popover for mentions with an "unread" tab and "mark all read"; unread is a badge on the channel. | Unread tab and one "mark all read"; no popover for a feed that runs all day |
| **Microsoft Teams** | Activity feed in a left rail; chat and channel open to the right; unread dot and count. | Unread dot while collapsed; the feed is for catching up, not for working in |
| **VS Code** (Notifications centre, Output, Problems panels) | A bottom/side panel the user toggles; a status-bar bell with a count and "Do not disturb"; the Output panel auto-scrolls until the user scrolls up, then stops; filter by source. | One toggle; a count on the toggle; follow-the-tail that stops when the user scrolls and resumes at the bottom; filter by source (= by sender) |
| **GitHub** (notifications inbox, PR timeline) | Inbox with unread/read, filter by reason, grouped by repository; the PR timeline shows an event line that links to the full thing. | Group by sender; keep a thin event line in the main timeline |
| **Agent tools that separate a side channel** (e.g. an IDE agent's "sub-agent" or "background task" tray; chat clients' "notifications" panes) | The main conversation stays the human's; machine-originated events go to a tray with a count, and anything needing approval pops into the main flow. | The rule this design uses: **approvals never live only in the side channel** |

Common practice, in one list:
1. **A count on the collapsed handle**, capped ("99+"), cleared when the user has seen the messages, not when they arrive.
2. **A stub in the main flow** that links to the full item (Slack's reply count, GitHub's timeline line).
3. **Follow the tail until the reader scrolls away**, then offer "jump to latest" (VS Code Output); never yank the view.
4. **Filter by sender** and by "needs me", search in the same box as the main view; grouping runs of one sender (within a few minutes) under one header.
5. **Anything that needs an answer is promoted** out of the quiet channel (a badge that does not clear, a card where the work is).
6. **The side panel is for reading and triage**; replies happen where the conversation is.

## 4. Evidence from the real case: a leader's pane

A leader's pane gets far more messages than any other agent's: while it is active, one every few minutes, each usually a paragraph long and sometimes several screens. Most agents get a few an hour.

So the panel must be comfortable at **a message every few minutes, in bursts, each usually a paragraph and sometimes much longer**. That fixes three choices: rows are one or two lines until opened; the list scrolls and follows; the unread count must mean something at that rate (it cannot be "messages since the pane was opened").

## 5. The design

### 5.1 Layout

```
 Wide pane (the pane's own width >= 900 px, docked)
 ┌───────────────────────────────────────────┬────────────────────────┐
 │ transcript                                │ Messages           ▸ ✕ │
 │  you ▸ "check the PR"                     │ ────────────────────── │
 │  claude ▸ looking…                        │ [All▾] [Needs me] [●3] │
 │  ↓ Jekt from AgentY · "Naki-2: #23219 …"  │ ┌ AgentY · 14:31 ····● ┐│
 │  claude ▸ on it                           │ │ #23219 is yours,     ││
 │  ↓ Jekt from Camper-2 · "FYI my PR …"     │ │ research and design… ││
 │                                           │ └ host-verified · coord┘│
 │ ───────────── composer ──────────────     │ ┌ Camper-2 · 14:42     ┐│
 └───────────────────────────────────────────┴────────────────────────┘

 Collapsed:  the panel is a 28 px rail on the right edge: "Messages" turned 90 degrees, a count badge (3), and
             a "needs you" dot that does not clear.

 Narrow pane (< 640 px, overlay): a "Messages (3)" button in the pane's header row opens the same panel
             over the transcript, like the Kanban drawer (min(440px, 92%), shadow, close).
```

- The panel is a **sibling of `.agent-view-zoomed`** in a new row wrapper, not a child of it. That leaves `AgentBottomPanels`' rule (each panel a direct flex child of `.agent-view-zoomed`) and the transcript's re-pin untouched, and `@container agent-pane` then measures the real, narrower transcript column, so every existing breakpoint behaves as it does today in a narrow pane. The composer stays under the transcript only. The panel takes the pane's zoom.
- **Collapse state, width and "seen up to" are per pane, in block meta** (AgentX 4), next to the verbosity switches (one new key, for example `agent:messages` = `{ open, width, seen }`).
- Width: 320 px default, draggable between 280 and 480; never more than 45% of the pane. The kit's resize handle, if it has one; otherwise fixed 360 px in the first version.
- **Narrow rule** (AgentX 5): when the pane's width cannot give the transcript at least its present minimum, the panel is an overlay opened from the header button, never a squeeze. I did not find a single "pane minimum width" constant in the agent code (the layout's minimum is in `frontend/layout/lib/layoutGeometry.ts`, which I did not trace); the build reads that value and uses the transcript's current narrowest usable width as the threshold. The numbers above (900 and 640) are starting values to confirm by screenshot.

### 5.2 One message in the panel

Collapsed (the default, one or two lines):

```
 ┌ AgentY                        14:31   ●        ← unread dot until seen
 │ #23219 is yours, research and design: incoming jekts in a collapsible…
 │ [host-verified] [coord]                            ← Tags from the kit; the same tone rules as OriginContainer
 └
```

Open (click, or Enter): the whole body (markdown and links as the transcript renders them), attachments, then the metadata row exactly as the transcript's open jekt shows it (From, To, MSGID, trust, priority, time, "Held for …", and **ESCALATE** once srv carries it), the `Raw payload` block, and two actions: **Show in conversation** (jumps to the stub) and **Open in Audit** (the Audit row for that MSGID, see 5.6).

- Sender, trust, tier, delivery and time come from the node or its `origin`; the panel **does not parse or re-derive anything** (AgentX 2). One shared function renders the tags for both places, so the transcript's open row and the panel row cannot disagree.
- Tone: sensitive is error-toned, an unproven sender warn-toned (as `JektVariant` does now); a forced message sits above the list in a "Needs you" strip, not in the time order.
- **Runs:** consecutive messages from one sender within 5 minutes share a header. At a busy pane's rate this mostly means a back-and-forth with one agent reads as one block.
- **Sort and follow:** oldest at the top, newest at the bottom, like the transcript. The list follows the tail until the reader scrolls up; then a "↓ N new" pill appears (see 5.5 for how, given the scroll rule).

### 5.3 What stays in the conversation (the stub)

A jekt starts a turn, or is delivered between a turn's steps, so the transcript must still show where it arrived. In panel mode every incoming jekt row renders as the **existing collapsed row** (32 px, already estimated and virtualised): the arrow, "Jekt from AgentY", the first words, the tags, and nothing else. It has no inline expand; clicking it opens the panel on that message (`Show in the panel`). Nothing is added to the node list and nothing changes in what the agent reads: **the history file, the marker and the text the model receives are untouched** (the task's point 4). The `jekts` switch keeps its meaning: `hidden` hides the stub as it hides the row today; forced messages (sensitive, unproven) keep their full row inline, as now.

The agent's own reply to a jekt, its `Message` tool calls, the outgoing echo and everything else in the turn stay where they are.

### 5.4 Messages that wait on the person (AgentX 3)

1. srv puts `escalate` on the signed origin (a field on `MessageOrigin`, so the window reads it from the same place as trust and tier), and the parser keeps `ESCALATE` on the jekt node.
2. A jekt with `TIER=sensitive` and `ESCALATE=required` registers a source with `waiting-for-you.ts` (so the pane gets the tone, the badge and the #18210 toast), and shows a card in the request stack of `AgentBottomPanels`, with the sender and trust, **Show message**, and the same "confirm in your own conversation" wording the full row uses. **AgentX:** it does not end on a guess that the person "answered in their own conversation". It ends when the person next sends in that pane, on an explicit **Dismiss**, or when the pane closes (the same `stillWaiting` hook questions use).
3. It is shown in the panel too, in the "Needs you" strip, and the collapsed rail's dot stays lit while it exists. It never counts as read by the panel being open alone.
4. `TIER=sensitive` with `ESCALATE=none` is informational (the sender is proven): ordinary row, sensitive tone, forced inline stub as now.

### 5.5 Scroll, without a second writer (AgentX 1)

The panel's list is a scroller, so it needs a follower and a "jump to node". The rule and its CI check stay as they are: **no scroll call in the panel's code**. The build extends `scroll/one-way-flow.ts` (the one writer) so that it can drive more than the transcript: a second `follower` instance for a named scroller, exported as `scrollMessagesTo` and `scrollMessagesBy` next to the existing two, and the check adds the panel's file to the files that may only *call* those. **AgentX checked:** `Follower` is a class (`follower.ts:58`) that the transcript controller creates per instance (`one-way-flow.ts:124`), and `scrollTranscriptTo`/`By` are plain functions. So the panel's follower is a new instance and `scrollMessagesTo`/`By` sit beside the two existing exports; the panel's file is added to `scripts/check-transcript-scroll-writers.mjs` as call-only. The risk is smaller than first thought. The jump from a panel row to its stub, and from a stub to its panel row, are `scrollTranscriptTo` and `scrollMessagesTo` calls with a short highlight.

### 5.6 Unread, filter, search, grouping

- **Unread** = incoming messages after the pane's `seen` marker (a node id/time in block meta). **AgentX:** the marker is written debounced (on panel close, on pane blur, or at most every few seconds), never per scroll tick. A message counts as seen when the panel is open, the pane is focused and the row has been in view for a moment, or by **Mark all read**. The collapsed rail shows the count (capped "99+"); the Swarm row and the pane's tab can show the same number as a small chip (second step). A "needs you" message never clears by being scrolled past.
- **Filter chips:** All, Needs me, Unread, then one chip per sender that has written in this pane (top 5, "more" in a menu). **Search:** the pane already has a search bar (`_search.scss`); the panel adds none in the first version. Text search across the panel is a follow-up if the operator misses it.
- **Several panes:** each pane has its own panel and state. **Swarm:** unchanged except the optional unread chip. **Mobile viewer:** I did not read the mobile code for this report. Recommendation: the mobile feed keeps its current inline rendering in the first version (a phone has no room for a side panel); a "Messages" tab there is a separate piece of work that can reuse the same selector.

### 5.7 Reply

**No reply box in version 1.** A jekt is addressed to the agent; the agent answers with its `Message` tool, which appears in the transcript. If the person wants to say something to the sender, they tell this agent in the composer (the person's words go to the agent, and the agent decides), or message the other agent from its own pane. A reply box in the panel would send as the person into another agent's pane, which is a new path with its own trust questions, and the operator did not ask for it.

### 5.8 Components, theme and motion (AgentX 7)

- Kit components only (`Button`, `IconButton`, `Tag`, `Select` and the disclosure used by `CollapsibleMessage`); no hand-made buttons. The panel's shell is a new kit **`SidePanel`** extracted from `.kanban-drawer` (position, head with title and close, scrolling body, border, shadow, stacking token), used by the Kanban drawer and the agent pane's overlay form, so the two look and behave alike. The docked form reuses the same head and body without the overlay shadow.
- Motion: the open/close slide and the highlight use the repo's motion tokens and respect reduced motion (`check-motion-gates` and the motion baseline are the check); no new durations.

### 5.9 Interaction with the Audit live log (#21666, approved)

| | Audit (all agents, one machine) | Messages panel (one agent's pane) |
|---|---|---|
| Question it answers | Who sent what to whom, when, how it travelled, across the fleet | What did this agent just receive, readable, and what needs the person |
| Message text | **Never** (a hash and a length) | **Yes**, read from this pane's own transcript |
| Source | `jekt-audit.jsonl` and the segment ledger | The pane's document nodes |
| Live | Yes (the Audit's own spec) | Yes (the transcript is live) |

They do not duplicate because they do not share a store or a purpose. They link: the panel's **Open in Audit** opens the Audit filtered to the MSGID (the audit row's `request_id` is the marker's `MSGID`, per the Audit spec 3.6), and the Audit's "open message" link, which opens the receiving agent's History at the time of the jekt, gains a better target when that pane is open: its panel, at the message. The panel never writes to the Audit, and the Audit never keeps text.

## 6. AgentX's seven constraints, and where each is met

| # | Constraint | Where |
|---|---|---|
| 1 | One scrollTop writer, one follow controller; the list gets its own container through the same controller | 5.5 |
| 2 | Origin from the signed origin; same data as the transcript container; TRUST/TIER/ESCALATE visible; don't re-derive | 2.1, 5.2 (ESCALATE needs srv to carry it) |
| 3 | A waiting jekt never hides: card in the dock and waiting-for-you.ts (tone, badge, #18210 toast) | 2.4 (does not exist today), 5.4 |
| 4 | Collapsed shows an unread count; state per pane in block meta | 5.1, 5.6 |
| 5 | Narrow: overlay or tab, not a squeeze | 5.1 |
| 6 | Replies, resumed history, verbosity read the same; only placement moves | 2.2, 2.3, 5.3 |
| 7 | Kit components and motion policy | 5.8 |
| - | Which of the old rows stay inline | 2.2 (outgoing echo, the agent's replies, broadcasts, `[AgentMux]` notes, the pane's own triggers, forced messages) and 5.3 |

## 7. Build plan (task #23220), in this order

| PR | Contents | Notes |
|---|---|---|
| 1 | Kit `SidePanel`; the Kanban drawer moves onto it | No behaviour change; a pure refactor with the Kanban tests as the proof. **AgentX:** Agent1 owns the kit: tell Agent1 before adding it and take their review of the API. |
| 2 | `escalate` on `MessageOrigin` and the jekt node (srv + types); the waiting-for-you source and the request-stack card for `ESCALATE=required` | Ships alone and is useful without the panel; it fixes today's gap. **AgentX: do this one first.** |
| 3 | The panel: selector over the document, row wrapper, block-meta state, unread, stubs in panel mode, filters; the follower generalised in `one-way-flow.ts` | The risk is the follower (5.5); gated by a pane setting, default off |
| 4 | Narrow overlay and header button; Swarm/tab unread chip; Audit links; flip the default after a soak | Screenshots at three widths |

Tests: the panel selector (what moves and what stays, per kind), unread arithmetic, the forced-message rule, the scroll-writer check with the new exports, the existing `OriginContainer` tests in inline mode, a vitest for narrow versus wide, and a long-run check with a busy pane's rate (a replayed hour of messages) for follow behaviour. Rust: the `escalate` field on the origin. Two runs of the build, as asked ("keep runs to 1 or 2"): PRs 1 and 2 first, then 3 and 4.

## 8. Open questions for the operator (recommendation first)

1. **Placement: docked or overlay?** Recommended: **docked beside the transcript when the pane is wide, an overlay when narrow.** The Kanban's panel is an overlay only; an overlay in a wide pane would hide the conversation the messages are about, which defeats "run in a detached panel".
2. **Default.** Recommended: **a per-pane setting, off at first, then on by default once the operator has used it for a week** (PR 4). Alternative: on from the start.
3. **Which messages move?** Recommended: incoming jekts from agents, bulletins and review/CI notices. **Not** your broadcasts, `[AgentMux]` notes, the agent's own sends, or the pane's own triggers.
4. **A stub for every incoming jekt, or only where it started a turn?** Recommended: **every one**, as the existing collapsed row: a message that arrives between tool calls has no turn of its own, and a timeline with holes is worse than one with stubs.
5. **Reply from the panel?** Recommended: **no** (5.7).
6. **When is a message read?** Recommended: **seen when the panel is open, the pane is focused and the row was in view; "Mark all read" available; "needs you" never clears by scrolling.**
7. **Messages that wait on you (the new card and waiting-for-you source).** Recommended: **build it first (PR 2), whether or not the panel is approved**: today a sensitive jekt has no card and no waiting-for-you entry.
8. **Mobile.** Recommended: **leave the mobile feed inline for now**; a Messages tab is a later piece.
9. **Unread chip on the Swarm row and the pane tab.** Recommended: **yes, in PR 4**, the same number.

## 9. Not in this report

Changing what a jekt is, how it is delivered, signed or held; the Audit's own build; a panel for non-jekt content (tool output, the Activity dock); search across all panes' messages; sending from the panel.
