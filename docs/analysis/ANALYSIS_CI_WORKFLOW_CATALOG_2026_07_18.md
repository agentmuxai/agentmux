# CI Workflow Catalog — What We Have, What's Redundant, What to Cut

**Date:** 2026-07-18
**Primary scope:** `agentmuxai/agentmux` `.github/workflows/` (12 registered workflows)
**Also checked:** `agentmuxai/agentmux-builder`, `agentmuxai/agentmux-landing`,
`agentmuxai/cef`, `a5af/shared-infrastructure` — cross-repo context needed to
answer this doc's own open questions (see §5). Could not locate `a5af/dev-tools`
or `agentmuxai/dev-tools` under any spelling — flagged in §5.4.
**Trigger:** investigating the nightly-macOS CEF gate failure (PR #2223) surfaced
that the repo has two similarly-named "nightly" build workflows and a monitoring
system (`a5af/shared-infrastructure`'s `gh-reporter`) that only watches one of
them. That prompted a full inventory.

---

## 1. The catalog

| # | Workflow (name) | File | Trigger | Purpose | Last activity | Status |
|---|---|---|---|---|---|---|
| 1 | CI (PR) — compile tests + run | `ci-pr.yml` | every PR + push to `main` | `cargo check --workspace --tests` + `cargo test` (Windows required, Ubuntu non-blocking) + `vitest`. The merge-time gate. | Runs constantly, all green | **Essential** |
| 2 | CI — nightly cross-platform build + test | `ci-nightly-build.yml` | schedule 07:00 UTC + manual | `cargo build --release --workspace` + `cargo test` + `vitest`, matrix over Windows/Ubuntu/macOS (Windows required, others non-blocking). | Daily, all green | **Mostly redundant — see §2.1** |
| 3 | CI — nightly artifacts (all platforms) | `ci-nightly-artifacts.yml` | schedule 06:00 UTC + manual | Full packaging health-check: builds + signs + notarizes a real DMG/AppImage/portable-ZIP+installer+MSIX for all 3 platforms, uploads as workflow artifacts (not a GitHub release). | Daily; **macOS leg failing 19 days straight** (fix in #2223) | **Essential — was silently broken** |
| 4 | build-linux | `build-linux.yml` | manual / `repository_dispatch` | On-demand: build **one** platform's AppImage from an arbitrary ref, optionally upload to an existing GitHub release. | **Last run 2026-06-25** (23 days ago), several releases since | **Idle but intentional break-glass tool — see §2.2, §5.1** |
| 5 | build-macos | `build-macos.yml` | manual / `repository_dispatch` | Same as #4 but for the signed+notarized macOS DMG. | **Last run 2026-06-25** (23 days ago) | **Idle but intentional break-glass tool — see §2.2, §5.1** |
| 6 | Release | `release.yml` | push tag `vX.Y.Z` + manual | The actual release pipeline: verify version consistency → build Windows/Linux/macOS → publish GitHub Release → WinGet + MS Store submission → trigger landing-page deploy. | Fires on every release (07-03, 07-04, 07-07, 07-11, 07-15), all green | **Essential** |
| 7 | Release consistency check | `release-consistency.yml` | PR/push touching `VERSION_HISTORY.md` | Deterministic CI gate for the 5-location version-agreement invariant (post-mortem: `retro-release-version-desync-2026-05-22.md`). Fast, no build. | Fires only on release PRs, all green | **Essential, well-scoped** |
| 8 | Container Agent Image | `container-image.yml` | push tag `v*` + manual | Builds + pushes the `agentmuxai/agent-claude` Docker image (amd64+arm64) to GHCR. | Fires alongside every release tag | **Essential, distinct purpose** |
| 9 | Input-handler layout-read guardrail | `input-handler-layout-reads.yml` | schedule 10:00 UTC + manual | Grep gate: no `scrollHeight`/`getBoundingClientRect`/etc. on the keystroke path (regression class: 22ms/keystroke reflow, `agent-typing-lag-trace-2026-04-12.md`). | Daily, all green, sub-minute job | **Keep — near-duplicate shape of #10, could merge files** |
| 10 | Input-handler sync-IPC guardrail | `input-handler-sync-ipc.yml` | schedule 10:00 UTC + manual | Grep gate: no blocking IPC on the keystroke path before paint. | Daily, all green, sub-minute job | **Keep — near-duplicate shape of #9, could merge files** |
| 11 | Input-latency bench (report) | `input-bench-report.yml` | manual only | Keystroke/echo latency benchmark vs. committed baseline. **Requires a self-hosted runner labeled `input-bench` that does not exist** — the job queues forever. | **Zero runs, ever** | **Dead — see §2.3** |
| 12 | Kimi Code Review | `kimi-review.yml` | (registered, not on `main`) | Community PR auto-reviewer (Kimi model). File only exists on the unmerged, abandoned branch `agenty/kimi-community-reviewer` (last commit 2026-05-05). GitHub still lists it "active" because it ran twice there. | 2 runs total, both on 2026-05-06, **both failed**, on its own branch | **Ghost — see §2.4** |

---

## 2. Findings

### 2.1 Two "nightly" workflows, only one of which is watched

`ci-nightly-build.yml` ("build + test") and `ci-nightly-artifacts.yml`
("artifacts") sound like variants of the same thing and are easy to conflate —
which is exactly what happened: `gh-reporter` (the nightly health-report Lambda
in `a5af/shared-infrastructure`) hardcodes `NIGHTLY_BUILD_WORKFLOW =
"ci-nightly-build.yml"` and has never watched the artifacts workflow. Result:
the artifacts pipeline's macOS leg was broken for 19 consecutive days while the
nightly email reported all-green, because the thing it checked (does the code
compile and pass tests) was genuinely fine the whole time.

Now that `ci-pr.yml` runs `cargo check --tests` + `cargo test` on **every PR**
for Windows (required) and Ubuntu (non-blocking) — added 2026-06-23 specifically
because manual-only release workflows and diff-only review were letting broken
test builds merge unnoticed (issues #1823, #1876) — `ci-nightly-build.yml`'s
Windows and Ubuntu legs are now redundant with a much tighter, per-PR gate.
**Its only remaining unique value is the macOS compile leg**, which nothing else
covers (no PR-time macOS compile check exists).

### 2.2 `build-linux.yml` / `build-macos.yml` — real tooling, just idle for 23 days

**Update after checking `agentmuxai/agentmux-builder` (§5.1): this is not
dead code.** Its README explicitly documents these two workflows, living in
the public `agentmux` repo on purpose, as the break-glass path — "rebuild one
platform's artifact against an *existing* release, without re-running the whole
pipeline" — with worked `gh workflow run` examples. The 23-day idle period isn't
neglect, it's the expected shape of a rarely-needed tool: every release since
2026-06-25 went through `release.yml` cleanly enough that nobody needed a
single-platform patch. That's a *good* sign, not evidence of cruft.

The real issue is still duplication, not disuse: their packaging logic is
**fully copy-pasted, not called** — `release.yml`'s own `build-linux`/`build-macos`
jobs inline the identical CEF-resolve/cache/download steps rather than
dispatching to these workflows. That's three independent copies of the same
~40-line CEF-provisioning block across `build-linux.yml`, `build-macos.yml`, and
`release.yml` (plus a fourth, until today's fix, in `ci-nightly-artifacts.yml`
— exactly how the Linux resolver bug and the missing macOS wiring both
happened: a fix landed in one copy and silently didn't propagate to the
others). Keep the capability, collapse the copies.

### 2.3 `input-bench-report.yml` — dead on arrival

Zero runs, ever. It requires a self-hosted runner labeled `input-bench` that,
per the workflow's own comment, "does not exist" — every dispatch just queues
forever with nothing to pick it up. This was correctly scoped as
`workflow_dispatch`-only and clearly labeled as blocked on infra that was never
provisioned; it's not broken, it's just not live.

### 2.4 `kimi-review.yml` — a ghost workflow

Registered as "active" in the Actions tab, but the file doesn't exist on `main`
— it only exists on `agenty/kimi-community-reviewer`, a branch that was never
merged and hasn't been touched since 2026-05-05. Its only two runs (2026-05-06)
both failed. GitHub keeps workflows that ran at least once listed as "active"
even after the source branch goes stale, which is why it still shows up
alongside the 11 real ones.

### 2.5 Minor: the two input-handler guardrails are near-identical shape

`input-handler-layout-reads.yml` and `input-handler-sync-ipc.yml` are
byte-for-byte the same trigger, same runner, same single-step structure —
differing only in which lint script they invoke. Not a correctness issue, just
file sprawl; trivial to fold into one workflow with two jobs (or a matrix) if
reducing the *count* of workflow files is a goal, at no loss of function.

---

## 3. Proposed distillation — status

Ordered by confidence. All decided/executed 2026-07-18 except one deliberately
deferred item — see §6 for the actual PRs.

1. ✅ **Done.** Fixed the monitoring gap: `a5af/shared-infrastructure` PR #382
   adds an independently-watched `ci-nightly-artifacts.yml` section to
   `gh-reporter` rather than swapping which workflow it watches (swapping just
   relocates the blind spot — the two workflows fail independently).

2. ✅ **Done.** Deleted `input-bench-report.yml` — `agentmuxai/agentmux` PR #2225.

3. ✅ **Done.** Deleted the abandoned `agenty/kimi-community-reviewer` branch —
   confirmed with Asaf, no plans to revive it. Removes the ghost workflow entry.

4. ✅ **Done, revised scope.** `build-linux.yml`/`build-macos.yml` kept (confirmed
   intentional). De-duplicated the CEF-provisioning logic — `release.yml`'s
   `build-linux`/`build-macos` jobs now call the two standalone workflows as
   *reusable workflows* (`workflow_call`, not `repository_dispatch` — that
   trigger can't block on / consume outputs from the dispatched run, so it
   wasn't actually usable for this) instead of inlining a third copy of the
   same ~40-line block. `agentmuxai/agentmux` PR (pending open, see §6).
   (`agentmux-builder`'s `build-windows.yml` — the Windows sibling — is moot;
   the repo it lived in was deleted 2026-07-18, see §5.1.)

5. ⏸ **Deferred — Asaf's call.** Asked directly: keep all three platforms in
   `ci-nightly-build.yml`'s nightly matrix, intentionally, as an independent
   signal from `ci-pr.yml` rather than pure overlap. No change made.

6. ✅ **Done.** Merged `input-handler-layout-reads.yml` +
   `input-handler-sync-ipc.yml` into one `input-handler-guardrails.yml` with two
   jobs — `agentmuxai/agentmux` PR #2225 (same PR as item 2).

**Not touched:** `ci-pr.yml`, `ci-nightly-artifacts.yml`, `release.yml`,
`release-consistency.yml`, `container-image.yml` — each covers a distinct,
currently-exercised need with no overlap.

---

## 4. Open questions — resolved

- ~~Is `kimi-review.yml` intended to come back?~~ Resolved: no, deleted the
  branch (§3.3).
- ~~Is the Windows/Ubuntu leg of `ci-nightly-build.yml` deliberately
  redundant with `ci-pr.yml`?~~ Resolved: yes, intentional — keeping as-is
  (§3.5).
- ~~`build-windows.yml` in `agentmux-builder`~~ — resolved: repo deleted
  2026-07-18 (§5.1), question moot.

---

## 5. Cross-repo context

You asked whether I'd checked `agentmux-landing`, the rest of
`a5af/shared-infrastructure`, `a5af/dev-tools`, and `agentmuxai/cef` — I hadn't,
beyond the one `gh-reporter` file from the earlier monitoring-gap thread. Here's
what each actually contains, and what it resolves.

### 5.1 `agentmuxai/agentmux-builder` — deleted 2026-07-18, was reference-only

**Update:** this repo has since been deleted. Per Asaf: it was deprecated —
AgentMux is open source and doesn't need a separate private repo to manage
signing secrets, so `agentmux-builder` was being kept around purely for
reference, and its stale docs (§4.2's outdated example commands, the
never-validated `build-windows.yml`) were actively causing confusion rather
than adding value. All signing secrets live directly in the public
`agentmux` repo. The analysis below is preserved as a record of what it
contained and why it doesn't change any conclusion in this report — the repo
itself is gone.

Its own README stated the architecture explicitly:

> **Status (2026-06): native-CEF CI across all three platforms.**
> - `build-windows.yml` (here) — builds the portable + Inno Setup installer...
>   **Needs runner validation.**
> - `build-macos.yml` + `build-linux.yml` — live in `agentmuxai/agentmux`
>   (public repo, unlimited free CI minutes)...

So `build-linux.yml`/`build-macos.yml` living in the public `agentmux` repo,
separate from `agentmux-builder`, is the *documented, intended* split — not
architectural drift. `agentmux-builder` exists specifically to keep Apple/
signing secrets out of the public repo's Action logs while still using free CI
minutes for the platforms that don't need those secrets managed there. I also
grepped `github-router/lambda/router.py` and did an org-wide code search in
`a5af/shared-infrastructure` for `build-linux`/`build-macos`/
`repository_dispatch` — zero hits, confirming nothing external triggers them
either. Combined: real, deliberately-placed, rarely-needed break-glass tooling.
Recommendation in §3.4 updated accordingly (keep, de-duplicate).

`agentmux-builder` hosted exactly one workflow, `build-windows.yml` — the
Windows sibling of the same family. Unlike its macOS/Linux counterparts, it had
**zero runs, ever**, and carried its own status banner: "⚠️ initial scaffold —
VALIDATE ON A RUNNER before relying on it." It was also missing its signing
secrets (`SIGNPATH_*` "not set; deferred"), so even a first run would have
produced an unsigned installer. That workflow, and the org's only Windows
single-platform-rebuild path, no longer exists anywhere — it's gone with the
repo, not migrated. If a Windows equivalent to `build-linux.yml`/`build-macos.yml`
is ever wanted, it'd need to be rebuilt from scratch (or from git history —
`agentmuxai/agentmux-builder` isn't recoverable, but nothing in it was unique;
Windows packaging logic already exists in `release.yml` and
`ci-nightly-artifacts.yml` in the public repo).

### 5.2 `agentmuxai/agentmux-landing` (private)

One workflow: `landing-deploy.yml`, triggered by `repository_dispatch`. Its run
timestamps (07-15, 07-11, 07-07) line up exactly with `release.yml`'s tag-push
runs in the main repo — confirming `release.yml`'s documented "trigger the
landing page deploy" step is working as designed. No overlap, no redundancy,
nothing to distill.

### 5.3 `agentmuxai/cef` (public)

No CI workflows at all — checked both the Actions API and the `.github/`
directory contents directly. This is the source tree holding the
`BeginWindowDrag`/transparency patches; the patched-framework release assets
(`cef-macos-arm64-*`, `cef-linux-x86_64-*`) referenced throughout §2.1/§2.2 are
cut **manually**, by running the build + `gh release create` commands locally
per `docs/cef-build/build-patched-framework-macos.md` — not by any automated
pipeline. Nothing to catalog or distill here; flagging only because it was
explicitly in scope for this pass.

### 5.4 `a5af/dev-tools` — not found

Tried `a5af/dev-tools`, `a5af/devtools`, `a5af/dev_tools`, and
`agentmuxai/dev-tools` directly via the API, plus `gh search repos` scoped to
both owners — no match. Full `gh repo list` for both `a5af` and `agentmuxai`
doesn't show anything by that name either. Either it's under a different
owner/org I don't have visibility into, it's private and hasn't been shared
with me the way `shared-infrastructure` was, or the name's slightly different
from what I'm guessing. If it's relevant here, the exact owner/repo would help.

---

## 6. Execution log

What actually shipped from this catalog, 2026-07-18:

| Item | PR | Status |
|---|---|---|
| Nightly macOS CEF wiring fix (the bug that started this whole investigation) | `agentmuxai/agentmux#2223` | Open, checks clean, awaiting merge approval |
| Delete `input-bench-report.yml` + merge input-handler guardrails | `agentmuxai/agentmux#2225` | Open |
| De-duplicate CEF-provisioning logic (`build-linux.yml`/`build-macos.yml` as reusable workflows called from `release.yml`) | `agentmuxai/agentmux` (branch `chore/dedupe-cef-provisioning`) | Open — smoke-tested `build-linux.yml`'s own `workflow_dispatch` path standalone before opening; the new `workflow_call` path from `release.yml` itself is **not** end-to-end validated (would require a real `release.yml` run, which touches production signing credentials and an existing release's assets — deferred to Asaf's judgment on when/how to validate before it's relied on for a real release) |
| `gh-reporter` monitoring fix (watch both nightly workflows independently) | `a5af/shared-infrastructure#382` | Open, tests pass (76/76), `tsc` compiles clean |
| Delete abandoned `agenty/kimi-community-reviewer` branch | — | Done directly (branch deleted) |
| Delete `agentmuxai/agentmux-builder` | — | Done by Asaf directly on GitHub (my token lacked the `delete_repo` scope) |

No merges happened without explicit approval — all four PRs above are open,
pending review/merge decisions.
