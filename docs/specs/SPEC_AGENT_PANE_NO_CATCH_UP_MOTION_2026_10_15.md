# Spec: an agent pane shows its latest content still, on a tab switch and on a send

**Date:** 2026-10-15
**Status:** active. Shipped: §4.1, rows on screen stay put when rows above are removed or inserted (#4962); §3.1 and §3.2, a pane shown again lands at its latest content (PR_PENDING). Left: §3.3-§3.5, §4.2, §4.3.
**Scope:** the agent pane's transcript (`frontend/app/view/agent/`): the one-way scroll flow, the virtual list, the live-feed roll-off.
**Related:** `SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md` (the follower, spacer room, commit points), `SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md` (stable row ids, #4952).

Two reports from the operator, both "the transcript moves when nothing new is being read":

1. **Switching to a window tab** whose agent pane got output while the tab was hidden: on show, the pane visibly scrolls through what arrived ("a fast scroll of content") instead of appearing at the latest content.
2. **Sending a message** in an agent pane not typed in for a while: on Enter, the whole conversation history flickers for a moment.

The wanted behavior: a pane that is shown, or that the user just sent from, lands at the latest content **without motion**. If the agent is working, new output after that scrolls in as it does today.

Everything below comes from reading the code at `main` @ `53682b5bc`. Nothing was measured yet; §5 says how to.

## 1. What happens on a tab switch

**The pane keeps its data but freezes its rendering while hidden.** A hidden window tab stays mounted and laid out (`window:keepinactivetabslaidout`, default; `workspace/window-tab-visibility.ts:98-110`: `visibility:hidden; opacity:0`, real `clientHeight`). The agent pane is "dormant" while its tab isn't shown (`agent-view.tsx:155-156`, `agent-dormancy.tsx:53`):
- `MarkdownBlock` stops re-rendering (`components/MarkdownBlock.tsx:94-117`): a block that grew while hidden stays at its old text.
- `ToolBlock` hides tool bodies and logs keep their old view (`ToolBlock.tsx:266-267`, `ToolOverlayLog.tsx:155-162`).
- The stream keeps flushing into the document; nothing is held back.

**On show, everything frozen commits at once**, in stages:
1. every frozen markdown block commits its current text in one reactive pass (`MarkdownBlock.tsx:103-109`);
2. about 90 ms later each one renders again, settled, with highlighting (`:112-116`), changing heights a second time;
3. a tool log that changed branch while hidden runs a 150 ms height transition (`ToolOverlayLog.tsx:368-369`, `resize-contract.ts:153-167`).

**Each stage moves the bottom, and the follower eases to it.** On the one-way path (default, `AgentDocumentView.tsx:209`), `OneWayFlow.onResize` corrects rows that moved on screen at once, then hands the gap to the bottom to `follower.update` (`scroll/one-way-flow.ts:296`). The follower lands at once only when the gap is more than 75% of the viewport (`scroll/follower.ts:87-91`, `SNAP_FRACTION`); anything smaller eases over ~140 ms (`FOLLOW_TAU_MS = 60`, `:21`, `:44`). So the first frame after the switch shows the pane short of the bottom, then it glides down, and stages 2 and 3 start further glides. That is the "fast scroll".

The pane's scroll code gets no signal that it was shown: `AgentDocumentView` and the virtual list don't read dormancy, and in the default mode the viewport never goes to 0 px, so the follow controller's `SUSPENDED`/`BecameSized` path (`scroll/follow-controller.ts:209-218`) and `OneWayFlow`'s `ch <= 0` branch (`one-way-flow.ts:237-242`) never fire.

The legacy path (`agent:onewayflow` false) has the same shape: `glideContent` animates jumps up to 75% of the viewport over 180 ms (`virtualization/AgentDocumentVirtualList.tsx:489-505`).

## 2. What happens on a send after idle

The send itself changes no transcript rows: `TurnStart` and `PendingMessageQueued` touch pane state only, and no row reads turn phase. What moves the transcript comes after, ranked by the evidence:

**2.1 The roll-off removes rows above the viewport while the list windows with a stale scroll position (most likely).** Each send schedules a live-feed roll-off (`hooks/useLiveFeedRollOff.ts:185-197`, run within ~1 s). `planRollOff` exempts the last turn from the 1 MB feed budget (`live-feed.ts:245-259`). At a turn end that is the turn just finished; at the send it becomes the lone new message, so the previous turn counts for the first time and the pass removes old turns of about its size. The document's head shrinks (`store/agent-document/reducer.ts:856-889`), the layout store prunes their heights but keeps `scrollTop` (`store/agent-pane-layout/reducer.ts:138-177`), and `windowRangeOf` (`:342-387`) windows the shifted positions with the old `scrollTop`: it selects rows about the removed height further down, or none. `<Key>` unmounts the rows on screen (`util/keyed.ts:87-96`). The browser clamps the real `scrollTop`, and the store learns it only on the next scroll event → frame → `Scrolled`. For one or two painted frames the visible history is missing, then it all remounts: a flicker of the whole visible transcript. Overscan is 5 rows (`store/agent-pane-layout/types.ts:98`), so any real removal shows. The first roll-off also inserts the history-link row at the top (`inject-history-link.ts:23-31`), a second shift.

**Why idle matters:** after a long, unattended turn, the previous turn is large. Its rows exceed the tail's hold-back (12 nodes / 256 KB, `virtualization/streaming-buffer.ts:126-127`), so the visible history lives in the virtualized head, which is what the stale window drops; and the removed height is large. After a short exchange the visible rows stay in the always-mounted tail and overscan hides a small removal.

**2.2 Spacer room is released in one write.** While following, rows that shrink at the bottom (the Activity Dock's done tools expire 8-15 s after they finish, `activity/types.ts:78-83`) leave blank spacer room instead of moving content (`one-way-flow.ts:283-288`). The send's commit drops it at once (`OneWayFlow.commit`, `:192-198`), and the whole transcript jumps down by that height. Only after idle has the dock had time to expire rows.

**2.3 The turn tail lost its first row when the sent message was re-keyed.** Introduced and fixed in #4952 (`locateFrontierRow`, `virtualization/streaming-buffer.ts`): the re-key that gives a sent message its transcript id made the tail's frontier lookup fail, and the tail re-targeted from scratch; with an older row still in progress, every row from there down remounted and faded in. Listed so it isn't re-investigated.

**2.4 Rows on screen remounted when the tail's frontier moves past its hold-back** (`AgentDocumentVirtualList.tsx:608-611`): held-open tool previews re-highlight asynchronously and paint plain first when the shared highlight cache was evicted meanwhile (`components/PreviewLines.tsx:47-57`, `preview-text/highlight.ts:26-55`), and heights handed from tail to head match by node object, so rows whose object changed (tool results unloaded in the send's roll-off pass) fall back to estimates (`AgentDocumentVirtualList.tsx:597`, `:832`).

## 3. Design: showing a tab

**3.1 Know when the pane is shown.** The virtual list reads dormancy (`useAgentDormant()`; the provider already wraps it, `agent-view.tsx:1219`) and reacts to dormant → active. The reaction only applies while the pane follows (`viewState.stickToBottom()`): a pane the user scrolled up keeps its place, which on-screen compensation already does.

**3.2 Land, don't ease, while the frozen content commits.** On show, `OneWayFlow` lands at the bottom and opens a settle window (`settleOnShow()`, the commit window's mechanism, `one-way-flow.ts:256-270`; spacer room is given up too, since nothing was painted while the pane was hidden): until it closes, every resize writes the true bottom before paint. Long enough to cover the staged commits (the 90 ms settled render, the 150 ms tool-log transition): 400 ms. The first frame after the switch is then already at the bottom, and so is every frame while the content settles.

After the window, output that arrives is new: the follower eases it in as today. That is "if the agent is busy, it scrolls normally as it processes", with no special case for busy.

**3.3 Fewer stages to settle.**
- `MarkdownBlock`: on dormant → active, commit the settled render (`streaming: false`) directly, instead of the leading commit plus the trailing re-render 90 ms later.
- `ToolOverlayLog`: skip the height-continuity transition (`beginHeightContinuity`) for a branch change caused by the pane being shown (track the previous dormancy).

**3.4 No fades for rows mounted while hidden.** Rows mounted while the tab was under `content-visibility: hidden` (the legacy setting, or during a window resize) would fade in together on show (`styles/_document.scss:287-325`). Clear `data-animate` while dormant and set it again a frame after show.

**3.5 Legacy path.** Skip `glideContent` and `holdBufferHeight` during the same window (`AgentDocumentVirtualList.tsx:1106-1108`).

## 4. Design: sending after idle

**4.1 Keep the window on the same rows when rows above it are removed.** In the layout reducer's `NodesChanged` (`store/agent-pane-layout/reducer.ts:138-177`), subtract from `scrollTop` the heights of removed rows that sat above the old window start (they're known: they're being pruned), so `windowRangeOf` selects the rows that are on screen. Same for a prepend (the history-link row): add the inserted height. The DOM `scrollTop` is clamped by the browser; the store now agrees with it in the same pass instead of a frame later. This also fixes any other head removal (a manual `/clear` is a full reset and unaffected).

**4.2 Drain spacer room, don't drop it.** At a commit, keep the spacer and let the rows that arrive consume it (the new message, the agent's output): the spacer shrinks by what each resize adds at the bottom, and the content never moves down. Drop what's left when the commit window closes, at the bottom and with nothing after it: if any remains it means the commit added less than the room, and the drop is then smaller than a row.

**4.3 Repaint nothing that didn't change.**
- `PreviewLines`: on mount, read the highlight cache synchronously and paint highlighted at once when it hits; only a miss highlights async.
- Height hand-off from tail to head: match by node id and a content key (for a tool: status, result presence, unloaded flag), not object identity, so an unloaded result keeps its measured height.

## 5. Verification

The motion is a visual property; unit tests pin the mechanisms, the lab measures the result.

**Unit tests:**
- `OneWayFlow.settleOnShow`: a resize inside the window writes the bottom with no follower step; after it, a small gap eases as today.
- the virtual list calls it on dormant → active only while following;
- `MarkdownBlock` renders once (settled) on show;
- layout reducer: removing rows above the window keeps the window on the same row ids (`windowRangeOf` before and after), and a prepend likewise;
- spacer drain: a commit followed by bottom growth consumes the spacer with no change in on-screen row tops.

**Lab** (isolated portable; the agent driven by the fake-Claude script, as the earlier flash and memory labs):
- *Tab switch:* a pane in a hidden tab receives 3 turns of mixed text and tool output; switch to it; sample the scroller's `scrollTop` and `scrollHeight` per animation frame for 600 ms (in-page probe over CDP). Pass: from the first painted frame, `scrollTop` equals `scrollHeight - clientHeight` within 1 px on every frame.
- *Send after idle:* one long turn (over the tail hold-back), wait 60 s, send; sample per frame the number of mounted transcript rows and the top row id. Pass: no frame where the row count drops by more than one and recovers, the top row id changes only by the roll-off's removal, and no frame with on-screen rows moving.
- Both with the agent still streaming after the event: new output eases in as before.

## 6. Order

1. §4.1 (the likeliest flicker), with its reducer tests.
2. §3.1 + §3.2 (the tab-switch motion), with the lab probe.
3. §3.3, §4.2, §4.3, §3.4, §3.5 as the lab shows what's left.
