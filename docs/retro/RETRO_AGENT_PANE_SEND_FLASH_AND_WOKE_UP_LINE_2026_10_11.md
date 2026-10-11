# Retro: sending a message flashes the agent pane and adds a "Woke up: …" line (2026-10-11)

**Status:** retro
**Reported by:** the operator, using an agent pane.
**Affects:** every Claude Code agent pane, in the live view, since #4503 (`ca8002bf0`, 2026-10-09), which added the "Woke up" line.
**Fixed:** in the PR that adds this retro, by removing the line (§6). §5.2 lists two other possible contributors to the flash that are not fixed here and should be measured.

## 1. What the operator saw

After an agent pane had been idle for a while, sending it a new message made the conversation flash. An ambient line also appeared under the message, reading `Woke up: <something>`. The operator asked for a root cause of the flash, and for the line to go: it isn't needed.

## 2. Root cause

There are two defects, and the second turns into the flash.

### 2.1 The "Woke up" line lands on the user's own turn

`frontend/app/view/agent/task-wake.ts` (#4503) watched the stream for a pass the CLI starts by itself when a background task finishes: a main-agent `system/task_notification`, then a `system/init` with no input in between. On that `init` it added an ambient row, `Woke up: <the notification's summary>`.

Two facts break that rule:

1. **Claude Code sends a `task_notification` for every Bash call, foreground ones too**, mid-turn, when the call finishes. The turn then ends with `result`. The detector kept the notification pending through the `result`, so after any turn that ran a Bash command, a notification was still pending when the turn ended.
2. **The live view never parses the user's own message.** srv writes the user's message to the CLI's stdin and persists a user record (`{"type":"user","message":{"content":"…"}}`) just before that pass's `init`. Live, that record arrives as an *echo* event: `TranscriptCursor` hands it to the `EchoLedger` to pair with the bubble already on screen, and never parses it (`transcript-cursor.ts`, `ev.echo`). So the detector never saw the input that should have cleared the pending notification.

Put together, live: the user sends a message after a turn that ran Bash. The CLI starts that message's pass with `init`, the detector still holds the previous turn's last notification, and it adds `Woke up: <the previous turn's last command>` directly under the user's message. For example, the operator's "whats next?" was followed by `Woke up: Merge PR 179 and confirm` (§3).

On replay the persisted user record is a normal line ahead of the `init`, so the detector clears and the line isn't there. The live view and a reopened pane disagreed.

### 2.2 That row brings back the "flash on send"

New rows in the live tail fade in (opacity 0 to 1 and an 8px rise, 180 ms, `@starting-style` in `frontend/app/view/agent/styles/_document.scss`). The user's own message is exempt, because the fade on it *was* the flash on send: the row mounts invisible, the pinned pane scrolls up to make room (an empty strip), then the row fades in. That was measured and fixed on 2026-10-01 (`REPORT_LIVE_FEED_ROLL_OFF_TOO_AGGRESSIVE_2026_09_30` §7: layout shift 0.09 → 0.00, three changing frames → one), and the rule's comment describes it.

The false "Woke up" row is an `ambient_narration` row, not a `user_message`, and it arrives at the pass's `init`, a moment after the send. It lands at the bottom of the tail, directly under the message (`StreamFlush` only appends, `store/agent-document/reducer.ts`), and gets the fade. So on a send that followed a turn with a Bash call, the pane did exactly what the 10-01 fix removed: an empty strip under the message, then a row fading in. That is the flash.

### 2.3 Why it looked tied to idling

It isn't the idle time. It is the previous turn having run a Bash command, which almost every turn of a coding agent does. The operator notices it on returning to a pane, because that is when they send the next message. (Idle time does make the two candidates in §5.2 more likely.)

## 3. Evidence

This pane's own stream (agent `efb53e4f…`, its `output` file in the transcripts filestore, 156,093 lines), replayed through the detector's rules:

| | Count |
|---|---|
| Inputs (user messages and jekts) | 196 |
| `task_notification` lines | 1,450, of which 1,392 foreground Bash calls and 58 background |
| "Woke up" lines on replay | 65 |
| "Woke up" lines live: on a pass the CLI started itself | 65 |
| "Woke up" lines live: on a pass the user's own input started (false) | **64** |

So roughly one message in three got a false line live. Examples, each the notification left pending by the previous turn, then the user's next message:

- notification "Merge PR 179 and confirm", then "whats next?"
- notification "List screenshot-related PRs and who signed them", then "right, so did we ever get our collection of screenshots into…"
- notification "Check tracking issue and open PRs", then an AgentA jekt

Line order on a user-started pass, from the same stream: `result/success` (the previous turn), then the user record, then `system/init`, then `system/status` and the reply. The user record is the line the live view routes to the echo ledger.

Code:
- The pending notification survives `result`: `task-wake.ts`, `createTaskWakeDetector` cleared `pending` only on input or `init`.
- Echoes not parsed: `transcript-cursor.ts`, the `ev.echo` branch in event delivery (`this.deps.echo(…)`), and `EchoLedger`.
- The fade and its `user_message` exemption: `_document.scss`, `.agent-document-streaming-buffer[data-animate] .agent-document-row:not([data-node-type="user_message"])`.

Not captured on video: §7 says how to confirm the fix in the running app.

## 4. Impact

- About a third of the messages sent to a Claude Code pane got a false "Woke up" line live, naming a command from the previous turn as if a task had woken the agent. That misleads about what started the turn, which #4503 set out to make clear.
- Each of those sends flashed.
- Live and a reopened pane disagreed (the line was gone on reopen), so the transcript was unstable.

## 5. Why nobody caught it

### 5.1 The tests used the wrong model

`task-wake.test.ts` fed hand-made line sequences in which a notification is followed by an `init` or by input. No test had a foreground notification inside a turn followed by `result`, the most common case in a real stream. No test ran the live path, where the input is an echo and never reaches the detector. The replay tests could not show it, because replay is correct.

### 5.2 Other possible contributors to the flash (not fixed here)

Found while tracing the send path, and plausible, but not shown to be what the operator saw:

1. **Previous turn's rows remount when a send moves the frontier.** The live tail is scoped to the turn: a new user message moves the previous turn from the always-mounted tail into the virtualized head (`virtualization/streaming-buffer.ts`, `turnScopedFrontier`). `advanceFrontier` (`AgentDocumentVirtualList.tsx`) defers only rows above the viewport with a recorded height for that exact node object, and moves anyway when more than `TURN_TAIL_MAX_DEFERRED_NODES` (12) rows are deferred. A long turn has more than 12 rows, so its visible rows remount. Its own comment says a remount "re-renders content — async markdown like mermaid would flash". Idle makes it likelier: heights aren't recorded while the pane is hidden (a 0px reading is skipped), and a node that changed meanwhile has no matching height.
2. **The send-time unload pass.** When a new message becomes the last node, `useLiveFeedRollOff.ts` unloads finished, collapsed tool results over 8 KB before it (`tool-result-unload.ts`, `planUnload`). It skips tools still in the Activity Dock, whose retention is 8–15 s, so after any idle it unloads the previous turn's visible ones too. Those rows get new node objects, re-render, and lose their recorded heights, which feeds item 1.

## 6. Fix

1. **The "Woke up" line is removed**, live and on replay, as the operator asked: `task-wake.ts` and its test are deleted, and the detector is unwired from `frame-intercepts.ts`, `useAgentStream.ts` and `parseHistoryLines.ts`. With no false row on a send, the fade under the message (§2.2) no longer fires. `frame-intercepts.test.ts` now asserts that a notification followed by an `init` adds nothing.
2. **The Working row and the Worked line still name a task trigger** for a pass a task started: they come from srv, which sees every input it writes and so isn't fooled (`SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md` §5.4). The spec's transcript paragraph is marked removed, with a pointer here.
3. **Not changed:** §5.2's two candidates. They need a measurement in the running app before any change (§7).

If the line is ever wanted back, two changes make it correct: clear the pending notification at the turn's `result` (a foreground notification always comes before its turn's `result`; a background task that finishes while idle comes after it), and feed echo records to the detector (the cursor delivers lines in order, and the echo comes before the `init`). Also exempt `ambient_narration` from the enter fade, or it brings the flash back.

## 7. Verifying in the running app

On a build with this fix, in a Claude Code pane:

1. Ask the agent for something that runs a foreground Bash command, and let the turn finish.
2. Wait a minute, then send a message.
3. Expect: no "Woke up" line, and no empty strip or fade under the message. Turn on `agent:virt:partition` logging to see whether the previous turn's rows remount (§5.2 item 1).
4. Repeat after a long turn (more than 12 rows) with large tool results, and after the pane was hidden for a while. A flash that remains there is §5.2, not this defect.

## 8. Lessons

- A detector that infers "nothing was written" from a stream must see what *was* written on every path it runs on. Here the live path filtered the input out before the detector, and only replay saw it. When a live path and a replay path share a detector, test both with the same lines.
- Write test sequences from a real captured stream, not from the protocol as imagined. A real stream has a `task_notification` for nearly every Bash call.
- A new kind of row that can arrive at the moment of a send must be checked against the enter-fade exemption. The flash on send was fixed once for user messages, and any row that lands with the send can bring it back.

## Also in this change: the status bar's token counts

The operator asked for the token counts at the right of the status bar to look like the stats on the left. They didn't. The counts were set in the monospace font at 10px, at 70% opacity, with arrows at 55%. The left section's readouts (`.stat-mono`, `.stat-disk-arrow`, `.system-stats` in `StatusBar.scss`) use the sans font at 11px with -0.02em letter spacing, 60% opacity and arrows at 50%. `.stat-mono` is deliberately sans; see `retro-statusbar-metrics-font-change-2026-09-16`.

`frontend/app/statusbar/_token-usage.scss` now sets the counts the same way: `var(--font-sans)`, 11px, -0.02em, arrows at 0.5, and the indicator at 0.6 opacity at rest (hover still 1, the zero state still 0.35). The breakdown popover is unchanged.
