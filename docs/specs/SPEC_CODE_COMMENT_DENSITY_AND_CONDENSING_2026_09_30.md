# SPEC: Code comment density and condensing

**Date:** 2026-09-30
**Author:** Maricon (charlie)
**Status:** active — Phase 0 (the gate, `--code-equal`, the `CONTRIBUTING.md` rules) is built in #4246, described in §9. Phase 1+ (condensing files) is not started.
**Baseline:** `main` @ `cee674b6f`. Every `path:line` below was read on that commit.
**Related:** [`SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30`](SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md) §7 ("Keep the prose", revisited in §3 here), [`SPEC_CLAUDE_MD_CONTENT_PORT_2026_09_18`](SPEC_CLAUDE_MD_CONTENT_PORT_2026_09_18.md) (why this repo has no agent-instructions file).

---

## 0. TL;DR

- **Comments are over a third of what an agent reads.** Excluding tests and generated code, comments are 27.7% of lines and ~37% of characters (§1). In the hottest files the share is 45–68% of characters: `agent-view.tsx`, `useAgentCommands.ts`, `server/mod.rs`.
- **Most of the content is worth keeping; the length is the problem.** In a 100-block sample, 85% of comment lines are kinds worth keeping (invariants, why-nots, interface docs, spec links). Review narration, restating, essays and stale text make up the other ~15% (§2). Nearly half of all comment lines sit in blocks of 10+ lines.
- **Rule: state the constraint and its consequence in ≤3 lines, and cite the PR once as `(#NNNN)`.** No reviewer names, severity tags, round counts or "an earlier version" stories. Explain a rule once, where it is enforced. Move essays over 8 lines to `docs/`, leaving a one-line pointer (§4).
- **Condense file by file, in comment-only PRs.** Check each PR mechanically: the code token stream, with comments stripped, must be identical before and after (§5, §7). Never condense inside a move-only refactor.
- **Stop regrowth with one cheap CI gate.** Fail on review-history markers in *added* comment lines. Report comment density per touched file, but don't fail on it. Put the rules in `CONTRIBUTING.md` (§6).
- **Expected savings (estimates):** 35–50% of comment tokens in condensed files. That is about 13–18% of all tokens an agent reads from non-test source, and 19–34% for the hottest files. `agent-view.tsx`: ~5.6–8.0k tokens per full read.
- **Risks:** deleting a guardrail and a fixed bug returning; noisy diffs colliding with parallel agent branches; a density gate so strict it gets switched off. The mitigations are the safety rules (§5.1), one file per PR (§7), and making the density check report-only (§6).

## 1. Measurement

### 1.1 Method

- **Files:** every tracked `*.ts`, `*.tsx` and `*.rs` on `main`, taken from `git archive main`. Excluded: `node_modules/`, `target/`, `dist/`, `build/`, the ts-rs bindings under `frontend/types/rpc/` (374 generated files) and `agentmux-launcher/src/splash_font.rs` (`@generated`). The hand-maintained `.d.ts` files stay in. Lockfiles are not TS/RS, so they are out by construction.
- **Test files:** `*.test.ts(x)`, `*.spec.ts(x)`, `tests/` dirs, `*_test(s).rs`. Rust inline `#[cfg(test)] mod tests` blocks count as non-test (not separable without parsing).
- **Counter:** a small lexer, not committed. §6 proposes committing it as the gate's counter. It skips strings, template literals (including `${}` nesting), Rust raw strings, char literals and nested `/* */`, so `//` inside a URL is not a comment.
  - A **comment line** has comment characters and no code characters. A code line with a trailing comment counts as code (TS 920, Rust 689 such lines), but its comment characters still count toward comment tokens.
  - The copyright/SPDX header in a file's first five lines is excluded.
  - Syntax kinds: `//`, `/* */`, JSDoc `/** */`, Rust `///` and `//!`.
- **Validation:** on `agent-view.tsx` a naive `^\s*(//|/\*|\*)` grep finds 863 comment lines and the lexer finds 968. The 105 extra lines are continuation lines of JSX `{/* … */}` comments (e.g. `agent-view.tsx:1675-1686`), which the grep misses.
- **Tokens:** ≈ chars / 4 (estimate). Comment tokens count comment characters, including inner spaces but not indentation. File tokens are bytes / 4. Two biases pull in opposite directions: prose tokenizes more efficiently than code, and indentation runs tokenize cheaply. Treat every token figure as ±20%.
- **Read-frequency proxy:** commits touching the file on `main` since 2026-08-01.

### 1.2 Repo-wide

| Scope | Files | Lines | Comment lines | Line ratio | Comment share of chars (≈ tokens) |
|---|---:|---:|---:|---:|---:|
| TS/TSX, non-test | 853 | 178,103 | 55,247 | 31.0% | 39.6% |
| TS/TSX, tests | 476 | 100,397 | 12,538 | 12.5% | 17.2% |
| Rust, non-test | 713 | 382,480 | 99,824 | 26.1% | 35.4% |
| Rust, tests | 71 | 53,095 | 7,867 | 14.8% | 22.1% |
| **All non-test** | 1,566 | 560,583 | 155,071 | **27.7%** | **36.8%** |
| All | 2,113 | 714,075 | 175,476 | 24.6% | 33.0% (~2.6M of ~7.9M tokens) |

Syntax split (non-test comment lines):

| Language | Syntax | Lines |
|---|---|---:|
| TS | JSDoc `/** */` | 27,713 |
| TS | `//` | 26,570 |
| TS | `/* */` | 964 |
| Rust | `///` | 48,040 |
| Rust | `//` | 43,238 |
| Rust | `//!` | 8,546 |

Block size (non-test comment lines):

| Block length | Share of comment lines |
|---|---:|
| 1 line | 6% |
| 2–3 lines | 13% |
| 4–9 lines | 34% |
| 10–19 lines | 26% |
| 20+ lines | 21% |

Among non-test files of 500+ lines, the median line ratio is 34% for TS (71 files) and 26% for Rust (251 files). 24 files in each language are at 40% or more.

### 1.3 Top 20 by comment lines

TS/TSX:

| # | File | Lines | Comment lines | Ratio | Comment tok (est.) | File tok (est.) | Comment tok share | Commits since 08-01 |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | `frontend/app/view/agent/hooks/useAgentCommands.ts` | 1,934 | 1,308 | 68% | 18,856 | 27,877 | 68% | 15 |
| 2 | `frontend/app/view/agent/agent-view.tsx` | 2,111 | 968 | 46% | 16,037 | 29,380 | 55% | 121 |
| 3 | `frontend/app/view/agent/components/AgentComposerStrip.tsx` | 1,526 | 927 | 61% | 14,384 | 21,929 | 66% | 26 |
| 4 | `frontend/app/view/swarm/swarm-model.ts` | 2,086 | 886 | 42% | 14,047 | 26,416 | 53% | 17 |
| 5 | `frontend/app/store/agent-pane-state/types.ts` | 1,213 | 859 | 71% | 11,600 | 14,729 | 79% | 22 |
| 6 | `frontend/app/view/agent/hooks/useAgentControllerStatus.ts` | 1,442 | 804 | 56% | 11,591 | 20,225 | 57% | 12 |
| 7 | `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` | 1,586 | 747 | 47% | 11,992 | 22,500 | 53% | 14 |
| 8 | `frontend/app/store/agent-pane-state/reducer.ts` | 1,732 | 713 | 41% | 9,923 | 22,189 | 45% | 16 |
| 9 | `frontend/app/view/agent/hooks/useAgentCommands.test.ts` | 3,338 | 665 | 20% | 10,268 | 41,174 | 25% | 10 |
| 10 | `frontend/app/view/agent/components/AgentFooter.tsx` | 1,310 | 574 | 44% | 8,936 | 17,042 | 52% | 21 |
| 11 | `frontend/app/store/agent-pane-state/reducer.test.ts` | 3,519 | 567 | 16% | 8,040 | 45,151 | 18% | 13 |
| 12 | `frontend/app/view/agent/types.ts` | 995 | 485 | 49% | 6,361 | 9,491 | 67% | 26 |
| 13 | `frontend/app/view/agent/components/MyAgentsList.tsx` | 1,286 | 459 | 36% | 7,406 | 18,311 | 40% | 12 |
| 14 | `frontend/app/view/agent/components/AgentPicker.tsx` | 1,128 | 445 | 39% | 6,277 | 14,471 | 43% | 25 |
| 15 | `frontend/app/view/agent/agent-model.ts` | 961 | 442 | 46% | 6,229 | 12,745 | 49% | 46 |
| 16 | `frontend/app/view/agent/useAgentStream.ts` | 954 | 439 | 46% | 6,106 | 13,225 | 46% | 23 |
| 17 | `frontend/app/view/editor/editor-model.ts` | 1,407 | 434 | 31% | 6,899 | 16,644 | 41% | 17 |
| 18 | `frontend/app/view/agent/components/AgentShellSubblock.tsx` | 788 | 430 | 55% | 6,402 | 10,973 | 58% | 12 |
| 19 | `frontend/app/view/agent/stream-parser.ts` | 895 | 388 | 43% | 5,061 | 10,061 | 50% | 10 |
| 20 | `frontend/app/element/PaneTabStrip.tsx` | 816 | 384 | 47% | 6,258 | 10,894 | 57% | 23 |

Rust:

| # | File | Lines | Comment lines | Ratio | Comment tok (est.) | File tok (est.) | Comment tok share | Commits since 08-01 |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | `agentmux-srv/src/backend/storage/agents.rs` | 3,787 | 1,390 | 37% | 20,736 | 45,934 | 45% | 39 |
| 2 | `agentmux-bashwrap/src/bash_wrap.rs` | 3,634 | 1,343 | 37% | 20,349 | 43,112 | 47% | 9 |
| 3 | `agentmux-srv/src/server/mod.rs` | 3,328 | 1,179 | 35% | 18,554 | 39,897 | 47% | 81 |
| 4 | `agentmux-common/src/ipc.rs` | 2,269 | 1,053 | 46% | 14,285 | 23,876 | 60% | 8 |
| 5 | `agentmux-srv/src/backend/blockcontroller/persistent/spawn.rs` | 2,144 | 1,029 | 48% | 13,425 | 33,433 | 40% | 30 |
| 6 | `agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs` | 2,832 | 994 | 35% | 14,735 | 34,862 | 42% | 22 |
| 7 | `agentmux-srv/src/backend/storage/migrations.rs` | 3,023 | 983 | 33% | 15,897 | 40,216 | 40% | 40 |
| 8 | `agentmux-srv/src/server/tests.rs` | 6,768 | 980 | 14% | 15,828 | 70,424 | 22% | 62 |
| 9 | `agentmux-cef/src/commands/window_pool.rs` | 2,184 | 907 | 42% | 13,773 | 25,257 | 55% | 6 |
| 10 | `agentmux-srv/src/server/reactive.rs` | 2,599 | 906 | 35% | 14,185 | 30,133 | 47% | 39 |
| 11 | `agentmux-srv/src/backend/lan_discovery.rs` | 2,693 | 885 | 33% | 13,637 | 31,441 | 43% | 13 |
| 12 | `agentmux-srv/src/backend/blockcontroller/persistent/mod.rs` | 1,675 | 872 | 52% | 13,703 | 21,648 | 63% | 19 |
| 13 | `agentmux-cef/src/state/mod.rs` | 1,625 | 861 | 53% | 12,930 | 21,175 | 61% | 17 |
| 14 | `agentmux-common/src/data_paths.rs` | 2,632 | 860 | 33% | 12,428 | 29,916 | 42% | 9 |
| 15 | `agentmux-srv/src/backend/bundle_import.rs` | 2,851 | 850 | 30% | 13,186 | 35,209 | 37% | 10 |
| 16 | `agentmux-srv/src/identity/resolver/inject.rs` | 3,414 | 840 | 25% | 12,472 | 38,357 | 33% | 22 |
| 17 | `agentmux-srv/src/backend/agent_config.rs` | 2,869 | 835 | 29% | 12,965 | 35,063 | 37% | 15 |
| 18 | `agentmux-srv/src/backend/blockcontroller/persistent/tests/send_input.rs` | 4,182 | 825 | 20% | 13,426 | 44,711 | 30% | 9 |
| 19 | `agentmux-cef/src/client/lifecycle.rs` | 1,669 | 815 | 49% | 11,725 | 22,534 | 52% | 15 |
| 20 | `agentmux-srv/src/bootstrap.rs` | 2,441 | 804 | 33% | 12,712 | 30,010 | 42% | 61 |

### 1.4 Cost per read of the named files

| File | Lines | Comment lines | File tok (est.) | Comment tok (est.) | Commits since 08-01 |
|---|---:|---:|---:|---:|---:|
| `frontend/app/view/agent/agent-view.tsx` | 2,111 | 968 (46%) | ~29,400 | ~16,000 (55%) | 121 |
| `frontend/app/view/agent/hooks/useAgentCommands.ts` | 1,934 | 1,308 (68%) | ~27,900 | ~18,900 (68%) | 15 |
| `frontend/app/store/agent-pane-state/reducer.ts` | 1,732 | 713 (41%) | ~22,200 | ~9,900 (45%) | 16 |
| `frontend/app/store/agent-pane-state/types.ts` | 1,213 | 859 (71%) | ~14,700 | ~11,600 (79%) | 22 |
| `agentmux-srv/src/bootstrap.rs` | 2,441 | 804 (33%) | ~30,000 | ~12,700 (42%) | 61 |
| `agentmux-srv/src/server/mod.rs` | 3,328 | 1,179 (35%) | ~39,900 | ~18,600 (47%) | 81 |

`bootstrap.rs` is being split into `bootstrap/*` on an open branch. The split moves the same comments, so the total is unchanged; re-measure after it lands.

Reading these six files once costs ~164k tokens (est.), of which ~88k are comments. There is also a line-limit cost. Claude Code's `Read` returns 2,000 lines by default, and `agent-view.tsx` is over that, so a full read takes two calls. At 35–50% fewer comment lines it would be ~1,630–1,770 lines.

### 1.5 Review-history markers and stale references

- **Review markers:** 3,636 non-test comment lines (2.3%) in 644 files carry a review-history marker. The regex matches a severity tag near a PR number (`P1 on PR #2338`), `re-review`, `round N`, or a review-bot name. In a 30-hit spot check, every hit was narration.
  - The heaviest files are `useAgentCommands.ts` (113 lines), `bundle_import.rs` (102), `persistent/spawn.rs` (57), `storage/agents.rs` (49) and `useAgentControllerStatus.ts` (47).
  - These counts are marker lines only; the narration around them adds more (§2.2).
- **Stale file references:** comments reference file names 8,146 times. 326 of those, in 203 files, name one of 160 file names that exist nowhere in the tree. This excludes names AgentMux generates at runtime, such as `CLAUDE.md` and `GEMINI.md`.
  - The top misses are modules split into directories: `persistent.rs` (46), `app.rs` (12), `app_api.rs` (11), `browser_panes.rs` (10), `shell.rs` (9).
  - Treat 326 as an upper bound. Some are deliberate history ("Split from the original rpc-api.ts.", `frontend/app/store/rpc-api/workspace.ts:5`).
  - `check-spec-citations.sh` already catches dead `docs/**.md` paths, but only in changed files, and not source-file names.
- **Verbatim duplication is small:** 1,127 duplicate comment lines of 60+ characters, mostly ruler lines. Paraphrased duplication is what costs, and a script cannot find it (§5.2).

## 2. Taxonomy

### 2.1 Kinds, with examples

**(a) Invariant / constraint: keep.**
- `useAgentCommands.ts:697-699`: "This destructive action requires POSITIVE proof of idle, not just absence of proof of activity."
- `frontend/layout/lib/TileLayout.core.tsx:327-330`: "Keeping them together is deliberate: the cursor grab-offset must be derived from the size the image actually has, and a separate nominal constant is exactly how the two drift and the ghost detaches from the cursor."
- `agentmux-srv/src/backend/layout/mod.rs:698-699`: "I2 — the `minimized` flag and the legacy minimizedSize/slipMinimize markers are leaf-only."

**(b) Why-not / rejected alternative: keep, short.**
- `frontend/app/util/pointer-drag-state.ts:35-41`: "Iterating the live Set on purpose, NOT a copy. … A defensive `[...releaseListeners]` copy would call it anyway, which is worse, not safer."
- `agentmux-common/src/runtime_mode.rs:95-97`: "sanitize it directly, do NOT call detect_branch (which has its own git fallback …"
- `useAgentCommands.ts:691-692`: "Deliberately `!isBackendTurnConfirmedIdle()`, NOT `isBackendTurnActive()`".

**(c) Review-history narration: move to git/PR.**
- `useAgentCommands.ts:780`: "codex P1 on PR #2338 (sixteenth re-review) pins that flag"
- `frontend/app/view/agent/hooks/useSubagentBackfillGate.ts:19`: "reagentx P1 + codex P1 (PR #2781, round 2): an earlier version defaulted"
- `agentmux-srv/src/backend/blockcontroller/app_server_controller.rs:323`: "ReAgent P1 on PR #3215's second review: don't just log and"

**(d) Restating the code: delete.**
- `agentmux-srv/src/backend/blockcontroller/persistent/spawn.rs:502`: "// Create stdin writer channel"
- `agentmux-srv/src/backend/reactive/sanitize.rs:83`: "// Keep printable characters and valid UTF-8"
- `frontend/layout/lib/layoutResize.ts:264`: "Callback that is invoked when the TileLayout container is being resized."

**(e) Incident narrative / design essay: move to `docs/`, leave a pointer.**
- `agentmux-srv/src/migrations/m0013_agent_direct_bindings.rs:4-23`: a 20-line module doc on a retired, no-op migration. Keep the one constraint ("the migration id stays registered") and the spec pointer.
- `agent-view.tsx:1080-1107`: a 28-line essay on the focus re-poll, including "(removed per direct user request — …)".
- `frontend/app/store/block-atom-cache.ts:51`: "Diagnostic-only snapshot — added while investigating a 2026-09-20 latency/memory report."

**(f) Cross-reference to a spec: keep, one line.**
- `frontend/app-init.ts:1015`: "Spec: docs/specs/SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §7 Phase 3."
- `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx:736-739`: route through the CEF clipboard wrapper, then "SPEC_UNIFIED_CLIPBOARD_2026_05_18.md §3.3."
- `frontend/app/store/agent-pane-state/types.ts:567`: "See docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md."

**(g) Stale: fix.**
- `agent-view.tsx:1103`: "persistent.rs's spawn_status_heartbeat". The function is now at `agentmux-srv/src/backend/blockcontroller/persistent/status.rs:65`.
- `agent-view.tsx:1072`: "Extracted into `declareAuthHealthy` (defined below, in scope via closure)". It now comes from `useAuthHealth` (`agent-view.tsx:1428`, `failure/useAuthHealth.ts:112`).
- `agentmux-srv/src/server/service/tab_move.rs:53-55`: "which appended at end... wait, wcore appends": thinking aloud, left in.

**(h) Interface / contract doc: keep.** Added because about a third of the sample fits none of (a)–(g). It covers what a field, parameter or return value means.
- `agentmux-srv/src/backend/storage/filestore/lines.rs:37-41`
- `agentmux-srv/src/backend/gh_guard.rs:75-80`: "Never fails the spawn: if the directory can't be prepared the variable is still set …"
- `frontend/app/view/agent/attachments/AttachmentLightbox.tsx:31`: "Still copying or processing (composer only): no id yet, but not gone."

### 2.2 Share by kind (sample)

**Method:**
- 100 comment blocks, drawn uniformly at random with a fixed seed from non-test files: 50 TS and 50 Rust. A block is a run of consecutive comment-only lines.
- One reader (the author) assigned each line to a kind; mixed blocks were split by line.
- n = 495 lines, so each share is ±4–5 points (sampling alone).
- The weight on TS vs Rust is equal, not proportional (Rust has 1.8× the comment lines).

| Kind | TS (286 lines) | Rust (209) | All (495) |
|---|---:|---:|---:|
| (a) invariant | 44% | 43% | 44% |
| (h) interface doc | 35% | 25% | 31% |
| (b) why-not | 5% | 10% | 7% |
| (c) review narration | 3% | 7% | 5% |
| (d) restating | 5% | 4% | 4% |
| (e) essay / narrative | 3% | 6% | 4% |
| (f) spec link | 5% | 2% | 3% |
| (g) stale | 0% | 3% | 1% |

**Reading:** (c)+(d)+(e)+(g) is ~14%. Deleting whole kinds would save little. The saving is in length: the same sample, rewritten under §4's rules without dropping any (a)/(b)/(f)/(h) content, shrinks from 495 to ~242 lines (TS −56%, Rust −44%). That rewrite is the author's estimate, not an executed edit, and it is optimistic. §0 therefore quotes 35–50%.

## 3. Revisiting "Keep the prose"

`SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md:377` says: "Keep the prose. agent-view's comments are its design record; move them with their code." It sits in a list of cautions for **move-only** refactors. Against the evidence:

- **Right about content.** 85% of sampled comment lines carry meaning worth keeping (§2.2), and the guardrails are real: `useAgentCommands.ts:697-699` records a bug the next edit could reintroduce.
- **Right for moves.** Mixing condensing into a move makes the move unreviewable. §7 keeps that rule.
- **Wrong as a standing policy.**
  - *The record is ~2× longer than its meaning.* The random sample condenses by ~50% (§2.2), and long blocks by 60–91% (§5.2).
  - *Parts of the record are wrong.* There are 326 dead file references (§1.5), and at least two stale pointers in `agent-view.tsx` (§2.1 g).
  - *The same rationale appears three times.* The auth-only gate is explained at `agent-view.tsx:1062-1070`, `useAgentCommands.ts:755-766` and `failure/useAuthHealth.ts:105-111`.
  - *The narration isn't the record.* "(thirty-fifth re-review)" adds nothing that `(#2338)` does not reach. `main` squash-merges (0 merge commits against 1,060 commits since 2026-09-01), so the review thread lives on the PR, one `gh pr view 2338 --comments` away.

**Revised position:** keep the meaning, not the prose. Move comments unchanged inside a move PR, and condense them in a separate, comment-only PR.

## 4. Best practice and the rule set

### 4.1 What established guidance says

- **Comments explain why; the code says what.** Google's "Code Health: To Comment or Not to Comment?" (Testing Blog, 2017) says to explain the why, and to consider refactoring before commenting on what code does. Kernighan & Pike (*The Practice of Programming*, ch. 1) add: don't belabor the obvious, and don't contradict the code. Kinds (d) and (g) break these.
- **Interface docs vs implementation comments.**
  - The Google TypeScript Style Guide: use `/** JSDoc */` for documentation a user of the code should read, and `//` for implementation comments. It also says not to restate what the types already say.
  - Rust's `///`/`//!` render through rustdoc, which shows the first paragraph as the item summary. A summary sentence first, detail after, is the convention.
  - Clippy's `undocumented_unsafe_blocks` requires a `// SAFETY:` comment. This repo has 34 of those for 333 `unsafe {` sites: an under-commented area, out of scope here.
- **History belongs in version control.** Commit messages and PR descriptions record what changed and why at that moment; `git log -L` and `git blame` retrieve it per line. A comment should describe the code as it is now.
- **Design rationale that spans modules goes in a design doc or ADR** (Nygard, "Documenting Architecture Decisions", 2011; Ousterhout, *A Philosophy of Software Design*, ch. 13 on cross-module design notes). This repo already has `docs/specs`, `docs/architecture` and `docs/retro` for it.

### 4.2 What is different when agents maintain the code

- **Comments are always-on context; history is on demand.** Every full read of a file pays for every comment in it, relevant or not. Anthropic's "Effective context engineering for AI agents" frames the goal as "the smallest possible set of high-signal tokens". A PR thread costs nothing until an agent runs `gh pr view`.
- **Guardrails work, but the constraint sentence does the guarding.** What stops the next agent reintroducing a bug is "requires POSITIVE proof of idle". The fact that ReAgent found it in round 21 adds nothing. This is reasoning, not measurement; no data here isolates the effect of a comment.
- **Where a guardrail is cheaper as a test:** if a comment exists to stop a regression, a named regression test enforces it and costs nothing to read. The comment can then be one line naming the test.
- **Where the narration comes from:** the fixing agent annotates each review finding at the fix site. Reviewers post PR comments, not code. Prevention has to target the agent writing the fix (§6).

### 4.3 Rules

Each rule says how it is checked.

| # | Rule | Check |
|---|---|---|
| C1 | No reviewer or bot names, severity tags (`P0`–`P3`), round or re-review counts, or "an earlier version / previously" stories. | Gate on added lines (§6) |
| C2 | Cite the PR or issue once, as `(#NNNN)`, next to the constraint it justifies. Cite a spec as `SPEC_NAME.md §N`. | Review |
| C3 | A why-comment states the constraint and the consequence of breaking it, in ≤3 lines. An interface doc may be longer, but its first sentence is the summary. | Report: added blocks >8 lines (§6) |
| C4 | Explain a rule once, where it is enforced. Other sites get one line pointing there. | Review |
| C5 | Rationale longer than ~8 lines that isn't interface doc goes to `docs/` (spec, architecture or retro), with a one-line pointer. | Report (C3) |
| C6 | Never restate the code or the types (`@param node The node`). | Review |
| C7 | Paths and symbols named in a comment must exist. Update them in the PR that moves the code. | `check-spec-citations.sh` for docs; proposed extension for source names (§6) |
| C8 | Never remove `SAFETY:`, `TODO`/`FIXME`, lint or compiler directives (`eslint-disable`, `@ts-expect-error`, `/// <reference`) or doctest fences. | Condense-diff check (§7) |

## 5. Condensing method

### 5.1 Procedure, per file

1. **Read the whole file first.** Condensing blind to the code loses the "why here" that placement comments carry.
2. **Tag each block** by kind (§2.1).
3. **Rewrite by kind:**
   - (a)/(b): constraint + consequence, ≤3 lines, one `(#NNNN)`.
   - (h): a summary sentence first; keep every field, unit, error and edge case.
   - (c): delete the narration and keep the PR number.
   - (d): delete.
   - (e): move to `docs/`, leave a pointer.
   - (f): one line.
   - (g): correct it against current code, or delete it if it has no referent.
4. **De-duplicate:** where a rule is explained at a definition and repeated at call sites, keep the definition's version (C4).
5. **Safety rules:**
   - Never delete an invariant or a why-not. Shorten it.
   - Keep every PR or spec citation that justifies a constraint: one citation, not the story.
   - Keep comments that guard ordering or placement ("must run before…", "a direct child of…") next to the code they guard.
   - Never touch C8 items.
   - Test files are out of scope for the first pass (12–15% comment ratio). Their comments often name the bug each test pins.
6. **Verify** (§7.3): code unchanged, tests green, and the meaning check passes.

### 5.2 Before and after

Token counts are chars / 4 of the comment text without indentation (estimates). These eight blocks are chosen for length, so their 78% cut (~1,964 → ~441 tokens) is not representative. Use §2.2 for averages.

#### 5.1 `frontend/app/view/agent/hooks/useAgentCommands.ts:706-727`, 22 → 4 lines, ~303 → ~60 tokens

Before:

```ts
// forceControllerRefresh's failure path
// (useAgentControllerStatus.ts) is best-effort — it only
// logs a warning and never sets canRetry()/loginWaiting()
// or state.failure. controllerRefreshPendingUntilIdle was
// ALREADY cleared above (this function commits to running
// once it decides to attempt the refresh, success or
// failure) — leaving it cleared here would mean NOTHING is
// left blocking: the very next fresh idle send would pass
// every guard in checkAuthGuard cleanly (canRetry,
// loginWaiting, authFailureToPreserve, live failure all
// read clean) and reach the still-stale, un-refreshed
// controller — reproducing the exact doomed "Working…"
// send this PR exists to prevent, just for the
// deferred-refresh-then-fails branch specifically. Re-arm
// the pending flag so the NEXT trigger (a later turn-end,
// the reactive turnIdle effect, or a fresh send's own
// call) retries the refresh instead of treating this as
// resolved — mirrors deferControllerRefreshUntilIdle's own
// semantics rather than inventing a new signal; every
// existing caller of flushPendingControllerRefresh already
// handles "still pending" correctly (hold, don't deliver).
// reagent P1 on PR #2338 (twenty-sixth re-review).
```

After:

```ts
// forceControllerRefresh failed without setting canRetry, loginWaiting
// or failure, so re-arm: otherwise the next idle send passes
// checkAuthGuard and reaches the stale controller. Callers already
// treat "still pending" as hold (#2338).
```

Keeps: why the flag is re-armed, what happens otherwise, that callers already handle it. Drops: the narration of which guards read clean, the round count.

#### 5.2 `frontend/app/view/agent/hooks/useAgentCommands.ts:755-766`, 12 → 2 lines, ~167 → ~34 tokens

Before:

```ts
// Gated on the failure actually being "auth" — FailureCleared
// has no payload and unconditionally clears state.failure
// regardless of code (reducer.ts's FailureCleared case), so
// dispatching it unconditionally here would silently
// dismiss an unrelated concurrent failure (rate_limited,
// overloaded, context_exceeded, unresponsive, etc.) showing
// on the same pane once a deferred /login refresh
// completes, even though that unrelated problem was never
// actually resolved. Mirrors the established pattern at
// useAgentFailure.ts's silent self-heal handler ("never
// blow away an unrelated concurrent failure"). reagentx P1
// on PR #2338 (thirty-fifth re-review).
```

After:

```ts
// Auth failures only: FailureCleared clears any code, and an
// unrelated failure (rate_limited, overloaded, ...) is still real (#2338).
```

Keeps: the gate and its reason. Drops: the pointer to a sibling pattern and the round count. The same rule is stated a third time at `agent-view.tsx:1062-1070`; after this pass it lives once, in `failure/useAuthHealth.ts:105-111`.

#### 5.3 `frontend/app/view/agent/agent-view.tsx:1032-1075`, 44 → 3 lines, ~600 → ~54 tokens

Before:

```ts
// A controllerstatus event with an ACTIVE turn is independent
// proof the CLI is alive and running turns — clear any stale
// "Retry Login" / auth notice left over from the mount-time
// gated launch flow's auth_failed classification. Otherwise
// the button can outlive the failure it was reporting: an
// agent recovers and starts answering messages through this
// same event stream, but nothing ever told useAgentControllerStatus
// its earlier canRetry=true was stale. Reported live 2026-07-18.
// Gated on an ACTIVE turn specifically (not any controllerstatus
// event) — codex P1 on PR #2338 (eighth re-review): an idle
// heartbeat from a controller left alive from before a
// just-FAILED recovery attempt carries no proof the credential
// is valid, and would otherwise silently clear that recovery's
// own canRetry=true, letting the next message bypass the
// fast-fail guard and reach the still-known-bad process.
//
// Also clears a stale live "auth"-classified state.failure —
// unlike the OTHER two places in this PR that declare a
// controller healthy (login.ts's finalizeLoginSuccess,
// useAgentCommands.ts's flushPendingControllerRefresh success
// path), this call site only ever cleared canRetry via
// notifyControllerHealthy, never the separate state.failure
// checkAuthGuard's liveAuthFailure check reads
// (paneSnapshot(...).failure?.data.code === "auth"). Without
// this, a stale failure row survives even this independent,
// stronger proof of health (a live controllerstatus event
// showing a turn genuinely streaming), permanently
// fast-failing every subsequent send. reagentx P1 on PR #2338
// (thirty-second re-review).
//
// Gated on the failure actually being "auth" — FailureCleared
// has no payload and unconditionally clears state.failure
// REGARDLESS of code (reducer.ts's FailureCleared case), so
// dispatching it unconditionally here would ALSO silently wipe
// an unrelated concurrent failure (rate_limited, overloaded,
// context_exceeded, etc.) that happens to be showing the moment
// a turn-active event arrives, even though that unrelated
// problem was never actually resolved. reagentx P1 on PR #2338
// (thirty-fifth re-review).
//
// Extracted into `declareAuthHealthy` (defined below, in scope
// via closure) so the SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md
// §2 bind-event listener can reuse the identical logic instead of
// duplicating this exact gating a second time.
```

After:

```ts
// An ACTIVE turn proves the credential works; an idle heartbeat does
// not (it can come from a controller left over from a failed recovery).
// What gets cleared, and why only "auth": failure/useAuthHealth.ts (#2338).
```

44 comment lines at a one-line call site. The auth-only gating is already stated at the definition (`failure/useAuthHealth.ts:105-111`), and "Extracted into `declareAuthHealthy` (defined below, in scope via closure)" is stale: it now comes from a hook (`agent-view.tsx:1428`). Keeps: why ACTIVE and not idle, plus a pointer.

#### 5.4 `frontend/app/view/agent/agent-view.tsx:1674-1687`, 14 → 3 lines, ~188 → ~53 tokens

Before:

```ts
{/* Loading overlay — covers the pane from mount until the initial
    history load resolves, so a content-heavy pane never sits
    blank while it replays. See
    docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md.

    Deliberately a DIRECT child of `.agent-view`, OUTSIDE
    `.agent-view-zoomed`. It is `position: absolute; inset: 0`
    (PaneLoadingCover.scss), so it covers its nearest POSITIONED
    ancestor — and the zoomed wrapper is `position: relative`. Nested
    inside it, the overlay stopped covering the Shell drawer (a sibling
    of the wrapper), leaving an open drawer visible and uncovered for
    the whole load. Keeping it out here also keeps it unscaled, so the
    cover can't be 69%-sized by the pane zoom.
    See REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md §F. */}
```

After:

```ts
{/* Loading cover (REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md §F). Must be a
    direct child of `.agent-view`, outside `.agent-view-zoomed`: nested, it
    stops covering the Shell drawer and gets scaled by pane zoom. */}
```

Keeps: the placement constraint (load-bearing: it is why the element sits where it does), both consequences, one citation. Drops: the CSS mechanics a reader can see in the SCSS.

#### 5.5 `frontend/app/store/agent-pane-state/reducer.ts:732-736`, 5 → 1 lines, ~58 → ~14 tokens

Before:

```ts
// reagent P1 on PR #2378: the turn ending (however it
// ended) means whatever compaction was in flight is
// moot — a CompactionBoundary for it, if it ever
// arrives, would be stale. See the same note on
// StreamUnsubscribe above.
```

After:

```ts
// Turn over: any in-flight compaction is moot (#2378).
```

Drops the cross-reference ("See the same note on StreamUnsubscribe above") because the one-line reason stands alone.

#### 5.6 `frontend/app/store/agent-pane-state/reducer.ts:1437-1448`, 12 → 3 lines, ~158 → ~42 tokens

Before:

```ts
// reagent P2 on PR #2378 (round 11): gated the same way as
// `compacting` above. When this boundary belongs to an
// OLDER compaction that's finishing after a newer one has
// already started (preservesNewerCompaction), its
// postTokens describes a context-fill state that's already
// stale -- overwriting the live lastContextTokens with it
// would show a smaller/incorrect reading while the newer
// compaction is still confirmed in flight. The
// context-compacted event below still reports this
// boundary's own true tokens (accurate historical record
// of what that specific compaction did); only the live
// state gate changes here.
```

After:

```ts
// Keep the live reading when this boundary is older than an
// in-flight compaction; the context-compacted event below still
// reports this boundary's own tokens (#2378).
```

Keeps: the gate and the fact that the historical event still carries the true value. Drops the restated mechanics.

#### 5.7 `agentmux-srv/src/backend/blockcontroller/persistent_resume.rs:190-208`, 19 → 7 lines, ~293 → ~104 tokens

Before:

```rust
/// A session-id-bearing stdout frame arrived for this generation.
/// `sid` is the id it carried; `is_confirmed_success` is true only
/// when THIS exact frame is a terminal `result` with `is_error:false`
/// — a fully-completed, genuinely successful turn.
///
/// reagentx P0 on PR #2371: the CLI echoes back whatever `--resume`
/// sid it was given as its FIRST stdout line REGARDLESS of whether
/// that resume goes on to succeed or fail — this is true even for a
/// frame that isn't itself an error (e.g. a "system"/init frame), and
/// this first echo can arrive before the independently-scheduled
/// stderr reader has had a chance to report "No conversation found"
/// and poison it. So `sid == attempted_sid` alone is NOT proof of
/// genuine progress — it's ambiguous until EITHER a different sid
/// appears (the CLI gave up and started fresh — unambiguous) OR a
/// successful terminal result confirms the WHOLE turn actually
/// completed (also unambiguous). Only those two cases resolve
/// tracking; a same-sid echo on any other frame type is left
/// untouched, giving `ResumeUnreachable` a real chance to promote the
/// retry first if this turns out to be the doomed case.
```

After:

```rust
/// A session-id-bearing stdout frame arrived for this generation.
/// `is_confirmed_success`: this frame is a terminal `result` with `is_error:false`.
///
/// The CLI echoes the `--resume` sid first even when the resume will fail, so
/// `sid == attempted_sid` proves nothing (#2371). Only a different sid or a
/// confirmed success resolves tracking; any other same-sid frame is left for
/// `ResumeUnreachable` to act on.
```

Rust doc comment on an enum variant. Keeps: field meaning, the ambiguity, the two resolving cases, what happens to the rest. Drops the reviewer tag and the repeated "(unambiguous)" asides.

#### 5.8 `agentmux-srv/src/server/mod.rs:1283-1293`, 11 → 4 lines, ~197 → ~80 tokens

Before:

```rust
/// Whether `shell_id` is a real `view:"term"` sub-block PARENTED TO
/// `agent_block_id` — the actual safety property Codex asked for (PR
/// #3177): `shell_id` lives in the same block-id namespace as every other
/// pane, and `Layout` exposes block ids for ordinary panes, so without this
/// check an agent could point `PtyShellInput`/`PtyShellStop`/etc. at any
/// controller-backed block (another agent's own CLI pane, a human's
/// terminal pane) and inject input into it or tear it down. Framed as
/// parentage rather than "did `PtyShell` create it" (the original,
/// narrower check) specifically so it also covers the reuse case above: a
/// shell the HUMAN created via the drawer is just as legitimate a target as
/// one `PtyShell` created itself, as long as it's this agent's own pane.
```

After:

```rust
/// Whether `shell_id` is a `view:"term"` sub-block parented to `agent_block_id`.
/// Security check (#3177): block ids share one namespace, so without it an agent
/// could drive another agent's or a human's terminal. Parentage, not "created by
/// `PtyShell`", so a shell the human opened from the drawer also qualifies.
```

Security-relevant doc comment: every clause of meaning stays. Drops "the actual safety property Codex asked for" and the list of pane kinds.

#### 5.9 `frontend/app/view/agent/agent-view.tsx:1103` (stale, 1 line)

Before: `// periodic status heartbeat (persistent.rs's spawn_status_heartbeat,`
After: `// periodic status heartbeat (persistent/status.rs's spawn_status_heartbeat,`
Same length; the pointer now resolves.

## 6. Enforcement

This repo deliberately has no agent-instructions file: #3403 removed `CLAUDE.md` on the principle that agent instructions belong in each agent's own provider configuration (`SPEC_CLAUDE_MD_CONTENT_PORT_2026_09_18.md` §1). Its gates run on changed files only, because repo-wide gates were switched off in the past (`scripts/check-spec-citations.sh:19-27`). The options below follow both constraints.

| Option | What | For | Against |
|---|---|---|---|
| A. Density ratchet with an allow-list | Like `scripts/check-time-helpers.allow`: list files above 40% with their comment-line counts; fail if a count grows | Forces the backlog down | Every condensing PR edits the shared list, so parallel agent branches conflict. Adding one real invariant to a hot file fails CI. Likely to be switched off |
| A′. Density ratchet against merge-base | Like `check-docs-lifecycle.mjs`: a touched file above 40% may not raise its ratio. No list | No shared file; blames only the branch's own change | Same false-fail risk as A on legitimate guardrails |
| B. Narration gate | Fail on *added* comment lines matching `\bP[0-3]\b.{0,40}#\d{3,}`, `re-?review`, `\b(reagentx?\|ReAgent\|Codex) (P[0-3]\|review\|flagged\|caught)`, `\bround \d+\b` | Precise (30/30 spot-checked hits were narration); no backlog, since it only sees added lines; cheap | Doesn't shrink existing text; can be dodged by rewording (but rewording tends to drop the narration anyway) |
| C. Written rules | §4.3 as a "Comments" subsection of `CONTRIBUTING.md` → Style Guide. The owner can also add a short Global Memory entry pointing at it, outside the repo | Reaches humans and agents; fits the #3403 policy | Advisory; relies on being read |
| C′. `AGENTS.md` | Repo-level agent instructions | Read automatically by several agent CLIs | Reverses #3403's policy |
| D. Review-bot instruction | Tell ReAgent to ask for "constraint + `(#PR)`" in fixes, and to flag C1 violations in diffs | Catches rewording that B misses | ReAgent's prompt lives outside this repo (`.github/workflows/reagent-review.yml` runs a pinned external image); a change there, not here |

**Recommendation: B (fail) + A′ (report only) + C + D.**

- Implement B and A′ as one script, `scripts/check-comment-hygiene.mjs`, wired into `ci-pr.yml` next to `check-time-helpers.mjs`. It uses the §1.1 lexer, so strings are never mistaken for comments.
  - Default mode fails on B, and prints A′ plus C3 (added blocks over 8 lines) as `::warning`s.
  - `--report` prints the §1.3 tables.
  - `--code-equal <base>` is the §7.3 check.
- Revisit A′ as a failing check once the Phase 1 files are condensed.
- Optional: extend `check-spec-citations.sh` to flag source-file names in comments that don't resolve (C7). Exclude runtime-generated names, which is where the 326 upper bound in §1.5 comes from.

## 7. Rollout

### 7.1 Order

Rank = comment tokens × commits since 2026-08-01 (a proxy for how often agents read the file).

| # | File | Comment tok (est.) | Commits | Note |
|---|---|---:|---:|---|
| 1 | `frontend/app/view/agent/agent-view.tsx` | 16,037 | 121 | After the remaining split steps (large-file spec §3) |
| 2 | `agentmux-srv/src/server/mod.rs` | 18,554 | 81 | After its split (large-file spec item 6), per file |
| 3 | `agentmux-srv/src/backend/storage/agents.rs` | 20,736 | 39 | |
| 4 | `agentmux-srv/src/bootstrap.rs` | 12,712 | 61 | Condense the `bootstrap/*` files after the open split lands |
| 5 | `agentmux-srv/src/backend/storage/migrations.rs` | 15,897 | 40 | Migrations freeze old logic; condense comments, never code |
| 6 | `agentmux-srv/src/server/app_api/mod.rs` | 10,831 | 55 | |
| 7 | `agentmux-srv/src/server/reactive.rs` | 14,185 | 39 | |
| 8 | `agentmux-mcp/src/main.rs` | 9,633 | 48 | |
| 9 | `agentmux-srv/src/backend/blockcontroller/persistent/spawn.rs` | 13,425 | 30 | |
| 10 | `agentmux-srv/src/server/websocket.rs` | 9,510 | 41 | |

The churn column is bent by the recent split: `useAgentCommands.ts` shows 15 commits but is read with `agent-view.tsx`, and is the densest file in the repo (68%). Condense the agent-pane cluster together with row 1:

- `hooks/useAgentCommands.ts`
- `hooks/useAgentControllerStatus.ts`
- `store/agent-pane-state/reducer.ts`
- `store/agent-pane-state/types.ts`
- `components/AgentComposerStrip.tsx`

### 7.2 PR shape

- **Phase 0:** land `check-comment-hygiene.mjs` (B + A′ report + `--code-equal`) and the `CONTRIBUTING.md` rules. This stops growth before the cleanup starts.
- **Phase 1+:** one file per PR, or up to three files of under 500 lines in one directory.
  - Keep each PR to ≤400 changed comment lines, and change comments only.
  - Never mix with a move, rename or code edit.
  - Rebase just before merge; a comment-only conflict is easy to resolve, but it still stalls another agent's branch.

### 7.3 Proving no meaning was lost

1. **Code unchanged:** `node scripts/check-comment-hygiene.mjs --code-equal origin/main` strips comments from every changed file at base and head and requires identical token streams. It fails on any code change, including whitespace inside strings.
2. **Protected items survive:** the counts of `SAFETY:`, `TODO`, `FIXME`, lint/compiler directives and doctest fences are unchanged per file. The script also lists every `#NNNN` and `SPEC_…` citation dropped from the file; a reviewer confirms each was narration or a duplicate.
3. **Tests:**
   - Frontend: `npx tsc --noEmit` and `npx vitest run <dir>`.
   - Rust: `cargo check -p <crate>` and `cargo test -p <crate>`, including `--doc`, since `///` blocks hold 42 doctest fences.
   - Guard tests that grep source text: grep `*.test.ts` for the file's name first, because a grep-shaped test can match comment text.
4. **Meaning check:** a second agent lists the constraints stated in the before-text and in the after-text for each changed block. Any constraint missing from the after-list is restored.
5. **Reviewer checklist:**
   - Every deleted block was (c), (d), (e) with its pointer, or a duplicate with its pointer.
   - Every ordering or placement comment is still next to its code.
   - No constraint lost its PR or spec citation.
   - Nothing was reworded into a different claim.

## 8. Open questions

- Should test files get a lighter pass later? They are 12–15% comments, and many of those comments name the bug each test pins, which is the guardrail case.
- Should the §1.1 lexer stay in `scripts/` as the only comment counter, so the gate and `--report` can never disagree? Recommended: yes.

## 9. Phase 0 as built

Phase 0 (§7.2) is `scripts/check-comment-hygiene.mjs`, its tests (`scripts/check-comment-hygiene.test.mjs`), a CI step in the `docs` job of `ci-pr.yml`, and the "Comments" section of `CONTRIBUTING.md`. The measurement and the cost/benefit analysis are in [`REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02`](../reports/REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02.md). Differences from the design above:

- **Gate (option B).** It lexes each changed file and checks the comment text of added lines only (diff against the merge-base, working tree included), so a marker word inside a string or in code never fires. It fails on six narration patterns: a severity tag next to a PR number; "re-review"; "round N"; a review bot named next to a PR number, a severity tag, or a verdict (asked, flagged, caught, …). The severity pattern is case-sensitive (`P1`, not `p1`), because `p1` is a point variable and `#333` a colour. The severity tag is matched on either side of the PR number. Before shipping I sampled 12 random hits per pattern on `main` (60 in total) and every one was review history; the reverse-order match, added after review, found 96 more lines on `main` and all 25 I sampled were too.
- **Opt-out.** A comment containing `comment-hygiene: allow` is skipped, for the rare comment whose subject is the review bot itself. Without it the gate would have no way out of a false positive.
- **Warnings (A′ and C3).** A new `//` or `/* */` block over 8 lines, and a file above 40% comment lines whose ratio the branch raised, print as warnings and never fail. Doc comments (`///`, `//!`, `/** */`) are exempt from the block-length warning, because §4.3 C3 allows a long interface doc.
- **`--code-equal <base>`.** Compares each changed `.ts`/`.tsx`/`.rs` file against the merge-base with `<base>`. It fails on any code change (runs of whitespace outside string literals are ignored, but a line break stays significant, because automatic semicolon insertion makes `return
x` differ from `return x`; a block comment spanning lines counts as a line break), on an added or deleted source file, and on a change in the per-file count of `SAFETY:`, `TODO`, `FIXME`, doctest fences and every comment the toolchain acts on: `eslint-…`, `@ts-…`, `<reference`, `@vite-ignore` and other bundler hints, `@vitest-…` docblocks, `prettier-ignore`, coverage-ignore, JSX pragmas, `#__PURE__` and legal comments. It prints every `#NNNN` and `SPEC_…` citation the comments dropped as a notice.
- **`--report`.** Prints the §1.2 totals and the top 20 files. On `9e3e01438` it gives 164,388 non-test comment lines (27.8%) and 3,623 marker lines.
- **Lexer limits.** It is a lexer, not a parser: JSX text containing `//` or an apostrophe, and a regex literal after an unusual token, can be misread (a comment missed, never a crash). §1.1's counter had the same limits.
- **Still open from §6:** option D (ReAgent's instruction) lives outside this repo, and C7 (a gate on stale source-file names) is not built. Density (A′) stays report-only until the Phase 1 files are condensed.
