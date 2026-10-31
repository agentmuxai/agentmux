# How the agent pane is built

**Status:** living (written 2026-10-31 against `origin/main`; update it with the pane's structure)

The agent pane is `frontend/app/view/agent/` (the view) plus `frontend/app/store/agent-pane-state/` (its state). This is the map for someone about to change it: where each job lives, which jobs have exactly one owner and what enforces that, and which files were split along which seams. It describes `origin/main` as of the S7 step of the cleanup plan (`docs/reports/REPORT_AGENT_PANE_CLEANUP_AUDIT_2026_10_30.md`, task #13044); the report has the history and the bug list.

## The rule behind the layout

Where one job had several owners, the pane had intermittent bugs: scroll jumping, a stale failure row coming back, a status that disagreed with another. The cleanup gave each such job **one owner and a check that fails the build when a second one appears**. Read the three below before adding a write to any of them.

## 1. State: one reducer

The pane's turn, token, context, pending-message, failure, activity and attached-task state each have one applier, the reducer in `store/agent-pane-state/reducer.ts` (types in `types.ts`, helpers beside it). Components and hooks **dispatch commands** to it and read the result; they do not keep a second copy of reducer state in a signal. There are 40 or so dispatch sites; that is fine, since a producer is not an owner. Where two producers can disagree, the reducer holds the rule (the failure row, below).

`stream-parser.ts` turns the backend's stream into the commands. Live output and history share the one parser. How a message's *origin* (the user, a jekt, a notification) is decided is still partly guessed from text there; typed origin is a separate step of the plan and is not described here.

## 2. Scroll: one writer of the transcript's `scrollTop`

Nothing under `view/agent/` writes the transcript scroller's position except `scroll/one-way-flow.ts`. It owns the follower (stay at the bottom while the content grows, never leave blank room below the latest content) and exports `scrollTranscriptTo` and `scrollTranscriptBy` for every caller that moves the transcript for its own reason: jump to a node, the older-history restore, keeping the rows on screen across a slice change (`virtualization/keep-on-screen.ts`), the zoom anchor (`virtualization/zoom-anchor-adapter.ts`), the hand-off.

- **Enforced by** `scripts/check-transcript-scroll-writers.mjs` (CI). It fails on `.scrollTop =`, `.scrollTo(`, `.scrollBy(` or `.scrollIntoView(` in any other file under `view/agent/`. Its `EXEMPT` list names the files that scroll something that is not the transcript (a tool's preview box, the History tab's list, the slash-command picker's list), with the reason for each.
- **The follow mechanism** is the controller in `scroll/` (`follow-controller.ts`, `follow-wiring.ts`, `follow-scroll.ts`) on the one-way-flow write path. The old pin/glide/shrink path, its `agent:onewayflow` setting and `legacyPreviewFollow` are gone, and dev and release run the same code.
- The virtualised list is `virtualization/AgentDocumentVirtualList.tsx`; its helpers (`anchor.ts`, `scroll-memo.ts`, `show-settle.ts`, `live-arrival.ts` and others) sit beside it.

## 3. Failure: one module raises and clears the failure row

Every `FailureObserved` and `FailureCleared` goes through `failure/failure-writer.ts`: `raiseFailure(model, { source, ... })` and `clearFailure(model, { reason, ... })`. A call says **who** it is (a `FailureSource`) or **why** it clears.

- **Precedence is written down**, in `store/agent-pane-state/failure-precedence.ts` and applied by the reducer: `live` (the backend's classified `agentfailure` event, always applies) outranks `command` (a command's own outcome), then `synthetic` (the pre-launch "Not signed in" row), `seed` (the failure stored in block meta, put up at mount, which yields to anything live) and `offline`. A source applies over a failure of the same or lower rank. A late `seed` is also dropped when a turn began or finished since the pane mounted, so it cannot end a working turn or replace a newer failure.
- **Enforced by** `scripts/check-failure-writers.mjs` (CI): a direct `type: "FailureObserved"` or `"FailureCleared"` anywhere in `frontend/` other than the writer, the pane-state store and tests fails.
- The row itself is `failure/useFailureRow.ts`; the other files in `failure/` handle the account picker, re-binding an account, refused sends and their restore, and the sign-in recheck. The spec for the row following live state is `docs/specs/SPEC_AGENT_PANE_FAILURE_ROW_FOLLOWS_LIVE_STATE_2026_10_29.md`.

## 4. The big files, and how they were split

Each split followed a seam (a lifecycle, a layout calculation, a self-contained component) so a fix lands in a named file instead of the one everyone touches. Line counts are from the time of writing and are held down by `scripts/check-file-sizes.mjs`, which lets a file over 1500 lines only shrink.

| File | Lines | What moved out |
|---|---|---|
| `agent-view.tsx` | 762 | `hooks/usePaneRegistration.ts`, `hooks/useShellDrawerLifecycle.ts`, `hooks/useTurnEndSweep.ts`, `failure/useFailureRow.ts`, `startup/startupOnReady.ts`, `pane-meta.ts` (block-meta reads) |
| `components/AgentComposerStrip.tsx` | 941 | `components/composer-row-layout.ts` (balance and rows), `components/context-bandwidth.ts`, `components/composer-stats-zone.tsx` |
| `components/AgentFooter.tsx` | 1027 | `components/working-row.tsx` (`AgentWorkingRow`), `components/status-formatters.ts` |
| `virtualization/AgentDocumentVirtualList.tsx` | 1232 | the old follow path (deleted, not moved) |

Still large: `components/MyAgentsList.tsx` (1398), `stream-parser.ts` (1093) and `components/AgentPicker.tsx` (915). They are cut when a change next needs them.

## 5. Where to put new code

- A new fact about the turn, a failure, a token count or a pending message: a reducer command and field, not a signal. Read it through the pane's model.
- Anything that moves the transcript: call `scrollTranscriptTo` / `scrollTranscriptBy`; if it scrolls a different element, add it to `EXEMPT` in the scroll check with the reason.
- A new way a failure is raised: pick its `FailureSource`; if none fits, add the source and its rank in `failure-precedence.ts` first, with a test.
- A lifecycle (mount, close, switch, bind): a hook under `hooks/`, wired from `agent-view.tsx`, not a block in the view.
- A control (button, tab, field): the kit in `frontend/app/element/ui/` (`Button`, `IconButton`, `Tag`, ...), the same as everywhere else. `scripts/check-ui-primitives.mjs` also holds the pane to the shared scales (z-index, durations, font sizes, colours, focus rings): a value that has no token and is not on that script's allowlist, with a reason, fails the build. Hand-made `<button>`s in the pane are still being converted and are the one rule the pane is exempt from.

## What this article does not cover

Typed message origin and the shared task-status mapping (steps S4) were in progress when it was written. The audit's "who writes what" table (section 3) is the place to check what each producer does today.
