# macOS PR Triage — 2026-06-01

Generated from open PR audit. Verdict for each: **CLOSE** or **FIX** with specific steps.

---

## CLOSE — stale, superseded, or version-conflict-only

### #209 feat(reactive): Docker attach delivery (2026-03-22, 70 days old)
**Branch:** agenty/feat-container-jekt-delivery | Conflicts: YES

Reagent block: duplicate version 0.32.74 already in main (parallel PR #212). No macOS relevance.
**Action:** Close. If Docker attach delivery is still wanted, open a fresh PR rebased on current main.

---

### #300 fix: Win11 focus border + packaging resilience (2026-04-05, 57 days old)
**Branch:** agentx/win11-focus-border-fix | Conflicts: YES

Reagent blocks:
- Version downgrade (main is 0.33.45, PR has 0.33.44)
- Hardcoded `#419FE0` color fallback — drifts with themes; should read `--accent-color` via CSS

Not macOS. 57 days old with version conflict.
**Action:** Close. The packaging-resilience half may be worth reviving as a fresh PR; the color fix is straightforward but needs a rebase.

---

### #403 fix(macos): patch NSApplication for macOS 26 Tahoe drag crash (2026-04-15, 47 days old)
**Branch:** fix/macos26-cef-drag-crash | Conflicts: YES

Reagent blocks:
- Version downgrade: PR has 0.33.169, main is far ahead
- CFBundleIdentifier bakes version string `ai.agentmux.app.v0-33-169` — resets user data every release
- `on_before_popup` stubs ALL unrecognized NSApplication selectors unconditionally — masks real bugs; should use an allowlist of the known CEF selectors

The drag-crash fix is now partly superseded: #1182/#1185/#1186/#1194 (merged) fixed macOS floating-pane tear-off/redock/drag-slideback. Unclear whether this specific drag-crash path still fires. CEF 148 on the `cef-148-bump` branch (PR #1221) may also resolve it differently.
**Action:** Close. If the specific drag path still crashes after #1221 merges, file a new targeted PR with only the NSApplication selector stubs (allowlist, not catch-all) on a current base.

---

### #444 feat(window): macOS traffic-light caption buttons (2026-04-18, 44 days old)
**Branch:** feat/macos-traffic-light-controls-v2 | Conflicts: YES | Review: APPROVED

Approved but 44 days stale with merge conflicts. No reagent blocks — purely a rebase issue.
**Action:** Close for now (conflicts make it unmergeable, and the macOS packaging work in #1221 has changed the window/bundle layout). Reopen as a fresh PR once #1221 is merged and the packaged macOS baseline is stable.

---

## FIX — actionable, specific steps

### #938 fix(shell,term): surface PTY errors + guarantee keystroke ordering (2026-05-20, 12 days old)
**Branch:** agenty/dead-terminal-error-propagation | Conflicts: YES

Reagent P1 (critical):
- `input_seq_next` / `input_seq_buf` initialized at constructor, never reset in `start()`. If the frontend's `TermViewModel.inputSeq` resets (page reload / reconnect) while backend persists, seq values are silently discarded as duplicates → terminal becomes unresponsive.
- **Fix:** In `start()` (shell.rs ~line 177), reset `self.input_seq_next = 0` and `self.input_seq_buf.clear()` before beginning the new session.

Reagent P2:
- bench-term-echo.mjs:110 — "portable" label conflates installed vs portable builds. Low priority.

Steps:
1. `git fetch origin main && git rebase origin/main` — resolve conflicts
2. Add seq reset in `start()` (shell.rs:177)
3. Push; reagent will re-review

---

### #1137 perf(pane-focus): skip updateTree for FocusNode + drop diag console.logs (2026-05-28, 4 days old)
**Branch:** agentu/pane-focus-perf-1136 | Conflicts: NO | Review: CHANGES_REQUESTED

Reagent P2 (trivial comment cleanup only):
- block.tsx:191 — comment says "removed in PR #..." — delete the whole comment; no PR references in code
- layoutModel.ts:588 — references issue #1136 and an external analysis doc; trim to only the technical WHY (FocusNode geometry-invariant, isFocused memos via localTreeStateAtom._set)

Steps:
1. Delete the stale comment at block.tsx:191
2. Trim layoutModel.ts:588 comment to just the invariant rationale, no issue/PR numbers
3. Push; should get approved

Likely the fastest merge of the group — 30 minutes of work.

---

### #1221 feat(macos): standard notarized DMG — CEF 148, patched renderer, launcher splash, lean packaging (2026-05-31, 1 day old)
**Branch:** agenta/cef-148-bump | Conflicts: YES (needs rebase after #1224/#1220 merged today)

Reagent P2 (all three already fixed in commits pushed today — reagent will re-review on push):
- ~~Schema asset missing from packaged DMG~~ → fixed: `dist/schema` now bundled at Resources/schema + MacOS/schema symlink
- ~~Dead per-type helper binaries~~ → fixed: only generic + Alerts helpers shipped now
- ~~Misleading MachPort comment~~ → fixed: corrected to say flags are belt-and-suspenders, patch is the real fix

Also fixed today (not yet in reagent review):
- `CloseWindowTask` → `try_close_browser()` instead of `window.close()` (stops SIGABRT on Cmd+Q)
- `on_before_close` / `on_after_created` / `can_close` `.expect()` panics → graceful returns
- `CreateWindowTask` null-client guard (stops SIGABRT on multi-window tear-off)
- `AgentMux Helper (Alerts)` added (stops posix_spawnp loop)

Steps:
1. Rebase onto current main (after today's merges): `git fetch origin main && git rebase origin/main`
2. Resolve any conflicts in `scripts/package-macos.sh` (likely minor — take theirs for version files)
3. Push; reagent will re-review the fixed issues and the new commits
4. Once reagent approves → merge
5. Rebuild notarized DMG and upload to the 0.40.1 release assets

---

## Not macOS / not closing — context only

### #1132 feat(floating-pane): resize + maximize support (2026-05-28)
Not purely macOS but related. P0 version downgrade (0.39.3 → 0.39.2) from stale rebase. Needs `git rebase origin/main` taking main's version files + all `.changesets/` kept. Separate triage.

---

## Summary table

| PR | Age | Verdict | Effort |
|----|-----|---------|--------|
| #209 | 70d | **CLOSE** | — |
| #300 | 57d | **CLOSE** | — |
| #403 | 47d | **CLOSE** | — |
| #444 | 44d | **CLOSE** (reopen after #1221) | — |
| #938 | 12d | **FIX** | 1–2h (rebase + seq reset) |
| #1137 | 4d | **FIX** | 30min (comment trim) |
| #1221 | 1d | **FIX** | 30min (rebase) + notarized DMG rebuild |
