# Report: the agent pane's motion fixes, measured in packaged builds

**Date:** 2026-10-15
**Status:** analysis
**Author:** Loap-3@narko, at the operator's request ("run the lab … take it to the end, write a report")
**Related:** `SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md` (the fast scroll on a tab switch, the flicker on send), `SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md` (scroll restore across a reload), PRs #4962, #4963, #4964 (the fixes measured), #4974 (the fixes this measurement led to), the cover on a show (motion spec §3.7, measured in §3.4 below, in the PR that ships this report), #4966 (a build break found on the way).

## 1. Summary

| Report | Fix | Measured | Now |
|---|---|---|---|
| Switching to a tab whose agent pane got output while hidden shows a fast scroll | #4963 (land at the bottom while the hidden content commits) | did not act in a packaged build: drawing before the first frame (~330 ms) used up its 400 ms window | **#4974**: every painted frame at the bottom (0 frames short of it, against 3 frames up to 584 px), but the bulk of the motion remained. **The cover (§3.7)**: no text moves after the show in 3 of 3 runs, covered 24–40 ms; see §3.4 |
| The conversation flickers on the first send after a long turn | #4962 (rows kept on screen when old turns roll off above them) | not reproduced on either build in the lab's scenario | unconfirmed; §4.3 |
| (Phase A) a reload should put each pane back where it was | #4964 | the right row, but 458 px off; in another run a later row | **#4974**: same row, 1 px off, in two runs (without the fix: back at the bottom) |

The most important finding: **the "fast scroll" is the content itself arriving in stages after the show**, about 5,400 px over three frames, and a pane pinned to the bottom shows that as text jumping up. No scroll rule removes it. The fix is to keep the pane covered until its layout has settled (motion spec §3.7). Measured against `main` built from the same commit, it appears once, at the bottom, with its final content, and nothing moves after that (§3.4).

## 2. Method

**Builds.** Packaged portables (`scripts/package.sh`) from the same `main` (`d4c0d22c4` plus #4966's compile fix), one with #4962, #4963 and #4964 reverted ("base"), one as-is ("fix"), and after the first round one with the follow-ups ("fix2", "fix3", now #4974). So each pair differs only by the fixes under test. For the cover (§3.4), both builds are from `main` at `251a02bbe` (which has #4974), one with the cover ("cover") and one without ("main"), run alternately.

**Lab.** Each run starts a fresh isolated instance (its own home, CDP on 9333), installs the fake Claude CLI playing a recorded long conversation (turns of 100–200 KB, 16–28 rows each), creates one agent, and plays turns. Three scenarios, driven over CDP:

- **send:** 8 turns of history (more than the 1 MB live-feed budget), 25 s idle, then one message.
- **tab:** a new window tab hides the pane, the pane gets 3 turns, then its tab is clicked.
- **reload:** the pane is scrolled up with real wheel input, then the window reloads through the app's own path (`AppApi.reloadWindow`; a page marker proves the reload).

**Measures.**
- A per-frame sampler of the pane's scroller: scroll position, distance from the bottom, mounted rows, the row at the top of the screen. Sampled **after paint** (a task posted from `requestAnimationFrame`): a read inside `requestAnimationFrame` comes before that frame's ResizeObserver callbacks, where the scroll is corrected, and showed gaps that were never painted (the first round's tab numbers were wrong for this reason).
- The app's own one-way recorder (`__agentmuxOneWay`), which checks every painted frame for text moving down while following and for blank space shown and then taken back.

## 3. Results

### 3.1 Tab switch

Sampled after paint, first 60 frames after the click:

| Build | Painted frames short of the bottom | Largest gap | Scroll motion |
|---|---:|---:|---:|
| base | 3 | 584 px | 5,289 px |
| fix2 (#4974) | **0** | 1 px | 5,449 px |

The recorder found no downward text motion and no blank space in either build.

Earlier rounds (sampled before paint, so overstating the gap) showed the first painted frame ~3,100–4,700 px short of the bottom in both "base" and "fix", and #4963's settle window not acting: the frame trace shows ~330 ms between the click and the first painted frame, longer than most of the 400 ms window started at the show. #4974 arms the window again on the first painted frame.

What remains in both builds: while the tab was hidden the pane's scroll height stayed at the old content, and after the click the new content landed in stages (+4,735 px, then +2,240 px, then ~+1,000 px over three frames). Pinned to the bottom, each stage moves the text up by that much.

### 3.2 Reload

| Build | Before (top row, offset into it) | After | Same row | Offset error |
|---|---|---|---|---:|
| base | `L3930`, 1,413 px | `L4772`, at the bottom | no | (at the bottom) |
| fix (#4964) | `L3930`, 152 px | `L3930`, 610 px | yes | 458 px |
| fix2 (hold) | `L3930`, 1,411 px | `L4370`, 164 px | no | (a later row) |
| fix3 (#4974), run 1 | `L692`, 1,250 px | `L692`, 1,251 px | yes | **1 px** |
| fix3 (#4974), run 2 | `L692`, 1,250 px | `L692`, 1,251 px | yes | **1 px** |
| base, same scenario | `L692`, 1,250 px | `L1454`, at the bottom | no | (at the bottom) |

Two causes, both fixed in #4974. The rows above the anchor are measured over the frames after the restore, and a pane that doesn't follow keeps nothing in place; fix: the position is held for up to a second, and input in the pane ends that. Separately, anchoring on the row cut by the top edge breaks when that row comes back a different height; fix: anchor on the first row that starts on screen.

### 3.3 Send after idle

| Build | Rows before | Fewest rows after | Frames with rows halved | Frames with nothing on screen | Recorder: text moving down |
|---|---:|---:|---:|---:|---|
| base | 43–45 | 43–44 | 0 | 0 | 0 frames |
| fix / fix2 | 42–43 | 42–43 | 0 | 0 | 1 frame, 0.8 px |

The reported flicker did not reproduce on either build. In one base run the content slid down ~400 px over 120 ms right after the send (the scroll height shrank while the pane stayed at the bottom; consistent with the spacer room the motion spec §4.2 describes), but the recorder didn't flag it in the second round.

### 3.4 Tab switch with the cover

The tab scenario, three runs per build, alternating. "Text moved" measures how far the text on screen moves between painted frames while the pane is visible (a row's position on screen, not the scroll position, which also changes when content is added below and nothing visible moves). Covered frames don't count as shown.

| Build, run | Frames shown | Frames where text moved | Text moved | Largest move | Frames short of the bottom | Covered |
|---|---:|---:|---:|---:|---:|---|
| cover, 1 | 57 | 0 | 0 px | 0 px | 0 | 3 frames, 32 ms |
| cover, 2 | 60 | 0 | 0 px | 0 px | 0 | 3 frames, 40 ms |
| cover, 3 | 60 | 0 | 0 px | 0 px | 0 | 3 frames, 24 ms |
| main, 1 | 52 | 38 | 425 px | 106 px | 37 (up to 123 px) | — |
| main, 2 | 45 | 0 | 0 px | 0 px | 0 | — |
| main, 3 | 60 | 0 | 0 px | 0 px | 0 | — |

With the cover, the first frame shown already has the final layout: its scroll range equals the one at the end of the run, and the scroll position doesn't change afterwards. On `main` the scroll position moved 5,000–6,300 px after the show in every run, and the first frame shown never had the final scroll range. Even so, in two of three runs no text visible on screen moved (why is not established); in the third, the text moved on 38 frames. So `main` doesn't show the jump in every run, and the cover removed it in all three. The cover was up for 24–40 ms, well under its 350 ms cap. The one-way recorder flagged nothing in any run.

## 4. Findings and next steps

1. **The tab-switch motion needed a cover, not a scroll rule** (motion spec §3.7, done). The pane's transcript stays hidden from the show until its layout has settled (two frames with no resize, at most 350 ms), then shows at the bottom, once, with its latest content (§3.4).
2. **The send flicker needs a closer reproduction** before more work on it: the lab sends through the RPC, not the composer, and the pane had no expired Activity Dock rows. Next: send from the composer after a turn with many tool calls and a longer idle, with the recorder on.
3. **#4974** (merged) has the two fixes from the first rounds (settle from the first paint; restore anchored and held).
4. **Why the content lands on show:** a hidden tab keeps its layout (`window:keepinactivetabslaidout`), but the pane's rendering pauses while hidden (`MarkdownBlock`, `ToolBlock`, `agent-dormancy.tsx`) so that hidden panes don't take the main thread from the pane being typed into, and everything held back is drawn on show. That pause stays; the cover hides its catch-up (motion spec §3.7).

## 5. Found on the way

- **`main` didn't compile `agentmux-cef` on Windows or Linux** after #4959 (a macOS cursor fix typed CEF's cursor handle as macOS's `*mut u8`; Windows uses `HCURSOR`, Linux `c_ulong`). PR CI doesn't build that crate on Windows or Linux, so it merged; the lab's packaging hit it. Fixed in **#4966** (merged), and Masty, the author, confirmed.
- **Coordination channel** (separate request, **#4969**, merged): `PostToChannel`, `ReadChannel`, and `recent_posts` in `WhoIsWorkingOn`, per repository, for every agent on this computer.

## 6. Reproducing

The lab scripts are in the author's workspace (`memlab/`), not the repository: `nm-ab.sh <label> <portable>` (a fresh instance, history, the three scenarios; `ONLY=tab|reload`, `TURNS`, `HIDDEN_TURNS`), `nm-probe.mjs <label> <send|tab|reload>` (the per-frame sampler, the recorder, the reload check; results in `memlab/results/`), on top of `lab.mjs` (CDP driving) and `launch.sh`.
