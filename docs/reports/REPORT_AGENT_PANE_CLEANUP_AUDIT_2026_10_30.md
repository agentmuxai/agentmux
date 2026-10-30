# Report: agent pane cleanup audit (duplicate paths, state writers, the plan)

Task #13043 (research), #13044 (build). Written 2026-10-30 by AgentX against `origin/main` 7f87ebba7 (`frontend/app/view/agent`). Ships with the first cleanup PR.

Operator, 2026-10-30: "how is the agent pane looking, as far as code cleanliness and DRYness? i am noticing subtle bugs, some things sometimes dont work ... lets do the audit, clean it up, youll probably fix the bugs while doing that, once it delivers, Ill dogfood anew". Then, on the hand-made buttons: "ok lets fix all of this".

**How to read this.** "Checked" means I read or grepped it myself on 7f87ebba7. "Audit" means a read-only research agent reported it and I did not re-check it; line counts from the audits are approximate. The operator's own list of subtle bugs has been asked for (once) and is not in yet: section 4 holds the bugs found in the code and the fix log, and gets the operator's list added when it arrives.

## 1. The short version

The pane is not a mess everywhere. The turn, token, context and pending-message state is cleanly owned by one reducer (`store/agent-pane-state/reducer.ts`), and live and history share one parser. The intermittent bugs come from **four places where one job has several owners**:

1. **Scroll has three mechanisms and five writers.** Dev and release builds run different ones.
2. **Failure rows have many producers.** At least 8 call sites raise or clear the same failure, with no stated precedence. The P1 muxreview found on Manoz's #5874 (a late seed overwriting a live failure) is this problem showing itself.
3. **Message origin is guessed from text in five places.** Already being fixed by Agent1 (#12499) and Maop (#4473).
4. **Three files carry most of the churn** (`agent-view.tsx`, `useAgentControllerStatus.ts`, `AgentDocumentVirtualList.tsx`), and two others sit at the size limit.

Plus 55 hand-made buttons, to convert inside the steps that touch each file.

## 2. Duplicate paths: two code paths for one job

| Job | Path A | Path B (and C) | Which wins today | Checked |
|---|---|---|---|---|
| Following the bottom of the transcript | `scroll/one-way-flow.ts` (402 lines) | the old pin/glide/shrink code in `virtualization/AgentDocumentVirtualList.tsx`, run when `agent:onewayflow` is `false` | OneWayFlow: `AgentDocumentView.tsx:225` passes `oneWayFlow = setting !== false` | yes |
| Following, controller version | `scroll/follow-controller.ts` + `follow-wiring.ts` + `follow-scroll.ts` (223 + 191 + 236) | the code above | **Depends on the build.** `agent:followcontroller` unset means on in a dev build and off in release (`follow-controller.ts:97-104`) | yes |
| Preview (tool output) following | `createPreviewFollow` with the controller (`preview-follow.ts:58`) | `legacyPreviewFollow` (`preview-follow.ts:151-250`, ~100 lines, "kept as the kill switch until Phase 4") | follows the controller flag, so also differs dev vs release | yes |
| "Is this task finished" mapping | `terminalStatus` in `activity/task-outcomes.ts:42` | an inline ternary in `activity/tool-adapter.ts:200` (and srv's `terminal_status`, per the comment) | identical today (done / error / stopped); three copies | yes |
| Message origin | `JEKT_BLOCK_RE` etc. in `stream-parser.ts:53-110`, `system-notes.ts`, supervisor header, `memory-reinjection`, attached-images | the typed `amx.origin` in the message-origin spec (P1) | text guessing wins today; being replaced | audit |
| Restoring an unsent message | `refused-send.ts` + `composer-restore.ts` live function | `setComposerDraft` fallback in `reload-persistence.ts` | live function if the footer is mounted, else draft | audit |

**Dead code once the old path goes** (audit, approximate): in `AgentDocumentVirtualList.tsx` the glide system (~45 lines), shrink-hold (~60), the second content ResizeObserver (~38), the legacy pin and first-overflow re-pin (~40), the near-bottom threshold and legacy disengage (~35); `legacyPreviewFollow` (~100). About 350 lines, which takes the file from 1797 to roughly 1440, under the 1500 limit.

## 3. Who writes what

**Reducer state: clean (audit).** `turnPhase`, `failure`, `activity`, tokens, `context`, `pending`, `attachedTask` and the rest each have one applier, the reducer. No state is mirrored between a signal and a reducer field. The 40+ dispatch sites are producers, which is fine for most of the state.

**Where it is not clean (checked):**

| State | Writers | Problem |
|---|---|---|
| The transcript's `scrollTop` | `scroll/one-way-flow.ts` (5 sites), `virtualization/AgentDocumentVirtualList.tsx:376, 1167, 1175, 1478, 1487`, `virtualization/keep-on-screen.ts:89`, `virtualization/zoom-anchor-adapter.ts:164`, `history/AgentHistoryView.tsx:725`, `scroll/preview-follow.ts:75, 177` (tool previews) | Five modules move the same scroller. OneWayFlow is not the single writer the bottom-tight spec says it is. Zoom (#11020, Camper-2) adds to this |
| `stickToBottom` | `state.ts:144` and its own setter, `follow-wiring.ts:183` (controller path), five call sites in `AgentDocumentVirtualList.tsx` (1136, 1177, 1270, 1344, 1379) | Two parallel writers, one per scroll mechanism |
| The pane's failure | `FailureObserved` from `useAgentFailure.ts:287, 299, 413`; `useAgentCommands.ts:604, 884, 1040, 1097`; `useAuthHealth.ts:52`; `offline/useOfflineInput.ts:202`; `FailureCleared` from `agent-view.tsx:611`, `useAuthHealth.ts:71, 116`, `useAgentCommands.ts:491, 532`, `useAgentFailure.ts:228`, `useOfflineInput.ts:203` | 8 raise sites and 8 clear sites in 5 files, with the "which wins" rule living in comments (`FailureCleared` clears any failure whatever its code, so callers guard with "only if it is an auth failure"). The reducer applies them in arrival order |

The failure row's mount-time seed in `useAgentFailure.ts` (waits up to `SEED_WAIT_MS`, then dispatches the stored failure) is a ninth producer with no check for what happened while it waited. That is muxreview's P1 on #5874.

## 4. The bugs

**Operator's list: asked once, not yet received.** To be added here, one line each, each mapped to the code behind it.

**Found in the code and the fix log (each is a hypothesis until reproduced):**

| # | Symptom a user would see | Cause in the code | Step |
|---|---|---|---|
| B1 | Scrolling behaves differently in a dev build and an installed one | `agent:followcontroller` defaults on in dev, off in release; the preview follow picks the same flag | S1 |
| B2 | A pane jumps or loses its place after a zoom, a load-older, or a jump-to-node while following | five writers of the same `scrollTop`; no one owns the "what wins" rule | S2 |
| B3 | A stale sign-in or rate-limit row comes back after you send a message, or a live failure is replaced by an older one | many failure producers, no precedence; the seed race on #5874 | S3 |
| B4 | An automated Monitor / task notification looks like your own message | origin guessed from text; #12499 and #4473 | S4 |
| B5 | A finished background task shows as running, or a status disagrees between the dock and the row | three copies of the status mapping, identical today but free to drift | S4 |
| B6 | Intermittent "nothing happened" after a close, a switch or a bind | `agent-view.tsx` and `useAgentControllerStatus.ts` touched 7 and 4 times in 2 weeks of fixes (audit): every lifecycle fix lands in the same two files | S5 |

Fix-log themes since 10-16 (audit, 40 fix commits): failure / account 5, lifecycle / close 5, scroll 2, message origin 1, history 3, container 3, pane composition 3, notifications 2, other 16.

## 5. Where the big files split (audit, seams not yet confirmed by building them)

| File | Lines | Cut |
|---|---|---|
| `virtualization/AgentDocumentVirtualList.tsx` | 1797 (at the baseline) | delete the old path (-350); then move `handleScrollNow` (~200 lines, 1202-1410) to `scroll/scroll-handler.ts`, the observers to `scroll/scroll-observers.ts`, jump/anchor to `scroll/scroll-commands.ts` |
| `agent-view.tsx` | 1464 | `useShellSubBlockTracking`, `useShellDrawerLifecycle` (322-358), the auth recovery handlers (562-613), send-with-state capture (909-957) into `useAgentCommands`, the keyboard stop rule (968-974), block-meta reads into `agent-model.ts` |
| `components/AgentComposerStrip.tsx` | 1566 (at the baseline) | `composer-row-layout.ts` (balance and rows, ~180), `context-bandwidth.ts` (~120), stats zone (~80) |
| `components/AgentFooter.tsx` | 1500 (at the limit) | `working-row.tsx` (`AgentWorkingRow`, ~350), `status-formatters.ts` (~60) |
| `components/MyAgentsList.tsx` | 1450 | list text and sort helpers (~80) |
| `components/AgentPicker.tsx` | 1075 | the four definition-lookup hooks (~80); `useAgentDefinitions` and `useInstanceOpenDefinitions` may duplicate each other (not verified) |

Not worth cutting: `stream-parser.ts` (one class, self-contained, shrinks after S4), `types.ts` (a schema), `useAgentCommands.ts` (types are the bulk; it gains from S5).

The file-size gate (`scripts/check-file-sizes.mjs`) lets a baseline file only shrink; the two at the limit are the first to go under it.

## 6. The plan: small PRs, each removing one duplicate or splitting along a real seam

Behaviour stays the same except for the bugs a step names. Light checks before every push, one commit and one push for review fixes, merge on approval, report once per merge. Each step also converts the hand-made `<button>`s in the files it touches to the kit's Button / IconButton; the pane's count reaches 0 by the end, and Clare then extends the hard-zero check to `view/agent`.

| Step | What | Fixes | Owner | Depends on |
|---|---|---|---|---|
| **S0** | Delete the `oneWayFlow=false` path and the setting `agent:onewayflow` (old pin, glide, shrink, second observer, legacy disengage). Already approved with #11293 as the "old path removal" follow-up | B2 (part); frees the big file under the limit | Loap-3 | #11294 merged (done, #5866) |
| **S1** | One follow mechanism in dev and release. Finish the controller's soak and turn it on in release and delete `legacyPreviewFollow`, or delete the controller. **Recommendation: finish it**, since it is the newer design and already on in dev | B1 | Loap-3 | S0; operator's call on soak |
| **S2** | One writer of the transcript's `scrollTop`: route `keep-on-screen`, `zoom-anchor-adapter`, the jump / anchor writes and the history view through OneWayFlow's single write. Add a check script that fails when another file writes `scrollTop` on the transcript scroller | B2 | Loap-3 with Camper-2 (zoom #11020 builds on this seam) | S0 |
| **S3** | One failure module: `raiseFailure(source, failure)` and `clearFailure(source, reason)` with written precedence (live event over a command over the stored seed over offline), a late seed that checks "has anything happened since", and a ratchet that fails on a direct `FailureObserved` / `FailureCleared` dispatch outside it. Folds in the #5874 fix | B3 | Manoz (Agent1 meanwhile) | #5874 merged |
| **S4** | Message origin: Agent1's #12499, then Maop's #4473 P1/P2 (typed origin, delete the text classifiers). In the same stretch, one shared task-status mapping | B4, B5 | Agent1, Maop | in flight |
| **S5** | Split `agent-view.tsx` along the table in section 5 into hooks and `agent-model.ts`; aim under 1000 lines; the lifecycle fixes then land in named hooks, not in one file | B6 | Agent1 (Camper if Agent1 is full) | S3 |
| **S6** | Split `AgentComposerStrip`, `AgentFooter` and the scroll handler out of the list; then `MyAgentsList` and `AgentPicker` helpers | file-size gate; fewer merge conflicts | Agent1 (composer, footer, lists), Loap-3 (scroll handler) | S0, S5 |
| **S7** | Last hand-made buttons, the guard checks (single scroll writer, single failure producer), and a docs article on how the pane is built | keeps it clean | Agent1, Clare | S1-S6 |

Order: S0 and S4 now (independent), then S1 and S2, S3 once #5874 merges, S5-S6, S7 last. The operator dogfoods when S7 merges; I report to AgentY then with the list of PRs and the bugs fixed.

## 7. What I did not verify

- The audits' dead-code line counts and the split seams for the big files are estimates; the first step that touches each file confirms or corrects them.
- Whether the controller's soak is as complete as Phase 4 of its spec needs: I did not read the spec's exit criteria.
- The B1-B6 symptoms are inferred from the code; none has been reproduced in a running build.
- Server-side classification (`reactive/sanitize.rs`) and the mobile app's origin handling are out of scope.
- The operator's own bug list is missing; it may change the order.
