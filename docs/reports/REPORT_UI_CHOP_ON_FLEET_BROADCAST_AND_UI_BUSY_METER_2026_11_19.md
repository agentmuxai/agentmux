# Report: the UI stutters when one broadcast starts a turn on many agents, and what "UI n%" measures

**Date:** 2026-11-19
**Status:** analysis. Read-only, from the logs of one event; no lab was run.
**Task:** #48174. Related: #47820 (long-running boundaries), #47543 (release-build page diagnostic).

## Summary

1. **"UI 44%" is not CPU and does not mean the page had room to spare.** It is the share of the last second
   this window's page spent inside single tasks of 50 ms or more. Work in shorter pieces is not counted, so it
   under-reads: for the five seconds after the broadcast the page was inside slow frames at least 70% of the
   time and drew about four to five frames a second (each 130 to 275 ms). The page has one thread and it was
   full; the machine, at the samples either side of the event, had most of its 32 cores free (5 and 10 busy).
2. **What filled those frames, from the log:** 16 turn starts across this window's 14 panes within four seconds, each pane's
   first redraw of the turn taking 42 to 75 ms; messages from srv handled one at a time (12 to 40 ms each in
   the burst, then about 85 a second for minutes, costing 10 to 18 s of every minute); the Swarm list rebuilt
   161 times in that minute; and 55 to 157 ms of rendering per frame on top. srv itself shows no warning and
   no slow line.
3. **Fixes, in order:** make the meter count slow frames so a stutter reads as one; handle the stream's
   messages in one batch per frame and do nothing for panes that are not on screen; rebuild the Swarm list at
   most once a frame; spread a broadcast's deliveries over a second or two. The log cannot say whether the
   host was also slow (calls to it took 1.3 to 1.7 s in the burst): timing those calls on the host side is the
   first thing to add.

## 1. What the figure is

`frontend/app/statusbar/ui-busy.ts`, reading `frontend/app/diag/main-thread-blocked.ts`:

- **Whose:** this window's page (its one main thread). Not srv, not an agent, not the host process, not
  another window, not the GPU or the compositor.
- **Unit:** milliseconds of Long Tasks entries (the browser reports a task only when it ran 50 ms or more)
  that fall in the window, over the window's length. Not a share of a core and not of all cores.
- **Window:** from the previous reading to this one, taken once a second by a timer. When the page is busy
  the timer runs late and the window is simply longer; nothing is lost.
- **What it cannot see:**
  - A run of tasks of 10 to 49 ms each. Forty such tasks fill a second completely and the figure reads 0%.
  - The rendering step of a frame when it is not part of a long task.
  - Time the page waits for someone else (a call to the host, the compositor, the GPU).

So a figure under half beside a visible stutter does not mean the page had spare time: **the figure averages
the stall away**, twice over. It counts only the 50 ms-and-over pieces of a frame's work, and it is a one-second
average, so three frozen frames of 150 ms read as "45%".

A frame is what a person sees, and a frame is made of several tasks. The log's `long-frame` lines (the Long
Animation Frames API) measure frames, and they tell the real story (section 2).

## 2. What the logs show

Times are relative to the broadcast (T). The window's page had 14 agent panes and about 43,000 DOM nodes.
All figures are from the installed build's host log, srv log and long-run record.

### 2.1 The page

Long tasks, which are what "UI n%" counts, by second of the page's clock:

| When | Long tasks |
|---|---|
| T+0 to T+1 s | 293 ms in 2 tasks |
| T+1 to T+2 s | 516 ms in 5 tasks (worst 165 ms) |
| T+2 to T+3 s | 385 ms in 3 tasks (worst 180 ms) |
| T+3 to T+4 s | 414 ms in 4 tasks |
| T+4 to T+5 s | 334 ms in 4 tasks (worst 174 ms) |
| T+5 to T+8 s | 55 to 119 ms a second |
| after T+10 s | a task of 50 to 100 ms every few seconds |

The meter's own one-second windows do not line up with these, which is how it showed 44% where the worst
aligned second holds 516 ms.

Slow frames (50 ms or more), by the batch in which the page's lines reached the log:

| Span | Slow frames | Time inside the logged ones |
|---|---|---|
| T+1.3 to T+3.1 s | 7 (one not logged) | 1.27 s of 1.84 s: 69% or more |
| T+3.1 to T+4.6 s | 6 (one not logged) | 0.96 s of 1.43 s: 68% or more |
| T+4.6 to T+6.0 s | 11 (four not logged) | 1.07 s of 1.41 s: 76% or more |
| T+6 to T+9 s | frames of 100 to 140 ms continue | |
| the next three minutes | 6 to 31 slow frames per 10 s | |

What the slow frames held (each line names the scripts that ran in it):

| Part of a frame | Cost | What it is |
|---|---|---|
| The stream scheduler's frame callback | 40 to 75 ms in the worst frames; 1.9 s in all | A pane's first redraw of a turn: `slow-flush` lines of 42, 65 and 75 ms for three panes in one batch. The scheduler's budget is 8 ms a frame but it always runs one pane, and one pane is 5 to 9 times the budget here |
| The websocket message handler | 12 to 40 ms per message, several per frame; 5.2 s in all | Each message from srv is handled in its own task as it arrives |
| The rendering step | 55 to 157 ms per frame in the burst | Not script: the browser laying out and painting what the above changed |
| The Swarm list's timer | 18 to 29 ms of forced layout each time | The Swarm view was open: 161 rebuilds in the minute (66 "other", 50 progress, 29 status) |
| Other frame callbacks | 20 to 33 ms of forced layout each | Three more callbacks that read layout after writing |

**The sustained part.** The page logs what its message handling cost per minute:

| Minute after the broadcast | Messages | Handler time | Of which transcript appends (`blockfile`) |
|---|---|---|---|
| first | 6,226 | 10.2 s | 3,633 events, 7.7 s, 2.1 ms each |
| second | 8,332 | 15.0 s | 5,246 events, 13.7 s, 2.6 ms each |
| third | 8,100 | 18.3 s | 5,039 events, 16.5 s, 3.3 ms each |

So after the burst, 17 to 30% of the page's thread goes to handling messages one at a time, nearly all of it
transcript appends, before any redraw. That is why typing stayed slightly late for minutes (key presses presented 48 to 72 ms late; 61
more such lines were not logged). An object update (`waveobj:update`) costs 16 to 20 ms each: 64 of them in
the first minute, one or two per turn start.

### 2.2 Calls from the page to the host

In the burst, seven `set_taskbar_attention` calls took 1.39 to 1.70 s each, two
`browser_panes_set_overlay_clip` calls 1.28 and 1.52 s, and one `input_select` 0.99 s. They normally take
under 50 ms. For `input_select` the host logged the request the moment it was sent, so that second was spent
after the host had it: either the host answering late or the answer waiting in the page's queue. **The log
cannot tell which**, because the host does not log how long it took to answer.

### 2.3 srv

88 log lines in the second the broadcast was delivered, 18 turns opened and 23 turn-state changes within
12 s, 31 transcript line counts read. No warning, no error, no slow line. srv is not where the time went.

### 2.4 The machine

The long-run record samples every five minutes, and the event fell between two samples:

| | 17 s before | 4 min 43 s after |
|---|---|---|
| Machine CPU (cores busy, of 32) | 5.4 | 9.6 |
| AgentMux's processes | 163 | 261 |
| AgentMux's memory | 5.0 GB | 7.2 GB |
| The window's page heap | 407 MB | 766 MB |

About a hundred processes started. At this resolution the machine was never short of CPU, which fits a page
limited by its one thread, but nothing recorded says what the machine's CPU was in the burst's own seconds.

### 2.5 Seen on the way

47 s before the broadcast, a click into a pane's composer right after the window came to the front took 528 ms
to show (two tasks of about 300 ms with no script running in them, and an `input_select` call of 689 ms).
The page was waiting there, not computing. It is a separate matter from the broadcast, in the focus
hand-off between the window and the page.

## 3. Cause

One Swarm broadcast is delivered to every recipient in the same second. Each recipient with a pane in the
window starts a turn at once, and the page does each pane's turn-start work, each message and each Swarm row
update as its own piece of work the moment it arrives, on one thread. None of the pieces is individually
long, so the "UI busy" figure, which counts only pieces of 50 ms and over, reads under half while the frame
rate is four or five a second.

The burst is the visible part. The cost that stays is the per-message handling of transcript appends while
many agents stream at once.

## 4. Fixes

| # | Fix | Why this one | Size |
|---|---|---|---|
| 1 | **The meter counts slow frames.** "UI busy" becomes the share of the last second spent in frames of 50 ms or more (Long Animation Frames, with Long Tasks as the fallback), and its tip says what it measures. Done in the change that adds this report; section 1 describes the figure as it was during the event. | It would have read above 70% here. The figure exists to answer "is the page the slow part", and today it says "no" when the answer is "yes" | Small, one PR |
| 2 | **Stream messages are applied once per frame.** The message handler only queues a transcript append; the stream scheduler applies a pane's queued appends in its flush, under the budget it already has. A pane not on screen applies nothing until it is shown. | 7.7 to 16.5 s a minute is the largest single cost, and it is paid for panes nobody is looking at | Medium |
| 3 | **A turn's first redraw fits the frame budget.** Find what the first flush of a turn does in 42 to 75 ms (it is 5 to 9 times the scheduler's 8 ms budget) and split or defer it; never run more than one of them in a frame. | It is the peak of the burst | Needs a profile first (see 6) |
| 4 | **The Swarm list rebuilds at most once a frame** and does not read layout in its timer. | 161 rebuilds a minute, each forcing a layout of 18 to 29 ms, at exactly the time the page is busiest, and the Swarm is where a broadcast is sent from | Small |
| 5 | **A broadcast's deliveries are spread out**: a few per 250 ms instead of all in one tick, so 31 agents start over about two seconds. | It flattens the burst at the source and also the hundred process starts. The last agent gets the message two seconds later | Small, in srv; a behaviour decision |
| 6 | **Object updates at turn start are batched** (`waveobj:update`, 16 to 20 ms each). | 1.2 s in the first minute | Small to medium |

Fix 1 changes what a person sees first and costs least. Fixes 2 and 4 remove most of the measured cost.
Fix 5 is the cheapest way to stop the burst but leaves the sustained cost.

## 5. What is missing from the logs, and what the two related tasks would have shown

- **The host's side of a call.** A line when the host takes more than 100 ms to answer a call from the page
  (received, answered) would have split section 2.2 into "the host was stalled" or "the page was". Add this
  first: without it the next event of this kind leaves the same question open.
- **The machine's CPU in the seconds of a storm.** The record's five-minute samples missed the event. Keeping
  the last minute of one-second machine and page-process CPU in memory, and writing it to the log when a run
  of slow frames is seen, would have shown whether the hundred process starts took the cores.
- **#47543 (per-page diagnostic on a release build)** would have shown the window's page process near 100% of
  one core while the machine was at a third: the direct proof of "one thread saturated", which this report
  infers from frame lines.
- **#47820 (boundaries)**: a boundary on message-handling time per minute would have fired on all three
  minutes above (10, 15 and 18 s, against a few seconds in ordinary use), and one on slow frames per minute on
  the first. Both figures are already logged; the record does not hold them. Writing them into the record's
  five-minute line makes a slow decline over weeks visible in the same place.

## 6. A lab that would settle fix 3 (not run; needs its own approval)

Question: what does a pane's first flush of a turn spend 42 to 75 ms on? One run in an isolated dev build
with scripted agents: 14 panes, one broadcast, a performance profile of the ten seconds after. About fifteen
minutes, no real mouse or keyboard, nothing on an instance in use. The logs name the callback but not what
inside it is slow.
