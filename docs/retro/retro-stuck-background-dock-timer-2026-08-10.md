# Retro: Stuck Background-Task Dock Timers (Issue #2518)

**Date:** 2026-08-10
**Severity:** Medium — no data loss, but the Activity Dock silently misrepresented finished work as still running, indefinitely, for the majority of backgrounded calls
**Observed by:** agenta (Claude agent), asked directly by the user: "in your running instance there are 17 tasks with timers that have been going for hours. are you able to introspect?"
**Related specs:** `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md`, `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md`, `SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md`
**Related retros:** none directly, but PR #2432 (muxspect `dock`/`dock clear`) is the sibling diagnostic tool this bug turned out to be invisible to

---

## TL;DR

The user pointed at 17 dock rows with hours-old, still-climbing timers and asked if the agent could introspect them from the inside. Cross-referencing the running session's own transcript against the harness's async completion markers showed all 17 had already finished — only 6 had received the `<task-notification>` the dock waits for; the other 11 had resolved synchronously and would never get one. Root cause: `tool-adapter.ts`'s `isAcceptedBackgroundLaunch` trusted `params.run_in_background === true` as proof a call was genuinely detached, but the harness decides per-call whether to actually detach — a command that finishes fast enough returns its real output directly instead, and gets misclassified as "launch accepted, wait forever." Fixed in #2519 (two review rounds — a codex-caught fallback-shape bug and a reagent-caught branch-staleness P0), which eliminates this exact bug class going forward — a fast-finishing call is now correctly excluded from the dock's "accepted background launch" classification, so it never gets a `running`-forever row in the first place. Follow-up #2520 gives `muxspect dock` visibility it structurally lacked either way: a `bg` column for genuinely-accepted launches, so a *different*, still-open risk (a real detached process whose completion notification never arrives) is visible server-side for its first hour of life, instead of requiring transcript introspection by hand (four review rounds — a scrub-path gap, a false-positive-tagging mistake that reintroduced #2519's own root cause one layer over, and two codex corrections to this retro's own claims about what the column detects, including that the one-hour cache eviction means it can't help with an hours-old row like the ones that motivated this very retro).

---

## What Happened

### The question

Mid-session, unrelated to what the agent was actively doing (driving a different PR through review), the user asked:

> "in your running instance there are 17 tasks with timers that have been going for hours. are you able to introspect?"

This was a real, current observation of the Activity Dock in the AgentMux instance the agent was running inside — not a hypothetical.

### Finding the 17

`muxspect list`/`describe` only see backend `ProcessBroker` state — they don't see the dock at all for exactly the class of entry in question. The agent instead read its own session transcript directly (the JSONL file Claude Code persists) and extracted every `Bash`/`PowerShell` tool call with `run_in_background: true` in its input. There were exactly 17, matching the user's count.

Cross-referencing each `tool_use_id` against the transcript for two signals:
- A `tool_result` (does Claude Code think the call resolved at all — yes, for all 17)
- A `<task-notification>` system message naming that `tool_use_id` (does the harness's own async-completion signal exist — yes for only 6 of 17)

Inspecting the actual `tool_result` content for one of the 11 "no notification" calls showed the harness's real output directly:

```
<exited 0 in 13.38s>
707M    /c/Users/area54/.cargo
1.6G    /c/Users/area54/.rustup
...
```

— not the acceptance message a genuinely detached launch gets:

```
Command running in background with ID: bdtzzfgo3. Output is being written to: ...
You will be notified when it completes. To check interim output, use Read on that file path.
```

Both shapes are legal responses to a `run_in_background: true` request — the harness itself decides, per call, whether the command finishes fast enough to just return synchronously instead of actually detaching it.

### Root cause

`frontend/app/view/agent/activity/tool-adapter.ts`'s `isAcceptedBackgroundLaunch` (added for issue #2490, the original "give backgrounded calls a dock row" feature) checked only:

```ts
(n.params as BashParams | undefined)?.run_in_background === true && n.status === "success"
```

Both of those are true for **either** result shape above. Once classified as an accepted launch, `toolActivities()` overrides the dock row's status to `running` and keeps it there until a `<task-notification>` for that exact `tool_use_id` arrives (see `backgroundCompletions()`). For the fast-finishing majority, that notification will never come — there is no detached process to report on — so the row sticks at `running` with `endedAt: undefined` forever, timer climbing on every render.

The original design (`BashParams.run_in_background`'s own doc comment) assumed the acceptance message and the flag always went together. Live traffic contradicted that assumption in the majority case: **11 of the 17** backgrounded calls in this one session were fast-finishing.

---

## The Fix — PR #2519

`isAcceptedBackgroundLaunch` now also requires the tool_result's own text to start with the literal acceptance prefix, `"Command running in background with ID:"` — not just the params flag and a terminal status.

**Review round 1** — two independent findings on the same commit:
- **codex, P1:** the fix checked `result.stdout` only. `claude-translator.ts`'s `buildToolResults` falls back to a plain `{ content: string }` shape (instead of the structured `{ stdout, stderr, interrupted }` sibling) when Claude omits a terminal-shaped `tool_use_result` or returns multiple `tool_result` blocks. A stdout-only check would reject a **genuinely** detached launch whose acceptance text arrived that way — dropping it from the dock entirely instead of just misclassifying it, which is worse than the bug being fixed. Fixed by adding a `resultText()` helper that checks both fields.
- **reagent, P0:** the branch was stacked on top of an unrelated, already-merged PR's own branch (`agenta/close-2368-held-error-line-regression-test`) rather than being cut fresh from current `origin/main` — which had since advanced through a `v0.55.3` release. Merging as-is would have reverted `package.json`/`Cargo.toml`/`VERSION_HISTORY.md` back to `0.55.2` and duplicated an already-merged test, risking a duplicate-definition conflict. Fixed by rebuilding the branch from a fresh `origin/main` checkout and cherry-picking just the two genuinely-new commits, then force-pushing over the same PR branch (see "Process lesson" below).

**Review round 2:** clean on both gates. Merged as `8e70f9ddb` (the squash-merge commit — `9d21c2b4c` was the branch's own pre-merge head, not the commit that actually landed on `main`).

---

## The Follow-up — PR #2520 (muxspect introspection)

Diagnosing this bug required an ad-hoc Node script cross-referencing the raw transcript by hand. `muxspect dock` (PR #2432) is the existing tool for exactly this class of "stuck dock entry" problem, and it could see none of this: it only ever reads the *raw* `ToolNode.status`, which for an accepted background launch goes terminal (`success`) within about a second. The `stuck` heuristic (`status == "running" && ...`) can structurally never fire for this category, no matter how long the actual dock row has been showing `running`, because that reclassification happens entirely client-side in `tool-adapter.ts` — the server never sees it.

The fix threads `params.run_in_background` through the existing `docknodestatus` push into `DockNodeSnapshot`/`DockNodeView`, rendered as a new `bg` column.

**What `bg` does and doesn't detect (codex P2 x2, both caught reviewing this retro):** `bg` is `true` only when `isAcceptedBackgroundLaunch` confirms the launch — the exact 11 calls that caused this incident report `bg: false`, correctly, since they were never accepted; #2519 already stops the dock from ever showing them as `running` in the first place, so there is nothing left for this column to catch about *that* bug. What `bg` actually adds is visibility into a different, still-open risk: a **genuinely accepted** launch (`bg: true`) whose `<task-notification>` never arrives — a real orphaned/leaked process, or a future regression in the notification-delivery path itself. The server still cannot tell "still legitimately running" apart from "notification was missed" for a `bg: true` row; that would need the srv to observe notification delivery too, which #2520 deliberately didn't attempt (see the Longer-term follow-up below).

That visibility is also **short-lived, not indefinite**: `DockSnapshotCache::get` evicts any snapshot older than `MAX_NODE_AGE_MS` (one hour) regardless of status, and a genuinely-accepted node gets exactly one push — the acceptance ack — with nothing to refresh it afterward (the eventual notification resolves the dock display purely client-side; it never mutates the underlying `ToolNode`, so there's no later "status changed" event to push). Concretely: for the **hours-old** dock rows this retro's own reporting session hit, by the time anyone thinks to check `muxspect dock`, the one-hour-old snapshot has already been silently evicted — the row shows up as nothing at all, not even a stale `bg: true` to investigate. `bg` helps within its first hour of life; past that, this retro's own motivating symptom is invisible to it too, same as before #2520.

**Review round 1 — reagent, P1:** the streaming push site (`pushDockNodeStatus`) forwarded the field, but the *scrub-path* push site (`pushResolvedDockNodes`, used by `SessionEnd`/`HistoryLoaded`/`HistoryRestored`/`ScrubOrphanedInProgress`) didn't. Since `DockSnapshotCache::push_delta` fully overwrites a node's cached snapshot per push (not a partial merge), an orphaned background node resolved via that path would silently blank an earlier `run_in_background: true` back to `undefined` — erasing the signal for exactly the orphaned/stuck-launch class the column exists to flag. Fixed by threading the field through `resolvedToolNodes` too (the full `ToolNode` with `params` is still in scope at the point the narrower scrub projection is built).

**Review round 2 — reagent, P1 (escalating a codex P2 from round 1 that had gone unaddressed):** both push sites forwarded the *raw* `params.run_in_background` flag — true for every call that merely *requested* backgrounding, most of which the harness resolves synchronously. This is the exact same mistake #2519 fixed in the dock's own display logic, reintroduced one layer over in the new diagnostic column: tagging **11 of 17** calls `bg` when only 6 were genuinely accepted would have made the column noisy on the common case, undermining its entire purpose. Fixed by exporting `isAcceptedBackgroundLaunch` from `tool-adapter.ts` and reusing it at both push sites instead of duplicating (and drifting from) the classification logic. In the scrub-path branch specifically this is now structurally always `undefined` — documented inline so it doesn't read as dead code: a node still stuck at raw status `running` can never have been confirmed as an accepted background launch (that confirmation requires status `success`), so reporting nothing there is the accurate answer, not a regression.

**Review round 3:** clean on both gates. Merged as `ca52e7400`.

---

## Process lesson: verify the branch before you push, not after reagent tells you

PR #2519's P0 happened because a new branch was created via `git checkout -b` right after a prior PR's branch was still checked out, without first confirming `HEAD` was actually on a synced `origin/main`. `git checkout main` had silently failed twice in the same session (a worktree elsewhere already had `main` checked out) and the failure wasn't caught — the subsequent `git checkout -b` branched from whatever was checked out instead, which was the previous PR's own tip.

The fix pattern, worth repeating whenever a new branch is cut mid-session:

```bash
git fetch origin main
git merge-base --is-ancestor origin/main HEAD && echo "OK: based on current main" || echo "STALE"
```

`git log origin/main..HEAD --oneline` is **not** sufficient on its own here (caught by codex reviewing this very retro, P2): it lists commits reachable from `HEAD` but not from `origin/main`, which is exactly this PR's own new commits *even when the base is stale* — a stale base's older history is still an ancestor of the current `origin/main` in the common case (a fast-forward release), so it's silently excluded from the diff either way. The count looks identical whether the branch is fresh or stale; it doesn't prove anything about the base. `git merge-base --is-ancestor origin/main HEAD` checks ancestry directly — it only succeeds if every commit on current `origin/main` is actually in `HEAD`'s history, which is the actual property "not stale" means.

If it fails, rebuild from a fresh branch and cherry-pick just the new commits rather than trying to rebase through it — cherry-pick + force-push to the same PR branch name preserves the open PR and its review thread.

This exact check is now baked into how this session drives every subsequent PR.

---

## What Worked

- Reading the running session's *own* transcript directly was the fastest path to ground truth — no speculation about what the harness "should" do, just what it actually did for all 17 real calls in this session.
- Both fixes shipped with regression tests that spawn/construct the exact failure shape (a real `git --version` process for the backend test in #2511's earlier adjacent work; fixture `ToolNode`s with both result shapes for #2519/#2520) rather than only asserting on the classification function's inputs in the abstract.
- Review caught bugs manual testing alone would likely have missed at every layer: a fallback-shape gap and a branch-staleness P0 on #2519, a scrub-path gap and a false-positive-tagging gap on #2520, and — reviewing this retro itself — a wrong commit hash, an unverified staleness-check command, and two overclaims about what the new `bg` column actually detects (including its one-hour cache lifetime). None of the code-level findings show up unless you specifically construct the harness's less-common response shapes; the doc-level ones are a reminder that a retro's own claims need the same scrutiny as the code it describes.

## Prevention / Follow-ups

### Immediate
- Done: `isAcceptedBackgroundLaunch` is now the single source of truth for "is this call actually a live detached background task," reused everywhere that needs the answer instead of re-deriving it from `params` directly.
- Done: `muxspect dock`'s `bg` column surfaces which entries are genuinely-accepted background launches — not a detector for the #2518 bug itself (that class is now structurally impossible; see "What `bg` does and doesn't detect" above), but visibility into the adjacent risk of a real launch whose notification never lands.

### Longer term
- `docs/MUXSPECT.md` still documents that `STUCK?` cannot fire for a `bg`-tagged row even when it should be investigated — worth revisiting if a server-side view of `<task-notification>` delivery ever becomes cheap to add (today it would mean duplicating client-side transcript-parsing logic server-side, which was judged disproportionate for what is now a fixed bug class).
- **Not yet tracked as an issue:** `bg` visibility silently expires after `MAX_NODE_AGE_MS` (one hour) with no refresh push for a still-pending accepted launch, so it can't help with the specific hours-old symptom that motivated this retro. A background launch that pushes a periodic "still alive" delta (or a longer/no TTL specifically for `bg: true` entries) would close this; worth its own issue rather than folding into #2520's already-closed scope.
- No general process exists yet for `git merge-base --is-ancestor origin/main HEAD` sanity-checking before every push — it's a manual habit this retro reinforces, not a hook. A pre-push check (local git hook or a `task` target) would remove the manual step entirely.
