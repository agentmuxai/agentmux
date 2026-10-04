# Open issues audit: what's stale, what overlaps, what to do

**Status:** active
**Author:** agent1
**Date:** 2026-09-27
**Investigated at:** `135251326` (main, v0.58.1 + #3949, #3951, #3954)
**Trigger:** the repo owner: "lets do an audit of the open issues on the repo
... there must be a bunch of stale stuff. we also want to consolidate issues
that cover mostly the same ground."

## Summary

68 issues were open, the oldest from 2026-03-31. Each one was checked against
`main`, merged PRs and its comments. A PR counted as delivering something only
if the code confirmed it. A triage bot had commented on most of them on
2026-09-26. Its comments were a useful lead, but one of them (#2956) was
already out of date when it was posted.

- **Nothing was fully done and still open.** No issue can simply be closed as
  fixed. The staleness shows up in other ways: about half the issues are
  **partly delivered**, and their bodies still describe the full original
  scope.
- **11 issues can be closed by folding them into another issue**, or
  transferred to another repo (§1). That takes 68 to 57. As applied it was 58:
  #3061 couldn't be transferred (see §1).
- **16 issue bodies are out of date.** They should be rewritten down to what's
  actually left (§2).
- **Some need something other than code** (§3):
  - 5 are waiting on an owner decision or an ops action.
  - 10 can only be settled by a human running a live test.
  - 3 are ideas with no owner and no activity.
- **#3216 stays open.** The weekly docs-sweep bot upserts it, so closing it
  just makes the bot open a new one.

## 1. Close by folding, or transfer

| Close | Into | Why |
|---|---|---|
| #1469 can_use_tool → decision UI | #551 | Two halves of one feature. Both are gated by the same hardcoded `false` (`persistent/mod.rs:1204`). Move #1469's backend scope (persisted rules, `updatedInput`, other subtypes) into #551. |
| #1400 container Phase 3 host integration | #2939 | This is workstream 1 of the umbrella. Sidecar reachability (#3393) and the image tools (#3427) are done. Move the unobserved acceptance test and the env denylist item into #2939. |
| #2708 verify CaptureWindow on macOS/Linux | #2712 | Platform verification only, low value on its own. |
| #3274 delete: cleanup failures invisible | #3275 | The three #3262 follow-ups (same author, same day) become one "agent delete follow-ups" issue. #3275 (panes in other windows survive) is the real correctness bug. |
| #3276 delete: work-queue rows unclaimable | #3275 | Same. It also needs a product decision (§3). |
| #2024 Armory bundle refs + naming Phases 3–4 | #3148 | Both are parked trackers covering bundle memory references and portability. #3148 is the broader one. |
| #2215 bashwrap idle-timeout | #3338 | The reported repro was fixed in #2589. What's left (the kill message, docs, a foreground command with redirected output killed at 600 s) is item 7 of #3338. |
| #1814 long-running commands tracker | #2979 | Items 2 and 4 are done, and 1 and 5 can be dropped. The one real gap: shells started with `Shell()` aren't stopped when their pane closes (no saga touches `shell_sessions`). That's a sibling of #2979's teardown-on-close, so retitle #2979 to "block teardown on close". |
| #2718 whole-window scrollHeight → 0 px | #2648 | The 251 px lead is closed by analysis. The 0 px collapse hasn't recurred, and since #3652 it's cosmetic. |
| #3473 SearchHistory opaque error | close | Error reporting was fixed in #3693, with a "reopen the agent" hint. The premise is shaky: the repro sent no auth header, and a 401 can't produce reqwest's "error sending request". Accept "reopen the agent" as the design. |
| #3061 muxbus calls a failed review "minor notes" | **transfer to agentmux-cloud** (not done: the auditing account has no access to that repo; commented instead, still open) | Valid bug, but the code is in the cloud repo. |
| #3943 seeded Global Memory never reaches agents | keep, child of #3925 | Not a close. Listed here because it belongs under #3925. Keep it separate until the owner decides (§3). |

## 2. Rewrite to what's actually left

| Issue | What's out of date | What's left |
|---|---|---|
| #2586 jekt WAN verification | Same-account signing shipped (#3727, #3734, #3771, #3775, plus a cloud-side change). Agents no longer hold the account's cloud login (#3881), and verified installs are trusted without an operator stop (#3885). `TRACKING_WAN_JEKT_VERIFICATION_2026_09_25.md` §1 still shows those two as open. | Cross-account W0–W2. The host-gated approval window. Instance retirement from the desktop. §6.5 instance-key storage hardening. A two-machine end-to-end run. |
| #3497 retire the slug | The body says "nothing implemented". Delivered: #3500, #3504, #3508, then M0–M4d-1 (#3543…#3633) and #3845. | M4d-2 to M4d-6 and M5. `db_agents.slug` still has no UNIQUE index. |
| #3477 Global Memory per-instance | The banner and import shipped (#3811), and the tool description now names the scope (#3835). | Only cross-machine sync, which is M5 of `SPEC_MEMORY_FOLLOWS_THE_AGENT`. Retitle to that. |
| #3667 bindings across instances | The session-continuity half is delivered (#3643, #3673, #3817, #3833, #3839, #3841, #3850, #3864). | A notice **before** the first message. "Bind account" exists only on the failure row. |
| #3925 Global Memory delivery | The checklist order changed in the comments. P0–P2 are done: #3942, then the SessionStart hook in #3949 and #3951 (`SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md` §7). | Live verification of P2, then P3–P5. Not commented on: it's under active work. |
| #3148 portability tracker | Phases 0–3 are done. | Format debt: `$schema` says v0.2 but `version` is 0.1.0. `DefinitionRecordV1` has no `memory_id`. Phase 4 now lives in `SPEC_MEMORY_FOLLOWS_THE_AGENT`. |
| #950 bulletproof terminals | Mostly superseded by PtyShell/PtyShellInput (#3177). | A 5 s open timeout with a structured spawn-error UI (G1/G5), and benchmark isolation. |
| #2977 tray + background service | Tray on all three OSes, on by default (#3785). Start at login (#3788, #3854). | macOS `SMAppService`, Windows code signing, the WS3 panel, and live Linux and packaged-macOS runs. |
| #3258 startup off the boot path | Says "P2 blocked on a decision", but the owner decided on 09-16. P3, P4 and P1a are done. | P1b (`run_migration_upgrade` has no caller), the P2 boot gate, the P5 `install_update` stub, and macOS splash parity. |
| #2671 settings audit | Recording, watchdog and drag-and-drop settings are done (#2751, #2748, #2744). | Only the messaging-bridges section. Retitle it. |
| #2939 containers umbrella | #3393, #3427 and the workstream 7 UI aren't ticked. | Workstream 1 acceptance, macOS Keychain, exec as host uid, token refresh, other providers' images, merging the two spawn paths. |
| #3651 install dialog | Phase 2 (#3684) isn't ticked. | Phase 3 (`install.plan`/`install.status`), Phases 4–5, the §13 platform items, and the §11 decisions. |
| #3242 Linux packages | Says `agentmuxai/cef` is private, but it's public. deb, rpm and tar.gz are done (#3236, #3238). | pacman, Snap, Flatpak, arm64, and the `/download` page. macOS Intel isn't tracked anywhere. |
| #2956 tool-preview formatting | #3901 deleted `extractToolDetail` and added the descriptor parity tests (item 1 in substance). The bot's 09-26 comment describes the deleted code. | Item 2: an owner doc mapping the now 11 specs. |
| #1190 browser Ctrl+T/W/F | The code comment says T/W wait on "browser-pane tabs, which don't exist yet", but Universal Pane Tabs exist now. | Decide the key map together with #3319 item 5. Ctrl+F still needs a find bar. |
| #3469 blank panes incident | A grab-bag. | Split into: layer B (`staticTabId()`, 16 frontend hits), layer C, and fork naming (into #3497). The render mechanism needs a live repro. |

## 3. Needs something other than code

**Owner decision or ops action**
- #2115: provision real OAuth client IDs. Every provider has `client_id: None`; the bring-your-own-client path is wired.
- #2481: a one-time `wingetcreate new`, then re-enable the WinGet job and fix the MS Store step.
- #3943: the seeded "Workspace Rules" and "Agent Memory" entries. Keep seeding them (and fix where they're written), or drop them?
- #3276: what happens to work queued for a deleted agent: reassign it, fail it, or leave it?
- #2712: a policy for agents clicking or querying other agents' panes.

**Needs a human, live test**

| Issue | What to test |
|---|---|
| #2155 | Editor rename-input caret. The triage has an A–G retest plan using real OS input. |
| #2873 | Tearing a pane off over another floater silently does nothing. |
| #768 | Phantom browser pane. The repro predates the tear-off rework. |
| #3846 | Approval windows come back after a restart. Only if AgentMux exits while one is open, and closing the stray window clears it. |
| #2188 | macOS close and quit chain. Tray-on-by-default changes the setup. |
| #3614 | Idle hung-renderer checks V1–V10, on an isolated box. |
| #2707 | The Working row restarting its type-out. Not re-checked since the #2921, #2946 and #3143 rework. |
| #2357 | A deferred `/login` refresh getting stuck. A low-priority edge case. |
| #3537 | The eager-resume race. Found by reading the code; it needs a targeted test. |
| #3469 | The blank-pane render mechanism (DevTools on a live blank pane). |

**Ideas with no owner or activity**
- #261: Computer Use pane. The screenshot half exists as CaptureWindow; no input half.
- #2504: Blender Phase 1 (spec only, #3783). Its input half depends on #261.
- #3546: private browser identities (spec only, #3739).

## 4. Still valid, no change needed

- **Agents and permissions:**
  - #1247: Codex permission modes are always full-bypass.
  - #1250: the Gemini auth check is inert.
  - #3894: deferred jekts ignore the sender's expiry. A cloud-side change reduced the flood; the srv fix remains.
  - #3680: the `.mcp.json` overwrite is fixed (#3803); the remaining key-handling follow-up is blocked on #3497 M4d/M5.
- **Panes and UI:**
  - #2551: OAuth popup sizing.
  - #2908: Ctrl+Wheel zoom in floaters.
  - #2842: the remaining PaneReadiness gates.
  - #2648: scroll-shrink steps 4–6.
  - #3655: the scroll-follow tracker.
  - #3319: pane-tab QA, shortcuts, and the editor's separate tab strip.
  - #3361: typing lag. Re-measure the baseline first.
- **Host and platform:**
  - #942: the supervision umbrella. No srv supervision on Unix, and `SupervisedService` is Phase 3.
  - #3666: killing the host counts as a crash, on Windows and Unix.
  - #3338: agent availability.
  - #871: a theoretical TOCTOU.
  - #3060: an mDNS label collision.
  - #2711: an OpenPane tool.
- **CEF fork and CI:** #3127 (transparency gating), #3160 (a testonly compile failure), #3140 (nightly CI: macOS leg, failure email, macOS PR leg).
- **Other:** #3853 (Mux Code harness).

## 5. Themes for later consolidation

These overlap, but aren't duplicates:
- **Floater architecture:** #2908, #3846, #2873 and #768 all live in `floating_pane.rs`. Floaters are child HWNDs recorded as `Subwindow`. One design pass would cover them.
- **Supervision:** #942 is the umbrella; #3614, #3666 and #2188 are focused children. Keep them as they are.
- **Browser popups:** #3546 G6 (popups stay in the pane's identity) and #2551 both touch `on_before_popup`. Do them in sequence.
- **Hoisted pane chrome missing:** #3469 §2 and #3319's Windows missing-header finding may be one bug. Worth one repro.
