# Widening Linux Package Targets + a Dedicated Downloads Page

**Status:** Draft — spec only, no implementation yet
**Date:** 2026-09-15
**Author:** Agent3 (fresh spec — no prior spec, issue, or discussion covering
either half of this was found; see §0)
**Scope:** `agentmuxai/agentmux` (build/packaging + release CI — primary
scope) and `agentmuxai/agentmux-landing` (new downloads page, consequence of
the first half, same relationship `SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23.md`
§6 established between these two repos)
**Related:**
`docs/specs/SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md` (Phase B — now
confirmed **implemented**, see §1.2 — is the nightly artifact pipeline this
spec widens), `docs/specs/SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23.md` (the
release→landing-page dispatch this spec's new formats ride on, unmodified),
`docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md` (the
CEF-runtime-pin machinery `build-linux.yml` already uses),
`docs/specs/SPEC_LINUX_APPIMAGE_PER_BUILD_CHANNEL_2026_06_25.md` (existing
Linux packaging task conventions), `agentmux-landing/docs/download-button-refactor.md`
(existing landing-page download-UX decisions this spec extends, not replaces)

---

## 0. Why this is a fresh spec, not a found one

A prior agent apparently described writing a spec for "widening the Linux
build targets on nightly" and "a downloads page like waveterm.dev/download."
Before writing anything, this session searched thoroughly: `docs/specs/` and
`specs/` in all four related repos (`agentmux`, `agentmux-cloud`,
`agentmux-landing`, `shared-infrastructure`), full git history and every
branch (`git log --all --grep`), and GitHub issues/discussions/code search
across the whole `agentmuxai` org (`gh search issues/prs/code`,
`gh api .../discussions`). **Nothing matches.** The closest adjacent material
— `SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md`'s Phase B, the existing
`#download` homepage section, and a stale `docs/linux.md` reference to a
nonexistent `agentmuxai/agentmux-builder` repo — is real but scoped to
neither of the two things asked for here (see §1 for exactly what exists
today vs. what's missing). This document is written fresh, grounded in a
direct read of the current build/release pipeline and landing-page code
(not a guess at what the described-but-unfound spec might have said).

---

## 1. Current state (verified this session, 2026-09-15)

### 1.1 Release build matrix today

| Platform | Formats produced | Arch | Built by |
|---|---|---|---|
| Windows | Portable ZIP, Inno Setup installer (`.exe`), MSIX | x64 only | `build-windows.yml` |
| macOS | Signed + notarized `.dmg` | **arm64 (Apple Silicon) only** | `build-macos.yml`, `macos-latest` runner |
| **Linux** | **AppImage only** | **x86_64 only** | `build-linux.yml`, `ubuntu-22.04` |

**Correction (Codex review, PR #3234):** an earlier revision of this table
claimed macOS also ships an Intel x64 `.dmg`. That was wrong — verified by
direct inspection of `build-macos.yml` (a single `macos-latest` arm64 job,
resolving only `cef-macos-arm64-*` CEF releases and globbing only
`AgentMux_*_arm64.dmg`) and `scripts/package-macos.sh` (`ARCH="arm64"`
hardcoded). **This is the same scaffolded-but-unproduced pattern §1.4
describes for Linux `.deb`** — `Download.tsx`'s `Assets` type and
`DownloadButton.tsx`'s fallback chain both already have an `x64` macOS slot
that nothing has ever populated. Worth noting as a second instance of the
same gap, not a one-off: this codebase's landing-page types appear to get
built ahead of the CI work that would fill them, more than once. Windows
ships 3 formats; macOS ships 1 architecture of one format; **Linux
ships exactly one format, one architecture** — the narrowest of the three by
a wide margin, and the one the ask is specifically about.

### 1.2 Nightly artifacts — Phase B is already live, not deferred

`SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md` (§3-4) described Phase B
(full packaged nightly artifacts, not just a compile check) as **deferred**
at the time it was written. That's stale: `.github/workflows/ci-nightly-artifacts.yml`
exists today (`schedule: '0 6 * * *' UTC`) and delegates to the *same*
`build-windows.yml` / `build-macos.yml` / `build-linux.yml` reusable
workflows `release.yml` calls, just with `release-tag: ""` (upload to Actions
artifacts, not a GitHub Release). **This is good news for scoping this
spec:** widening `build-linux.yml`'s own packaging step widens both nightly
artifacts *and* real releases simultaneously, with no separate nightly-only
code path to maintain — exactly the same "one shared point, many
consumers" shape the model-default change in `shared-infrastructure`'s
`analyst_runner.py` had earlier today, just in this repo's build graph
instead of a Python default parameter.

### 1.3 How the Linux build works today (relevant to widening it cheaply)

`build-linux.yml` (`ubuntu-22.04`) stages a `dist/` tree (launcher, CEF host,
`agentmux-srv`, a patched `libcef.so` pulled from the private
`agentmuxai/cef` release — see `SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md`
— GL/Vulkan libs, `.pak`/locale resources, bundled frontend), then
`scripts/build-appimage-linux.sh` (invoked via `task package:release:linux` →
`scripts/package-linux.sh`) wraps that tree into an AppImage with
`appimagetool`. **The expensive part (compiling, CEF provisioning, staging
the runtime tree) happens exactly once** — packaging is a separate,
comparatively cheap final step over an already-staged directory. This matters
for scoping §2: adding more *package formats* at the same architecture does
not require additional compiles or additional CEF downloads, only additional
repackaging steps over the tree `build-appimage-linux.sh` already stages.

### 1.4 The `.deb` gap — three places already expect it, nothing produces it

This is the most concrete evidence of an intended-but-unfinished wider
target list, found by direct inspection, not inference:

- `docs/linux.md:119`: *"Linux .deb package | Produced by CI builder
  (`agentmuxai/agentmux-builder`) only, not by `task package:linux`"* —
  **`agentmuxai/agentmux-builder` does not exist** (`gh repo list agentmuxai`
  returns 8 repos, none named that). Stale since at least a 2026-06-06
  commit; not a live pipeline.
- `agentmux-landing/scripts/fetch-release.mjs`: already recognizes `.deb`
  assets (`name.endsWith('.deb')` → `assets.linux.deb`), already has a
  `.deb` content-type and an unversioned-name mapping (`amd64.deb` →
  `AgentMux_amd64.deb`), already computes and uploads checksums for it like
  any other asset. **This code has never had a `.deb` asset to actually
  process**, because no workflow uploads one.
- `agentmux-landing/src/components/Download.tsx`: the `Assets` type already
  declares `linux: { appimage?: ReleaseAsset; deb?: ReleaseAsset }`, and
  `getOptionsForOS`/`AllPlatformLinks` already have working, tested branches
  for rendering a `.deb` download button — again, dead code paths today
  because `a.linux.deb` is always `undefined`.

Three independent pieces of the pipeline were built *for* a `.deb` that was
never wired up on the producing end. Closing that gap is the lowest-risk,
highest-confidence first step of §2 — it's finishing existing scaffolding,
not adding a new concept to three files at once.

### 1.5 Downloads UX today — a homepage section, not a dedicated page

`agentmux-landing/src/index.tsx`'s router has `/`, `/vision`, `/trust`,
`/features`, `/whynow`, `/usecases`, `/privacy`, `/terms`, `/compare`,
`/sources` — **no `/download` route.** `Download.tsx` is a `<section
id="download">` rendered inline on the homepage (`App.tsx`), reached via a
`#download` anchor. It already does real work — OS auto-detection, a primary
CTA button, a compact "all other platforms" text-link row, an alpha-software
warning, an enterprise CTA — but it is homepage real estate, not a
standalone page a link can point to, and its "all platforms" row is a single
line of links, not a comparison grid. That's the structural difference from
`waveterm.dev/download` (confirmed by direct fetch, 2026-09-15): a dedicated
page with three OS sections, each listing **every** format for that OS (for
Linux specifically: `.snap`, `.deb`, `.rpm`, `.AppImage`, `.zip`, `.pacman`,
each in x64 **and** arm64), plus a Homebrew line for macOS. Checksums/GPG
signatures/release-notes links were not visibly present on their page
either, for what it's worth — §3.3 recommends AgentMux do slightly better on
that front since the plumbing already half-exists (§1.4).

---

## 2. Widening Linux package targets

### 2.1 Recommended approach: repackage a staged tree with `fpm` — but the *real* staged tree, not raw `dist/`

[`fpm`](https://github.com/jordansissel/fpm) (a well-established Ruby CLI,
`gem install fpm`, pure-Ruby + `dpkg`/`rpmbuild`/`pacman` tooling as backends)
converts one staged directory into `.deb`, `.rpm`, `.pacman`, and other
formats via `-t <format>` with no format-specific build step. This avoids the
alternative (hand-rolling `dpkg-deb`/`rpmbuild` control-file plumbing per
format, each with its own dependency-declaration syntax) for formats that
don't need anything format-specific about how AgentMux itself is laid out.
Land format-by-format so each is independently revertable if CI proves
unstable for it (§2.3's phase table).

**Correction (Codex review, PR #3234):** an earlier revision of this section
said `fpm`/`tar` could run directly over `dist/`, the flat directory
`bundle:linux` populates. That's wrong, confirmed by reading
`scripts/build-appimage-linux.sh` directly: `dist/` is raw build output, not
a runnable layout. The actual AppImage runtime input is
**`build/AgentMux.AppDir`** — a separate directory the script wipes and
reassembles on every run, relocating `dist/cef/agentmux-cef` →
`AppDir/usr/bin/agentmux-cef`, the launcher, `target/release/agentmux-mcp`,
`agentmux-srv-{version}-linux.x64`, CEF's `.so`/`.pak`/locale files, the
bundled frontend, plus `AppRun`, the `.desktop` entry, icons, and the
install/AppArmor helper scripts (see the script's own header comment for the
full expected layout) — running `fpm`/`tar` over `dist/` instead would ship
packages missing files and carrying build-tree-relative paths that don't
resolve outside the checkout. **Any new format must consume
`build/AgentMux.AppDir` (or a factored-out equivalent staging step), not
`dist/`** — see the revised §2.5 file list.

### 2.2 What's genuinely new work vs. what's already solved

| Concern | Status |
|---|---|
| Compiling AgentMux for Linux | **Already solved** — unchanged, one compile per run regardless of how many package formats it feeds (§1.3) |
| CEF/patched-`libcef.so` provisioning | **Already solved** — unchanged, x86_64 only (see §2.4 for the arm64 gap) |
| Staging a runnable tree | **Already solved, but it's `build/AgentMux.AppDir`, not `dist/`** (§2.1 correction) — the AppImage script's staging steps need factoring out into something reusable, not literally reused as-is (that step is currently interleaved with AppImage-specific work like the `.DirIcon` fix and the CEF-patch gate) |
| `.deb`/`.rpm`/`.pacman` packaging | **New** — `fpm` invocations, one per format, in `build-linux.yml` after the existing AppImage step |
| `.tar.gz` portable archive | **New but trivial** — `tar czf` over the same staged tree, no tool dependency at all; matches Windows' "portable ZIP" and waveterm's `.zip` |
| Landing-page plumbing (`fetch-release.mjs` categorization, checksums, S3 mirroring) | **Mostly already solved for `.deb`** (§1.4); needs the same few lines added for `.rpm`/`.pacman`/`.tar.gz` |
| Landing-page `Assets` type + UI (`Download.tsx`) | **Partially solved for `.deb`**; needs new fields + render branches for the other new formats |

### 2.3 Phased rollout

| Phase | Formats | Why this grouping |
|---|---|---|
| **Phase 1** | `.deb`, `.tar.gz` | `.deb` closes the exact gap in §1.4 (scaffolding exists end-to-end already); `.tar.gz` needs zero new tooling. Lowest risk, ships first, proves the `fpm`-in-CI approach before adding more formats. |
| **Phase 2** | `.rpm` | Same `fpm` mechanism as `.deb`, one more `-t rpm` invocation + dependency-name mapping (Fedora/openSUSE package names differ from Debian's for the same runtime libs — needs a real pass, not a copy-paste of the `.deb` deps list). |
| **Phase 3** | `.pacman` (Arch/AUR) | Same `fpm` mechanism again, but Arch's own community convention is a PKGBUILD in the AUR rather than a downloadable binary package on the project's own release page — worth deciding *which* of those two AgentMux wants (see OQ1) before building either. |
| **Phase 4 (larger, separate spec recommended)** | Snap, Flatpak | Structurally different from Phases 1-3 — not a repackage-the-existing-tree job. Snap needs a `snapcraft.yaml` + either self-hosted or Snap Store publishing; Flatpak needs a manifest + sandboxed build + a decision on Flathub vs. self-hosted `.flatpak` bundle distribution. Both are real, scoped efforts with their own tradeoffs (auto-update channels, store review processes) that deserve their own spec rather than being bundled into this one as an afterthought. |
| **Out of scope here, tracked as a blocker** | Linux **arm64**, any format | See §2.4 — blocked upstream, not something this spec's changes can unblock on their own. |

### 2.4 Linux arm64 — explicitly out of scope, and why

Every format above is scoped to **x86_64 only**, matching today's AppImage.
Widening architecture (not just format) requires a patched-`libcef.so`
**arm64 Linux** build from the private `agentmuxai/cef` repo — this spec's
author did not check whether that exists today (no access to that private
repo from this session), and building it if it doesn't is a substantial,
separate effort (per `docs/cef-build/CEF_FORK_MAINTENANCE.md`'s general
description of what maintaining a CEF fork build entails) unrelated to
packaging-format work. **Recommendation: file this as its own tracking
issue/spec once someone with `agentmuxai/cef` access confirms whether an
arm64 Linux patched runtime already exists**, rather than silently scoping
it into this spec's Phase 1-3 and discovering the blocker mid-implementation.

### 2.5 Files to create / modify (§2)

| File | Repo | Action |
|---|---|---|
| `.github/workflows/build-linux.yml` | agentmux | **Modify** — add `fpm`-based `.deb` (Phase 1), `.rpm` (Phase 2), `.pacman` (Phase 3) packaging steps after the existing AppImage step; add a `tar czf` step (Phase 1) for `.tar.gz`; `actions/upload-artifact` for each new asset, mirroring the existing AppImage upload pattern |
| `.github/workflows/release.yml` | agentmux | **Modify — required, not optional (Codex P1, PR #3234).** Its `publish` job's asset-flattening step (`find release-artifacts -type f \( -name "*.zip" -o -name "*.exe" -o -name "*.msix" -o -name "*.AppImage" -o -name "*.dmg" \)`, line ~338) is an **explicit extension allowlist** — a new format uploaded by `build-linux.yml` but not added to this `find` is silently excluded from the actual GitHub Release (and therefore from the landing-page deploy, which reads the Release's assets — `SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23.md` §6). Without this change, Phase 1 would ship `.deb`/`.tar.gz` as nightly-artifact-only, visible nowhere a real user downloads from. Add each new extension to this `find` in the same PR as the `build-linux.yml` change that starts producing it. |
| `scripts/build-appimage-linux.sh`, and a new shared staging script (e.g. `scripts/stage-linux-appdir.sh`) | agentmux | **Modify + create** — factor the AppDir assembly steps (§2.1 correction) out of `build-appimage-linux.sh` into something a new `scripts/build-<format>-linux.sh` per format can also call; keep the existing AppImage script's own AppImage-specific logic (the `.DirIcon` real-file fix, the CEF-patch release gate, `appimagetool` invocation) where it is — don't risk regressing the one format that already works while extracting the shared part |
| `Taskfile.yml` | agentmux | **Modify** — new `task package:linux:deb` / `:rpm` (and `:tarball`) local-build tasks, mirroring `package:linux`'s existing per-build-channel isolation conventions (`SPEC_LINUX_APPIMAGE_PER_BUILD_CHANNEL_2026_06_25.md`) so local testing doesn't require CI |
| `docs/linux.md` | agentmux | **Modify** — correct the stale `agentmuxai/agentmux-builder` reference (§1.4); document each new format's install/run instructions once shipped |
| `agentmux-landing/scripts/fetch-release.mjs` | agentmux-landing | **Modify** — add `.rpm`/`.pacman`/`.tar.gz` to `UNVERSIONED_NAMES` and `CONTENT_TYPES`, and a categorization branch per format in the asset loop (mirrors the existing, already-working `.deb` branch) |
| `agentmux-landing/src/components/Download.tsx` | agentmux-landing | **Modify** — extend the `Assets`/`ReleaseAsset` types and `getOptionsForOS`/`AllPlatformLinks` for the new formats (§3 folds this into the larger downloads-page work rather than doing it twice) |

---

## 3. A dedicated downloads page

### 3.1 What to build

A new `/download` route in `agentmux-landing` (`src/components/DownloadPage.tsx`,
registered in `src/index.tsx` alongside the other top-level pages), modeled
on `waveterm.dev/download`'s structure (§1.5): three OS sections
(Windows/macOS/Linux), each listing **every** format currently in
`release.json` for that OS — not just a primary button + compact link row.
Reuses `release.json` as the single data source (already fetched at build
time by `fetch-release.mjs`; §2's widening work is what makes this page's
Linux section actually have something to show beyond one AppImage link).

### 3.2 Relationship to the existing homepage `#download` section

**Keep `Download.tsx` on the homepage — don't remove it.** Per
`agentmux-landing/docs/download-button-refactor.md`'s own prior design
intent ("Download.tsx — it has the full multi-button layout... It's the
dedicated download section and doesn't need simplification"), the homepage
section's job is a fast, OS-detected, one-click path for the common case.
The new `/download` page's job is the comprehensive comparison view for
someone who wants a specific format/architecture, arrived at via:
- A new "See all download options →" link inside the existing homepage
  section (small addition to `Download.tsx`, not a redesign of it).
- A `Download` entry in `Nav.tsx`'s nav bar pointing at `/download` — today
  `Nav.tsx` (per `download-button-refactor.md`) uses the reusable
  `<DownloadButton>` component (an OS-detected direct-download link, not a
  navigation link); recommend adding a separate plain nav link to `/download`
  alongside it rather than repurposing the existing button (open question,
  OQ2 — could also be a discoverability regression if the direct-download
  button in the nav bar is removed in favor of a navigation link).

### 3.3 What the new page should include that neither `waveterm.dev/download` nor today's `Download.tsx` currently surfaces

`fetch-release.mjs` already computes a SHA-256 per asset and uploads a
combined `checksums.sha256` manifest to S3 (`releases/latest/checksums.sha256`,
`releases/v<version>/checksums.sha256`) — **but nothing in the UI links to
it or displays the per-asset hash today.** Since this is already computed
and already hosted, surfacing it on the new page (a small "sha256:
`<hash>`" line per download row, copy-to-clipboard, plus a link to the full
manifest) is close to free and is a real security/trust improvement over
both the current homepage section and the waveterm reference page (which,
per §1.5's fetch, doesn't visibly show checksums either). Also surface:
version number + published date (already in `release.json`), and a link to
the matching `VERSION_HISTORY.md` section / GitHub Release notes for that
version (the release notes link waveterm's page was also missing).

### 3.4 Files to create / modify (§3)

| File | Repo | Action |
|---|---|---|
| `src/components/DownloadPage.tsx` | agentmux-landing | **Create** — the new dedicated page, three-OS-section grid, sourced from `release.json` |
| `src/index.tsx` | agentmux-landing | **Modify** — register `<Route path="/download" component={DownloadPage} />` |
| `src/components/Download.tsx` | agentmux-landing | **Modify** — add "See all download options →" link to `/download` (small addition, §3.2) |
| `src/components/Nav.tsx` | agentmux-landing | **Modify** — add nav link to `/download` (§3.2, OQ2 needs resolving first) |
| `src/components/Download.tsx`, `DownloadButton.tsx`, `DownloadPage.tsx` | agentmux-landing | **Modify/create** — shared `Assets`/`ReleaseAsset` type extended for §2's new formats (one shared type, not redefined per file — check whether `platform.ts` or a new `lib/releaseTypes.ts` should own it, since it's currently duplicated inline in both existing files per `download-button-refactor.md`'s own noted duplication problem; don't add a third inline copy) |
| `scripts/fetch-release.mjs` checksum output | agentmux-landing | **No structural change needed** — already produces what §3.3 needs; the work is entirely on the display side |

---

## 4. Sequencing recommendation

1. **§2 Phase 1** (`.deb` + `.tar.gz`) first, landing-page plumbing included
   (categorization + type + at least the existing homepage section's
   rendering) — proves the `fpm`-in-CI approach end to end on the format
   with the least new surface area, and immediately closes the three-places
   dead-code gap in §1.4.
2. **§3** (dedicated downloads page) next, once Phase 1 gives it more than
   one Linux format to actually display — building the comparison-grid page
   against a one-format Linux section would understate what it's for.
3. **§2 Phases 2-3** (`.rpm`, `.pacman`) as follow-up PRs, each independently
   shippable, each immediately visible on the already-built downloads page
   with no further landing-page work beyond the categorization line each
   adds (§2.5).
4. **§2 Phase 4** (Snap/Flatpak) and **§2.4** (arm64) as their own
   follow-up specs, not implemented here.

---

## 5. Open questions

| # | Question | Notes |
|---|---|---|
| OQ1 | For `.pacman` (Phase 3): ship a downloadable `.pacman` package on AgentMux's own release page, or a PKGBUILD submitted to the AUR (the convention most Arch users actually expect)? | These have different maintenance models (AUR PKGBUILDs are typically community-maintained and versioned separately from upstream releases) — needs a decision before Phase 3 work starts, not during it. |
| OQ2 | Does adding a `/download` nav link replace or sit alongside the existing OS-detected `<DownloadButton>` in `Nav.tsx`? | Recommend alongside (add, don't replace) — removing the one-click OS-detected button in favor of a navigation link is a discoverability regression for the common case §3.2 explicitly wants to keep fast. |
| OQ3 | Is `fpm` an acceptable new CI dependency (`gem install fpm` on `ubuntu-22.04`), or does the team prefer hand-rolled `dpkg-deb`/`rpmbuild` invocations to avoid a third-party packaging tool in the release pipeline? | §2.1's recommendation; flagging as a real choice since `fpm` is unmaintained-adjacent (still functional and widely used, but slow-moving) — worth a deliberate yes/no rather than silent adoption. |
| OQ4 | Should Phase 1's `.deb` declare real `Depends:` entries (e.g. against whatever GTK/Wayland/X11 libs the AppImage bundles itself and a `.deb` normally wouldn't need to), or bundle everything statically the way the AppImage does? | Affects both package size and whether `apt install ./agentmux.deb` can fail on a minimal system — needs a decision informed by what the AppImage actually bundles vs. what a `.deb` conventionally expects the system to provide. |
| OQ5 | Who actually has push access / a valid arm64 CEF question to ask against `agentmuxai/cef` (§2.4)? | This spec's author had no access to check; blocks even scoping the arm64 follow-up spec until someone with access answers it. |
