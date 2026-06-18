# Retro: Main couldn't build after a clean session of merges

**Date:** 2026-05-15
**Author:** AgentA
**Related:** RFC #857 phases 0-3 (#860/#862/#865), 4 other-agent rebases (#866-#869), #873 (Taskfile YAML), #875 (launcher match arms)

---

## Summary

In a single working session, I shipped RFC #857 phases 0-3, rebased 4 other-agent PRs onto current main, and merged the YAML quoting fix. Every PR had clean CI-equivalent verification at its own HEAD. But when I ran `task package` to produce a fresh portable build at the very end, **main didn't compile**.

Six compile errors across two crates required two more PRs (#873 partial, #875) before the build went green. None of those errors were introduced by the day's work — every one had been silently latent on main for hours, possibly days.

This retro is about why latent breakage like this is invisible until somebody actually tries to build, and what changes would catch it earlier.

## The 6 errors

| # | Crate | Site | Cause | Introduced by |
|---|---|---|---|---|
| 1 | `agentmux-cef` | `window.rs:792` `browser.host().window_handle()` | `host()` returns `Option<BrowserHost>`; can't call `.window_handle()` on the Option directly (E0599) | Pre-existing — `cef` crate semantics |
| 2-3 | `agentmux-cef` | `window.rs:794-796` `hwnd.is_null()`, `hwnd as isize` | `HWND` became a struct (`.0` for the inner ptr) in current `windows-sys` (E0599, E0605) | windows-sys crate update — some prior PR |
| 4 | `agentmux-cef` | `window.rs:848,857` `WindowOpacityApplied`/`Cleared` patterns | Variants gained a `version: u64` field, patterns non-exhaustive (E0027) | Reducer-version field added by a parallel PR |
| 5 | `agentmux-launcher` | `ipc/server.rs:651` exhaustive match on `Command` | `Command::UpdateWindowMeta` variant added; launcher's match was non-exhaustive (E0004) | PR #856 (Phase E.5.x reducer migration) |
| 6 | `agentmux-launcher` | `reducer/mod.rs:87` same | Same as #5 | PR #856 |
| 7 | `agentmux-launcher` | `event_log.rs:245` exhaustive match on `Event` | `Event::WindowMetaUpdated` variant added; match non-exhaustive (E0004) | PR #856 |

Each is **trivially correct in the originating PR**. When PR #856 added `UpdateWindowMeta` to the Command enum + appropriate handler in srv, it didn't *touch* `agentmux-launcher`, so its CI-equivalent (whatever local `cargo check` AgentY ran) didn't surface the launcher's exhaustive match becoming non-exhaustive. The launcher only fails to compile when somebody runs `cargo check -p agentmux-launcher` later.

## Why CI didn't catch it

**Because there is no CI compile gate.** The repo has:
- ✅ Reagent bot reviewing code
- ✅ Codex bot reviewing code
- ✅ A `test.yml` workflow that runs Python unit tests (Reagent's own, not agentmux's)
- ❌ **No GitHub Action that runs `cargo check` / `cargo build` / `cargo test`**
- ❌ No required status checks on the `main` branch protection

So every PR that lands is gated only on:
1. Reagent + codex review (logic, style, regressions — not build)
2. `dismiss_stale_reviews: true` (which only dismisses APPROVED, not CHANGES_REQUESTED — caveat documented in RFC #857 Phase 3)

A PR that compiles against its own base but breaks main can land cleanly.

## Why is this happening NOW?

It's been latent for a while. Two trends amplified it today:

1. **Multi-agent velocity.** 3+ agents committing to overlapping enum surfaces (the IPC `Command`/`Event` enums, the host's `HostEvent` enum, CEF binding versions). Each agent's PR was correct in isolation. The combinations created the breakage. Today saw 7+ PRs landing in a few hours — easily the densest day this repo has had.

2. **Rebases batched at the end.** I rebased 4 other-agent PRs in series, each on top of current main. None of them touched the broken sites. But they meant when I ran `task package` for a build, it cargo-checked the whole workspace and surfaced the latent errors. Without my rebase batch and build attempt, the breakage would have sat undetected until somebody did `task dev` or `task build:backend`.

## Was it changesets?

**No.** The changesets pattern (RFC #857 Phase 2) prevents version-file merge conflicts. It does nothing for API-evolution conflicts. The breakage today would have happened identically with `bump patch` per-PR — and arguably WORSE, because more conflicts would have masked the real signals.

If anything, changesets made this *easier* to fix: the 4 rebase PRs went through cleanly without me wrestling 4 Cargo.toml conflicts each, so I had attention budget left over to debug the compile errors when they surfaced.

## What we should do

### Phase 6 — CI compile gate on every PR

A GitHub Actions workflow that runs on every PR push:

```yaml
name: Build
on: [pull_request, push]
jobs:
  cargo-check:
    runs-on: windows-latest  # primary target; CEF needs Windows toolchain
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo check --workspace --release --locked
  vitest:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - run: npm ci
      - run: npx vitest run
```

Then add the check as a **required status check** on main branch protection:

```bash
gh api -X PATCH "repos/agentmuxai/agentmux/branches/main/protection/required_status_checks" \
  -F strict=true \
  -F 'contexts[]=cargo-check' \
  -F 'contexts[]=vitest'
```

`strict=true` (= "Require branches to be up to date before merging") forces the PR to be rebased onto current main before the status check runs. This catches our exact failure mode: PR compiles against its own base but not against current main.

### Phase 7 — Merge queue

GitHub's native merge queue (or Mergify) takes this further:

1. PR is approved + reviews pass
2. Author clicks "Merge when ready"
3. GitHub creates a queue, picks the next ready PR, creates a hidden temporary branch with PR rebased onto current main
4. CI runs against THAT branch
5. Only merges if green

The advantage over `strict=true` alone: with `strict=true`, two PRs each individually rebased-onto-main can still race past each other (PR A rebases at 10:00, PR B rebases at 10:01, both pass their checks at 10:03, both merge at 10:04 — but PR A's merge changed main at 10:04, and PR B's check was against pre-merge main). Merge queue serializes this: PR A merges first, then PR B's rebase happens AGAINST PR A's merged version, CI re-runs, then merges.

For multi-agent workflows, the merge queue is the right fit.

### Phase 8 — Cargo workspace `--locked` flag

`cargo check --workspace --release --locked` adds: refuse to update Cargo.lock. If any PR landed an upstream crate update without committing the Cargo.lock change, this fails. Catches the windows-sys / CEF crate drift class.

## Verification this session

After all the fixes landed (#873 + #875):

- ✅ `cargo check --workspace --release` clean (51 warnings, 0 errors)
- ✅ `task package` produced `~/Desktop/agentmux-0.33.898-x64-portable.zip` (164 MB)
- ✅ Extracted folder exists and is correct shape

## Sequence of fixes shipped today

| PR | Title | Why |
|---|---|---|
| #860 | Phase 0 conflict-marker hook | Conflict-marker P0 class |
| #862 | Phase 1 Cargo workspace version | Reduce conflict surface 9→4 |
| #865 | Phase 2 changesets | Conflict surface 4→0 in feature PRs |
| #866 | rebase #839 (taskfile) | Other-agent PR brought current |
| #867 | rebase #827 (DPI) | " |
| #868 | rebase #863+#858 (hamburger + opacity) | " |
| #869 | rebase #797 (Wayland transparency) | " |
| #873 | Taskfile YAML quoting | Phase 2 introduced parse error |
| #875 | Launcher match arms | Pre-existing main breakage from #856 |
| #870 | issue: dev:serve TOCTOU race | Follow-up from #866 review |
| #871 | issue: opacity drag lag + hwnd race | Follow-up from #868 review |
| #872 | issue: LCD-text gate + opacity flash | Follow-up from #869 review |

11 PRs + 3 follow-up issues. ~9 hours of session work. The Phase 6 + 7 changes proposed above would have eliminated #873 partial-fix and #875 entirely — they'd have shown up as failed status checks on whatever PR caused them at the time it tried to merge, not weeks later.

🤖 Authored by AgentA. Phase 6+7 implementation can ride with the next infra PR (per `feedback_no_doc_only_prs.md`).
