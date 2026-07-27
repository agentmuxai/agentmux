# Spec: Finalize Media pane + CEF Windows codec PRs, confirm CI pulls the real build

## Context

Two PRs are open and functionally verified end-to-end via live `task dev`
testing (codec-enabled `libcef.dll` boot-verified, Media pane confirmed
playing a previously-failing MP4 by the user), but neither is mergeable yet:

- **#2299** `feat(media): add Media pane widget with live directory-watch
  updates` — CI green, but `reviewDecision: REVIEW_REQUIRED` with one open P2
  from reagent's latest re-review.
- **#2308** `feat(cef): proprietary codec support -- Windows implementation +
  Linux/macOS spec` — CI green, `mergeable: MERGEABLE`, but
  `reviewDecision: CHANGES_REQUESTED` with two P0s and one P2 from reagent's
  latest re-review (2026-07-27T07:20:33Z).

Separately, the user asked to "tidy up the build system so windows pulls in
the updated CEF on the main repo's CI release builds" — see the "Build system
already wired" section below for why this needs verification/documentation
rather than new plumbing.

## Outstanding review blockers

### PR #2299 — one open P2

> `CLAUDE.md:159` — Widgets table lists `defwidget@media` as "Pinned", but the
> `widgets.json` entry this PR adds sets `"display:pinned": false`, so the
> widget actually only surfaces in the "More" dropdown, contradicting the doc
> row added in the same change.

Confirmed live: `agentmux-srv/src/config/widgets.json`'s `defwidget@media`
entry has `"display:pinned": false`, while `CLAUDE.md`'s widget table already
documents it as `Pinned` (added in the same PR). Every other core pane
(agent/browser/terminal/sysinfo/editor/drone/help/swarm/warden) is pinned;
Media is meant to be a first-class, directly-discoverable pane per the PR's
own framing, not a secondary tool like `defwidget@toolchain`. **Fix: flip
`"display:pinned"` to `true`** so code matches the already-intentional doc
row, rather than watering down the doc.

### PR #2308 — two P0s + one P2

> `package.json:9` — Version downgraded from 0.54.5 (current main) to 0.54.4;
> same downgrade in Cargo.toml — merging reverts the released version number.
>
> `VERSION_HISTORY.md:1` — Entire "## 0.54.5" changelog section present on
> main is deleted by this branch, losing the record of already-shipped fixes.

Root cause: the branch was cut before a `chore: release v0.54.5` PR landed on
main and has never been rebased since. **Fix: rebase
`docs/cef-proprietary-codecs-spec` onto `origin/main`** — this is a pure
version-desync, not a real content conflict (confirmed: `origin/main`'s
`package.json`/`Cargo.toml` are already at `0.54.5`; the rebase just needs to
pick that up instead of the branch's stale 0.54.4 baseline).

> `docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md:10` —
> References `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md` as
> the triggering root-cause report, but this file does not exist anywhere in
> the repo.

Confirmed: the report file exists and is committed on `feat/media-pane` (it
was written while diagnosing the Media pane's MP4 playback failure — the
event that triggered the whole codec investigation) but was never carried
over to `docs/cef-proprietary-codecs-spec`, even though this branch's own
spec doc cites it by path. **Fix: cherry-pick the report file onto this
branch** so the citation resolves.

## Build system: already wired, needs confirmation not new code

`build-windows.yml` (this session, PR #2308) resolves the codec-enabled
runtime the same way `build-linux.yml`/`build-macos.yml` already do: if
`cef-runtime-tag` input is blank (the default `release.yml` passes),
it resolves to the newest `cef-windows-x86_64-*` release via
`gh release list --repo agentmuxai/cef ... select(startswith("cef-windows-x86_64-"))`.

This was **validated live** on 2026-07-27, not just read from source: after
publishing `cef-windows-x86_64-148.0.7778.180` to `agentmuxai/cef`, running
the exact same `gh release list` command CI uses resolved that tag correctly
with zero code changes needed. So "tidy up the build system" for Windows is
scoped to **finishing the PR** (the plumbing already does the right thing),
plus updating two doc headers that still say "first-pass draft, not yet
validated" now that a real build has shipped:

- `docs/cef-build/build-patched-cef-windows.md` — top-of-file status note.
- `scripts/cef-build/args-windows.gn` — header comment citing an "unverified"
  size delta and a "not yet verified against a real build" GN-args set (the
  GN args themselves were already corrected mid-build on 2026-07-26 per that
  file's own changelog comment — only the top status framing is stale).

`CEF_RUNTIME_TOKEN` (the PAT `build-windows.yml` uses to read from
`agentmuxai/cef`) is already provisioned as a repo secret — confirmed via
`gh api repos/agentmuxai/agentmux/actions/secrets`, same secret Linux/macOS
already depend on. No new secret provisioning needed.

## Plan

1. **PR #2299**: flip `defwidget@media`'s `"display:pinned"` to `true` in
   `agentmux-srv/src/config/widgets.json`. Commit, push, comment on the PR
   addressing the P2, re-request reagent review.
2. **PR #2308**:
   - `git fetch origin main && git rebase origin/main` on
     `docs/cef-proprietary-codecs-spec` — resolves both P0s by construction.
   - Add `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`
     (cherry-picked from `feat/media-pane`) — resolves the P2.
   - Update the two stale doc-status headers to reflect the real, booted,
     published build (cite the actual release URL + boot verification).
   - Push (force-push required after rebase — this is my own PR branch, no
     other collaborators pushing to it), comment addressing all three
     findings, re-request reagent review.
3. Re-verify CI stays green on both PRs after the pushes.
4. Report back with both PRs' final state; **do not merge without explicit
   user confirmation** — merging is a shared/visible action outside the scope
   of "get it into PRs."

## Files touched

| File | Change |
|---|---|
| `agentmux-srv/src/config/widgets.json` | `defwidget@media` → `"display:pinned": true` |
| `docs/cef-proprietary-codecs-spec` branch | rebase onto `origin/main` |
| `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md` | added to PR #2308's branch (already exists on `feat/media-pane`) |
| `docs/cef-build/build-patched-cef-windows.md` | status header: draft → validated, real build details |
| `scripts/cef-build/args-windows.gn` | header comment: unverified → validated, real build details |

No `agentmux-srv`/`agentmux-cef` runtime code changes — this pass is entirely
PR-hygiene (review blockers, doc accuracy) plus confirming (not rebuilding)
already-working CI plumbing.
