# REPORT: Is compressing the repo's code comments worth it?

**Date:** 2026-10-02
**Author:** agent3
**Baseline:** `main` @ `9e3e01438` (pulled 2026-10-02 from `agentmuxai/agentmux`)
**Status:** analysis. Step A of the recommendation (§5) shipped in #4246 with this report; §6 (dead file references, 2026-10-03) and its guard follow in a second PR. Steps B and C are not started.
**Related:** [`SPEC_CODE_COMMENT_DENSITY_AND_CONDENSING_2026_09_30`](../specs/SPEC_CODE_COMMENT_DENSITY_AND_CONDENSING_2026_09_30.md) (#4065)

The request had three questions: (1) was this done before, (2) what does outside practice say, (3) is it worth it.

---

## 0. TL;DR

1. **Prior work: a full plan exists, and nothing from it is built.** #4065 (2026-09-30) is a 646-line spec with a measurement, a taxonomy, rules, a CI-gate design and a rollout order. Since it merged there is no gate script, no `CONTRIBUTING.md` comment rules, no condensing PR, and no open PR or issue for it. Before that, the only precedent is small mechanical strips in June (#1682, #1683) and many one-off stale-comment fixes.
2. **Outside practice agrees on direction, and is thin on the numbers.** Every style guide says comments carry *why*, not *what*. The strongest empirical result is that *stale or wrong* comments cause harm, including to LLMs. I found no study showing that comment *volume* hurts, and none that measures the token cost to an agent. The case for trimming rests on reasoning, not measurement.
3. **Worth it, in part.** I re-measured and the spec's numbers hold (§2). The saving is real but concentrated: condensing the 100 densest files would cut about 5% of non-test source tokens, and condensing everything about 15%. The main risk is not cost but *rewriting meaning wrongly*, because a wrong comment is worse than a long one. **Recommendation: do the cheap prevention (Phase 0), then a measured pilot on about 10 hot files, and decide on the rest from the pilot's numbers. Don't start a repo-wide campaign justified by token cost alone.** (§5)

---

## 1. Was work done in the past?

### 1.1 The existing spec (#4065)

`docs/specs/SPEC_CODE_COMMENT_DENSITY_AND_CONDENSING_2026_09_30.md`, by Maricon@charlie, status "proposed — nothing here is built". It already contains:

- a measurement (comments 27.7% of lines, ~37% of characters in non-test source);
- a taxonomy of eight comment kinds, with a 100-block sample (85% worth keeping, ~14% narration, restating, essay or stale);
- eight rules (C1–C8) and nine worked before/after rewrites;
- a CI-gate design (option B: fail on review-history markers in *added* lines; option A′: density report only);
- a rollout: one file per PR, comment-only, with a "code token stream identical" check.

It also revises the earlier "Keep the prose" rule from the large-file spec: keep the *meaning*, not the prose.

### 1.2 What has been built since

I checked by searching the tree and git history, not by trusting the spec's status line. The table is as of `9e3e01438`; the PR carrying this report adds the first two rows (spec §9).

| Thing the spec proposes | Present on `main`? |
|---|---|
| `scripts/check-comment-hygiene.mjs` (gate, report, `--code-equal`) | No (no file; no reference in `scripts/`, `.github/`, `package.json`, `Taskfile.yml`) |
| "Comments" rules in `CONTRIBUTING.md` | No (no mention of comments there) |
| Any condensing PR since 2026-09-30 | No (135 commits since, none about this) |
| Open PR or issue | No (GitHub search finds only #4065 itself; 3 open PRs repo-wide, none related) |

### 1.3 Earlier, smaller precedents (all merged)

| Commit | Date | What | Size |
|---|---|---|---|
| #1682 `e35f5ad09` | 2026-06-22 | removed 211 vestigial command-label comments in rpc-api | 2 files, −208 net lines |
| #1683 `404470d8b` | 2026-06-22 | stripped phase-migration comments and trivial field docs (Rust) | 9 files, +68 / −176 |
| `ca99d416f` | — | stripped PR-citation comments from `bash_wrap` source | 1 file, ±9 lines |
| a dozen more | various | stale or garbled comment fixes after a refactor | 1–3 files each |

All were mechanical, targeted at one *pattern* (labels, phase tags, PR citations), and tiny against a ~164k-line stock. None reduced density measurably. They show the pattern-based approach is cheap and was accepted without trouble.

---

## 2. Re-measurement (does the spec's baseline hold?)

I wrote an independent lexer (`scratch/measure.mjs` in my workspace, not committed) and ran it on `9e3e01438`. It skips strings, template literals, Rust raw strings and char literals, and handles nested block comments. Excluded: tests, the generated `frontend/types/rpc/` bindings, `splash_font.rs`. Tokens are chars / 4, so treat them as ±20%.

| Measure (non-test TS + Rust) | Spec (09-30, `cee674b6f`) | Mine (10-02, `9e3e01438`) |
|---|---:|---:|
| Files | 1,566 | 1,699 |
| Lines | 560,583 | 591,387 |
| Comment-only lines | 155,071 (27.7%) | 164,388 (27.8%) |
| Comment share of characters | 36.8% | 36.6% |
| Review-history marker lines | 3,636 | 3,474 (2.1%) |
| Lines in comment blocks of 10+ | "nearly half" | 44.7% (4,165 blocks, 73k lines) |

The totals agree within 1–2%, and the ratio is flat. (My file count is higher because the `crates/` move and new files landed since; the spec's marker regex differs slightly from mine.)

New facts the spec doesn't have:

- **Doc comments are 54% of comment lines.** `///` and `//!` are 59,082 and JSDoc is 29,782 of 164,388; `//` is 74,555. Half the stock is interface documentation that rustdoc and editors surface, so it is the most careful part to touch.
- **Concentration.** Comment characters by file rank:

  | Condense the top… | Share of all comment tokens | Comment tokens | Saving at a 40% cut | As share of all non-test source tokens |
  |---:|---:|---:|---:|---:|
  | 10 files | 6.8% | ~163k | ~65k | 1.0% |
  | 25 | 14.5% | ~347k | ~139k | 2.1% |
  | 50 | 23.3% | ~558k | ~223k | 3.4% |
  | 100 | 35.2% | ~844k | ~338k | 5.2% |
  | 200 | 50.2% | ~1.2M | ~482k | 7.4% |
  | all 1,699 | 100% | ~2.4M | ~960k | 14.7% |

  Non-test source is about 6.5M tokens. The 40% cut is the middle of the spec's 35–50% range; it is the spec's estimate and I did not execute it. Marker lines are concentrated too: the top 50 files hold 40% of them.
- **Regrowth is proportional, not accelerating.** In the last 30 days, non-test additions were 118,682 non-blank lines, of which 28,325 (23.9%) were comments, against 27.8% in the stock. Removals were 9,934 comment lines. Review-marker lines were 1.4% of added comments (398), below the 2.1% stock rate. So new code is slightly *less* commented than old, and narration is not the main growth.
- **Churn is high**: 1,216 non-merge commits in 30 days (about 40/day), but only 3 open PRs right now, because PRs land fast.

### 2.1 My own spot check

I drew 8 random blocks of 10+ lines (seeded; script `scratch/sample.mjs`). Rough reading, one reader, not a measurement:

- **Keep nearly whole (2):** `leases.rs:589` (explains why the lock file is never deleted: the unlink race), `model-turn-signal.ts:4`.
- **Condense by a third to a half (4):** `ToolOverlayLog.tsx:169` (carries "the old code pinned once…" history), `mcp_servers.rs:99` (`Codex, PR #3152`, `ReAgent, PR #3152`), `import.rs:363` (`round 8 / round 9`), `identities.rs:4` (retired-layer and extraction history).
- **Light trim (2):** `commands/types.ts:4`, `window_snap.rs:38`.

That matches the spec's picture: most content is real, and the waste is mostly length plus review history, not whole blocks to delete.

---

## 3. What outside practice says

> [!NOTE]
> I did not read these sources myself. A research sub-agent gathered them with WebSearch/WebFetch, and it saw only abstracts or summaries for several of the papers. Evidence labels are its. Several are small pilots or preprints. Verify a source before quoting it in a PR.

### 3.1 Guidance: opinion, but unanimous

- **Ousterhout, *A Philosophy of Software Design*, ch. 13:** comments describe what the code can't. "Comment repeats code" is a red flag.
- **Clean Code (Martin):** comments are "a failure to express ourselves in code"; redundant ones "collect lies". The strictest view.
- **Linux kernel coding style:** say WHAT and maybe WHY, never HOW; no boilerplate kernel-doc that repeats the signature. https://docs.kernel.org/process/coding-style.html
- **Google C++/Python guides:** don't state the obvious; explain why. https://google.github.io/styleguide/pyguide.html
- **Go doc comments, Rust API Guidelines:** document what a *caller* needs; Rust adds `# Safety`, `# Errors`, `# Panics` sections. https://rust-lang.github.io/api-guidelines/documentation.html

These map onto the spec's kinds (d) restating and (g) stale. None of them argue for a density *target*.

### 3.2 Empirical work

- **Strong:** stale or inconsistent comments are a real defect source. Wen et al. (ICPC 2019, 1.3 billion AST-level changes) catalogued the rot, and a later 32-project study found inconsistent comment changes about 1.5× more likely to introduce a bug (odds ratio 1.52 over 7 days, fading over time). https://www.inf.usi.ch/lanza/Downloads/Wen2019a.pdf, https://arxiv.org/html/2409.10781v1
- **No evidence that comment *volume* matters.** The large density study (5,229 projects, 18.7% average) treats density as a maintainability proxy with no defect link.
- **LLM effects (moderate, preprints and pilots):**
  - Comments help bug-fixing modestly, and are not dominant (https://arxiv.org/pdf/2601.23059).
  - *Wrong* comments mislead models more than *missing* ones do. A small pilot reports accuracy falling about 12 points with false comments (https://pith.science/paper/2506.11007).
  - One preprint finds removing comments improved RepoQA for most models, but not all (https://arxiv.org/pdf/2512.16790).
  - Likely-LLM comments skew to low-information "Explanation" and "Meta" categories (https://arxiv.org/html/2607.01867v1).
- **Gap:** no agentic-task benchmark (SWE-bench style) compares stripped against kept comments, and nobody measures the token cost of comments in agent context. That cost is logical, not measured. The same is true of the spec's central claim here.

### 3.3 Practice in agent tooling

- Claude Code's own system prompt says: default to no comments; add one only when the WHY is non-obvious. https://github.com/Piebald-AI/claude-code-system-prompts/blob/main/system-prompts/system-prompt-comment-why-only-guidance.md
- Anthropic's best-practices page warns that long instruction files get ignored and that hooks, not prose, are deterministic. https://code.claude.com/docs/en/best-practices
- Users report agents ignoring "no comments" instructions (https://github.com/anthropics/claude-code/issues/65961). So **a gate beats a rule in a file.**

### 3.4 Mass-cleanup precedent and tooling

- **`uncomment`** (tree-sitter, supports Rust and TS): keeps doc comments, TODO/FIXME, linter directives and `~keep` markers; has `--check`. https://github.com/Goldziher/uncomment. It strips; it does not condense, so it fits the narrow "delete restating comments" tier and nothing more.
- Public cleanup PRs (e.g. vendua #168, 250 files by parallel agents) verified safety by stripping comments on both sides and requiring zero non-comment diff. The spec's `--code-equal` is the same idea.
- Clippy has `undocumented_unsafe_blocks` (keep every `// SAFETY:`) but **no lint for comment volume**; ESLint has none either.
- **Blame churn:** use a `.git-blame-ignore-revs` file (Git 2.23+; GitHub's blame honours it). With squash merges the hash exists only after merge, so it needs a follow-up commit. Keep the cleanup commit purely mechanical, because ignoring it also ignores any real fix folded in.

---

## 4. Is it worth it? Analysis

### 4.1 What it would buy

| Benefit | Strength | Notes |
|---|---|---|
| **Fewer wrong comments** | **Strong** (best-evidenced harm) | 326 comment references to file names that no longer exist (spec §1.5); at least two stale pointers found in `agent-view.tsx`; "wait, wcore appends" left in `tab_move.rs`. Condensing *forces* a re-read against current code, so it doubles as a staleness sweep. |
| **Less context per read** | Moderate, unmeasured | `useAgentCommands.ts` is ~27k tokens with ~18k of comments; a 40% cut saves ~7k per full read (27%). Files over 2,000 lines also need two `Read` calls. |
| **Less distraction / copy-forward** | Weak, plausible | Agents imitate the density and narration they read, and 3,474 lines of review history are in the tree for them to copy. |
| **Human readability** | Moderate | Long blocks with "(thirty-fifth re-review)" are hard to scan. |
| **Token cost ($)** | **Small** | At ~40 tasks a day and a few files per task, the saving is on the order of 1M input tokens a day at the very most, and most of that is cache-priced. This is not the reason to do it. |

The honest summary: the *accuracy* argument is evidence-backed, the *context* argument is sound but unmeasured, and the *money* argument is weak.

### 4.2 What it would cost and risk

| Cost / risk | Size | Mitigation |
|---|---|---|
| **A rewrite changes meaning** | The main risk. A wrong comment is worse than a long one (§3.2). `--code-equal` proves *code* is unchanged, not that *meaning* survived. | Condense, don't delete, constraints; keep every `(#NNNN)`; a second reader lists constraints before/after (spec §7.3); start with the mechanical tier. |
| **Reviewer load** | ~150 PRs to reach the top 100 files at ≤400 comment lines each (my estimate: ~600 comment lines per file on average) | Larger PRs per directory for low-risk tiers; ReAgent + Codex already review every PR. |
| **Merge conflicts** | Hot files change about every 12 hours (`agent-view.tsx`: 121 commits since 08-01). Today only 3 PRs are open. | Do hot files fast, rebase just before merge, announce on the bus first. |
| **Lost guardrail, bug returns** | Real; the spec cites `useAgentCommands.ts:697-699`. | Never delete invariants or why-nots; a named regression test where a comment is the only guard. |
| **Blame noise** | One-time | `.git-blame-ignore-revs`, added after merge. |
| **Rework cost if the gate is too strict** | Past repo-wide gates were switched off (spec §6) | Gate only on *added* lines and review markers; density report-only. |
| **Opportunity cost** | Agent time on non-feature work | Pilot first; stop if numbers are poor. |

### 4.3 Where I differ from, or add to, the spec

1. **The spec's headline "13–18% of all tokens" is a ceiling.** It needs every file condensed. At 100 files it is about 5% (§2). The right unit is *tokens saved per file read*, weighted by how often agents read the file, and the spec's churn column is only a proxy for that.
2. **Nothing validates the 35–50% figure.** It rests on one author's rewrite of 495 lines ("optimistic", per the spec). The pilot should replace it with a measured number.
3. **Tier the work by risk.** The spec treats all condensing as one activity. I'd split it:
   - *Tier 0, mechanical:* delete review-marker narration ("`reagentx P1 on PR #2338 (thirty-fifth re-review)`") while keeping the `(#NNNN)`, and fix the dead file references. About 3,474 lines in 664 files, and the top 50 files hold 40%.
   - *Tier 1, judgment:* condense long why-comments in the hot files.
   - *Tier 2, structural:* move essays (>8 lines) to `docs/`.
   Tier 0 is low-risk and most of the "wrong comment" benefit. Tiers 1–2 carry the meaning risk.
4. **The doc-comment half (54%) needs a different bar.** Rust `///` and JSDoc feed rustdoc and editor hovers, and the Rust API Guidelines ask for them. I would not condense public interface docs in the pilot; start with `//` and internal blocks.
5. **Prevention is cheaper than the stock and already justified** (the spec's option B). It is a few hours of work and stops about 400 narration lines a month. Because agents ignore prose rules (§3.3), it should be a gate.

---

## 5. Recommendation

**Do it in three steps, each a separate go/no-go. Do not start the campaign.**

| Step | What | Size | Go/no-go signal |
|---|---|---|---|
| **A. Phase 0: prevention** | Land the spec's `check-comment-hygiene.mjs` (review-marker gate on added lines, density report-only, `--code-equal`), plus a short "Comments" section in `CONTRIBUTING.md`. **Done in this PR.** | Small | Gate has no false positives on a sample: 12 random hits per pattern on `main`, 60 in all, were every one review history. Watch the first week of real PRs for others. |
| **B. Pilot: ~10 hot files** | Tier 0 + Tier 1 on the agent-pane cluster (`useAgentCommands.ts`, `AgentComposerStrip.tsx`, `store/agent-pane-state/types.ts`, `useAgentControllerStatus.ts`, `reducer.ts`) and 3–5 of the Rust hot files (`storage/agents.rs`, `server/reactive.rs`, `persistent/spawn.rs`). One file per PR, comment-only, code-equal check green. | ~10 PRs | See below. |
| **C. Scale or stop** | If the pilot passes, continue down the ranked list (top 50 ≈ 23% of comment tokens). Otherwise stop at Tier 0. | — | — |

**Pilot success criteria** (decide before starting, so the result isn't argued after):

- measured comment-token cut per file (replaces the 35–50% guess; a ≥30% cut on the files touched is the bar);
- code-equal check passes on every PR, and the full test suite is unchanged;
- no reviewer finds a lost constraint; if one does, count it and treat two or more as a stop;
- review rounds per PR no worse than the repo median;
- conflict count: how many pilot PRs needed a rebase for a code change in the same file.

**Not recommended:**

- A blanket `uncomment`-style strip. It deletes why-comments along with narration, and the repo's own guardrails (`useAgentCommands.ts:697-699`-style) are exactly what it would lose.
- A density-ratio CI gate that fails. It would punish adding one legitimate invariant to a hot file, and past repo-wide gates here were switched off.
- Condensing during a move-only refactor. The spec's rule stands.

**Honest limits of this report:**

- No outside source measures the benefit to an agent, so the benefit side is partly reasoning.
- I verified the *baseline* and spot-checked 8 blocks; I did not trial a condense, so the spec's 35–50% remains an estimate.
- GitHub search was unauthenticated and only covers the open PRs and issue titles it returned; local git covers merged work.
- Token counts are chars / 4, about ±20%.

---

## 6. Dead file references: full triage (2026-10-03)

§4.1 named accuracy as the best-evidenced benefit. This section measures it for one concrete kind of rot, comments naming a file that does not exist, and sets the tiers of the C7 guard (spec §9.1) from the result.

### 6.1 Method

- `node scripts/check-comment-hygiene.mjs --dead-refs --json` on `85c7a0b57` listed every file name (ending `.rs .ts .tsx .mjs .cjs .sh .ps1 .scss .css .md`) in a `.ts`/`.tsx`/`.rs` comment that no tracked file matches by full path, path suffix or basename. Runtime names AgentMux writes (`CLAUDE.md`, …) were excluded up front. Result: 417 occurrences of 177 names in 251 files.
- Every name was checked, not sampled. Four sub-agents each took a quarter. For each occurrence they read the comment in context and used git history (`--diff-filter=DR` renames and deletes, `git log -S`, `git grep` for the symbol the comment names) to give a verdict, the file it should name now, and the evidence.
- I re-checked a handful by hand. Two of five held exactly, one target was half right (`acp.rs:818` names a publish step that lives in `persistent/status.rs`, not `persistent/input.rs`), and one "never existed" claim was wrong (`agentmux-cef/src/app.rs` did exist). So the verdicts are good enough to tune a guard, but every fix is re-read in context before it is applied.

### 6.2 Results

| Verdict | Occurrences |
|---|---:|
| Stale: the file moved or was split, and the comment should name the new one | 186 |
| Stale: the file and what the comment describes are gone | 34 |
| Wrong name: never existed under that name; the intended file exists | 12 |
| Cited doc never committed under any name (checker: unsure) | 9 |
| **Genuinely broken, total** | **241 (58%) in 156 files** |
| Deliberate history ("ported from `src-tauri/…`", "split out of …") | 62 |
| Not a file (`item.ts` is a property; `a.ts/b.ts` is a list) | 51 |
| Example or test-fixture name | 30 |
| A file in another repo | 24 |
| A runtime file | 9 |

- **Cause:** 176 of the 241 point at a file that a rename, split or deletion removed. The top names are `persistent.rs` (43), `bootstrap.rs` (24), `browser_panes.rs` (10), `app.rs`, `app_api.rs` and `identity/resolver.rs` (9 each). Nothing updated the pointers when the module split.
- **The other 65 never existed under that name.** 12 cited doc names were never committed anywhere (often a date written `2026-05-14` instead of `2026_05_14`, or a spec that was planned and never written). `mstore.rs` comes from a bulk `wstore`→`mstore` rename (#3287) rewriting a reference that was already stale.
- **Stale beyond the name:** some comments name the wrong file *and* describe behaviour that has since changed (the `bundle_export.rs` notes on `parse_json_field_or_warn`). A guard catches the name; the sentence still needs a reader.

### 6.3 What it means for the guard

Precision per tier, after splitting `a.ts/b.ts` lists and treating sibling repos, `src-tauri/` and Copilot's runtime instruction files as foreign (that removed 31 reports, none of them stale):

| Tier | Reported | Genuinely broken | Guard |
|---|---:|---:|---|
| Repo-rooted path (`crates/…`, `frontend/…`) | 22 | 95% | error on added lines |
| Doc name (`SPEC_…`, `docs/…`) | 32 | 72% | error on added lines; the message says how to cite another repo or mark an example |
| Partial path (`identity/resolver.rs`) | 67 | 58% | warning |
| Bare name (`persistent.rs`) | 265 | 59% | warning |

The rename trigger needs no precision estimate: it only fires on a name the branch itself just removed. On a trial rename of `pane_env.rs` it flagged the two comments naming it, and skipped the third mention, which is a path string in a test's code.

### 6.4 Backlog

The scan was later widened to comments in JavaScript-family files and stylesheets (spec §9.1), which adds a few more, e.g. `muxlog.mjs` naming `bootstrap.rs` and stylesheets citing never-committed specs; they are fixed the same way.

The 241 broken references stay until fixed. 196 have a verified replacement path; 45 need rewording (the target is gone, or the cited doc never existed). They are fixed in a separate comment-only PR, proven with `--code-equal`, so the guard PR stays reviewable on its own.

## 7. Trial: one file condensed (2026-10-03)

§4.3 and §5 recommended condensing one file first and measuring it against criteria set in advance. The file was `frontend/app/view/agent/hooks/useAgentCommands.ts`, the densest file in the repo (68% comment lines) and the one with the most review narration (111 lines).

### 7.1 Method

1. One agent condensed the comments under spec §4.3/§5 (keep every constraint, delete narration and restating, explain a rule once and point to it elsewhere).
2. A second, independent agent compared every changed block before and after (49 hunks, ~75 blocks), opened every pointer the new text uses, and listed anything lost. It found no lost constraint, changed claim, bad pointer or dropped citation, and 5 minor omissions. All 5 were restored; one of them also corrected a claim that was already stale before the edit (`initiatesTurn`).
3. Mechanical proof: `--code-equal` (code identical), the file's 65 tests, `tsc --noEmit`, and the narration and reference gates.

### 7.2 Result against the §5 criteria

| Criterion | Bar | Result |
|---|---|---|
| Comment-token cut on the file | ≥30% | **59%** (~18.4k → ~7.6k tokens) |
| Code-equal on the PR | green | green (code identical) |
| Tests | unchanged | 65 / 65 pass; `tsc` clean |
| Lost constraints found by review | 0 (2+ = stop) | 0 (independent check), 5 minor omissions restored |
| Review rounds, merge conflicts | repo median | measured on the PR |

- **Per read:** the whole file goes from ~27.6k to ~14.8k tokens (−46%) and from 1,926 to 1,164 lines.
- **Accuracy:** the pass fixed several stale pointers on the way (e.g. `trackTurnJustEnded` and `wasTurnActive` now point to `turn-confirmation.ts`, where they live).
- **Versus the estimate:** the spec estimated 35–50% from one person's rewrite of a random sample. This file came out above that range, as expected for the file with the most narration and duplication; the random-sample figure remains the better guess for an average file.

### 7.3 What it means

The trial passes every criterion fixed in advance. The cost was one condensing pass plus one independent review pass, about 15 minutes of agent time, and the two-pass method (rewrite, then independent before/after check) is what made a 59% cut safe to take. Continuing down the §7.1 ranking is justified; the next candidates are the rest of the agent-pane cluster (`AgentComposerStrip.tsx`, `store/agent-pane-state/types.ts`, `useAgentControllerStatus.ts`, `reducer.ts`), one file per PR, same method.

## Appendix A: top 15 non-test files by comment characters (`9e3e01438`)

Comment tokens are chars / 4. "Marker lines" are lines matching a review-history regex (severity tag near a PR number, re-review, round N, or a review-bot name).

| File | Lines | Comment lines | Line ratio | Comment tok | Comment share of chars | Marker lines |
|---|---:|---:|---:|---:|---:|---:|
| `crates/srv/src/backend/storage/agents.rs` | 3,846 | 1,405 | 37% | ~21k | 45% | 46 |
| `crates/bashwrap/src/bash_wrap.rs` | 3,627 | 1,341 | 37% | ~20.3k | 48% | 31 |
| `frontend/app/view/agent/hooks/useAgentCommands.ts` | 1,927 | 1,310 | 68% | ~18.4k | 67% | 111 |
| `crates/srv/src/backend/storage/migrations.rs` | 3,032 | 993 | 33% | ~16k | 40% | 16 |
| `crates/srv/src/backend/lan_discovery.rs` | 3,088 | 972 | 31% | ~15.1k | 42% | 20 |
| `crates/srv/src/server/reactive.rs` | 2,766 | 958 | 35% | ~15k | 47% | 33 |
| `crates/srv/src/backend/blockcontroller/shell/lifecycle.rs` | 2,829 | 996 | 35% | ~14.8k | 42% | 30 |
| `crates/common/src/ipc.rs` | 2,269 | 1,055 | 46% | ~14.3k | 60% | 11 |
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | 1,529 | 930 | 61% | ~14.2k | 65% | 28 |
| `crates/cef/src/commands/window_pool.rs` | 2,215 | 923 | 42% | ~14k | 55% | 25 |
| `frontend/app/view/swarm/swarm-model.ts` | 2,138 | 902 | 42% | ~14k | 52% | 20 |
| `crates/srv/src/backend/blockcontroller/persistent/mod.rs` | 1,695 | 886 | 52% | ~13.9k | 64% | 19 |
| `crates/srv/src/backend/agent_config.rs` | 3,190 | 877 | 27% | ~13.7k | 35% | 39 |
| `crates/srv/src/backend/blockcontroller/persistent/spawn.rs` | 2,213 | 1,048 | 47% | ~13.7k | 40% | 54 |
| `crates/srv/src/backend/bundle_import.rs` | 2,851 | 852 | 30% | ~13.2k | 38% | 99 |

`agent-view.tsx` has since shrunk to 1,497 lines (672 comment lines, ~10.7k comment tokens) as its split progressed; `store/agent-pane-state/types.ts` is 71% comments (862 of 1,218 lines).

## Appendix B: reproducing the numbers

```
node scratch/measure.mjs <repo-root> <out.json>   # aggregate + per-file table
node scratch/sample.mjs  <repo-root> <out.json>   # seeded sample of 10+ line blocks
```

Both scripts live in the agent3 workspace, are not committed, and run in about 2 seconds. The spec proposes committing a lexer as the gate's counter; mine is a candidate starting point.
