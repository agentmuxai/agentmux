# Spec: CEF proprietary codec support (H.264/AAC) across Windows/macOS/Linux

**Status:** implemented — Windows #2308, macOS rebuild (tag cef-macos-arm64-148.23.23-codecs) closed via #2347/#2399, Linux release cut, CI resolver #2353; H.264/AAC verified on all three platforms. Verified 2026-08-10.
**Author:** Agent2
**Date:** 2026-07-26
**Related:** `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`
(the finding that triggered this spec — MP4/MOV fail to play anywhere in
the app on the current CEF build), `docs/cef-build/build-patched-libcef.md`
(Linux build process this extends), `docs/cef-build/build-patched-framework-macos.md`
(macOS counterpart), `docs/specs/patched-libcef-bundling-2026-05-08.md`
(the Linux CI-sourcing design this generalizes to Windows),
`docs/specs/SPEC_BUILDER_MACOS_LINUX_CI_2026_06_24.md`,
`docs/specs/SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md` (contains
a stale claim corrected here — see "Doc fix" below).

## Key finding — corrects the original request's premise

The request that prompted this spec was "open PRs on `agentmuxai/cef` for
any updates." **Based on two independent research passes, no source-code
PR against `agentmuxai/cef` is actually needed for proprietary codec
support**, and it's worth explaining precisely why before describing what
*is* needed, since it changes where the real work happens:

- `proprietary_codecs=true` / `ffmpeg_branding="Chrome"` are **GN build
  arguments**, not a source patch. They gate whether Chromium's build
  system *compiles in* H.264/AAC decoder code that already exists in the
  public, open-source FFmpeg/Chromium source tree — unlike the existing
  `CefWindow::BeginWindowDrag()` patch (a real new C++ API surface that
  had to be added to CEF's source and its Rust bindings), codec support
  needs zero source changes to CEF itself.
- Confirmed: `agentmuxai/cef` has **zero CI** (`gh api
  repos/agentmuxai/cef/actions/workflows` → `{"total_count": 0}`) and its
  own contributors' own documented convention (see PR #6 on that repo,
  "add `CefBrowserHost::SetZoomIsolated()`") treats "rebuild and cut a new
  binary release" as a distinct, deliberately-separate, manual step from
  any source PR — never something a PR itself triggers or completes.
- The GN args that *do* need changing are already version-controlled —
  **in this repo**, not the fork: `scripts/cef-build/args.gn` (Linux),
  `scripts/cef-build/args-darwin.gn` (macOS). Adding the codec flags there
  is a normal PR against `agentmux`, exactly like any other change in this
  spec.
- What the fork's GitHub Releases mechanism actually is: a **distribution
  channel for a compiled binary**, cut by hand
  (`gh release create cef-<platform>-<ver> ...`) after someone runs the
  multi-hour build locally — never a PR. This is confirmed as the
  deliberate, accepted design (not a stopgap) in `SPEC_BUILDER_MACOS_LINUX_CI_2026_06_24.md`
  §5 and `SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md` §8, both
  of which explicitly list "build the binary in CI" as out of scope.

**What this means concretely:** the "PR to `agentmuxai/cef`" step doesn't
exist as a distinct action for this feature. The real sequence is (1) a PR
against `agentmux` changing the GN args + adding the missing Windows
build infrastructure (this spec's actual deliverable), (2) someone with
build capacity manually runs three platform builds with the new args, (3)
that person cuts three new tagged Releases on `agentmuxai/cef`, (4) CI
(already wired for Linux/macOS, newly wired for Windows by this spec)
picks them up automatically from there. Flagging this now rather than
silently reinterpreting the request — worth confirming before proceeding.

## Doc fix (small, unrelated to the main plan, found during research)

`docs/specs/SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md` line
283 states Windows "already ships its own patched CEF via the launcher
bundle." This is stale/incorrect — confirmed at every layer (Taskfile
tasks, all CI workflows, resolver scripts, the codec-gap report filed
today) that Windows uses the plain stock `cef-dll-sys` binary with zero
override capability anywhere. Worth a one-line fix alongside this work.

## Confirmed current architecture (Linux/macOS precedent to extend)

```
maintainer's own machine (manual, 3-6h, ~100GB disk)
  chromium+cef checkout on agentmuxai/cef's agentmux/7778-drag-rightclick-and-transparency branch
  → scripts/cef-build/configure-cef-build.sh installs scripts/cef-build/args.gn, runs `gn gen`
  → ninja build (OOM-resistant wrapper)
  → tar the output
  → `gh release create cef-linux-x86_64-<ver> --repo agentmuxai/cef file.tar.gz` (by hand)
                                    │
                                    ▼
CI (build-linux.yml / build-macos.yml / release.yml / ci-nightly-artifacts.yml):
  gh release list --repo agentmuxai/cef --jq 'filter by cef-linux-x86_64-* or cef-macos-arm64-*'
  → gh release download (cached by tag via actions/cache)
  → export AGENTMUX_CEF_RUNTIME_DIR / AGENTMUX_CEF_RUNTIME_DIR_DARWIN
  → app build/package steps consume the patched binary from there
```

**Important existing-design nuance to preserve, not "fix":** the two
compile-check gates (`ci-pr.yml`, `ci-nightly-build.yml`) deliberately use
plain stock CEF for *every* platform, including Linux/macOS — they never
set the resolver env vars. Only the packaging-grade workflows
(`release.yml`, `build-linux.yml`, `build-macos.yml`,
`ci-nightly-artifacts.yml`) fetch the patched binary. This is a sensible
existing split (PR compile-checks shouldn't pay a multi-hundred-MB
download + cache cost on every push) — the codec-enabled binary should
slot into exactly the same split, not change it.

**Windows today has none of this** — confirmed at every layer:
`Taskfile.yml`'s `build:host:windows`/`bundle:windows` tasks, every CI
workflow's Windows job, and a full-repo grep for
`AGENTMUX_CEF_RUNTIME_DIR` (17 hits, zero Windows). Windows sources DLLs
directly from `target/release/`, populated by `cef-dll-sys`'s own
build.rs downloading the stock binary — by design, not oversight, since
Windows never needed the `BeginWindowDrag` patch (native drag already
works there via Win32).

## Plan

### 1. GN args — expanded for wide file-type coverage, not just H.264/AAC

Confirmed directly against Chromium's own `media/media_options.gni`
source (the authoritative declaration site, not a community guide) that
`proprietary_codecs=true` alone doesn't turn everything on for a desktop
build:

| Flag | Default when `proprietary_codecs=true` | Needed here? |
|---|---|---|
| `enable_hevc_parser_and_hw_decoder` (H.265/HEVC) | `... \|\| is_win \|\| is_apple \|\| is_android \|\| is_linux` — **already true on all 3 of our platforms** | Set explicitly anyway, for documentation/clarity — costs nothing, makes the args file self-describing rather than relying on an unstated default |
| `enable_platform_ac3_eac3_audio` (Dolby Digital/Plus audio — common in MP4/MKV rips) | `... \|\| is_cast_media_device \|\| (is_ios && tvos)` — **false on desktop Win/Mac/Linux by default** | **Yes, explicit `true` required** — this is exactly the kind of gap that would otherwise silently limit format coverage |
| `enable_platform_dolby_vision` (HDR video profile; requires HEVC) | `... \|\| is_cast_media_device \|\| is_win` — true by default on Windows only | **Yes, explicit `true`** on macOS/Linux too, for parity across all three platforms' args files |
| `media_use_openh264` (H.264 *encoding*, not playback) | `true` by default once proprietary | No change needed — already on |

Open/unencumbered codecs (VP8/VP9/AV1, Opus/Vorbis/FLAC, Theora, WAV/PCM)
need no flags at all — always compiled in regardless of
`proprietary_codecs`. MP3 is bundled under the `proprietary_codecs` gate
itself (Chromium's own arg description: *"Enables proprietary codecs and
demuxers; e.g. H264, AAC, MP3, and MP4"*), no separate flag.

**Not included, deliberately:** DTS, MPEG-H, and other codecs with no
open Chromium implementation at all — no GN flag unlocks these; Chromium
simply doesn't have a decoder for them, licensed or otherwise. "Wide
coverage" here means everything Chromium's own media stack is *capable*
of, not literally every codec that exists.

Add to `scripts/cef-build/args.gn`, `scripts/cef-build/args-darwin.gn`,
and the new `scripts/cef-build/args-windows.gn` (§2 below):

```
proprietary_codecs=true
ffmpeg_branding="Chrome"
enable_hevc_parser_and_hw_decoder=true
enable_platform_ac3_eac3_audio=true
enable_platform_dolby_vision=true
```

Both files already carry `enable_widevine=true` — a precedent for
knowingly including a licensed/gated component, so this isn't a new kind
of decision for this codebase, just a new instance of one already made.
Bump the version-in-filename convention these builds already use (e.g.
`cef-linux-x86_64-148.0.20-3` → the next suffix) so the codec-enabled
binary is distinguishable from the current one in the release list, and
so CI's existing "resolve latest tag by platform prefix" logic picks it
up automatically without needing to change the *selection* logic itself
— only the *content* of what gets tagged next changes.

### 2. Windows — new build infrastructure (doesn't exist today)

Windows needs everything Linux/macOS already have, built fresh:

- **`scripts/cef-build/args-windows.gn`** — new file. Base: standard CEF
  Release GN args (mirroring the shape of `args.gn`/`args-darwin.gn`) plus
  `proprietary_codecs=true ffmpeg_branding="Chrome"`. Does **not** need
  the Linux/macOS size-reduction block's rationale re-derived from
  scratch — copy the `is_official_build=true` reasoning, it applies
  identically on Windows.
- **`docs/cef-build/build-patched-cef-windows.md`** — new doc, Windows
  counterpart to the two existing per-platform build docs. Same
  chromium/depot_tools/GN mechanics, Windows-specific toolchain
  prerequisites (Visual Studio, Windows SDK — same ones this repo's own
  `agentmux-cef` build already requires, per `CLAUDE.md`'s Build
  Prerequisites section).
- **Open question this doc should not silently resolve — needs a decision
  before implementation:** should the Windows build come from the *same*
  `agentmux/7778-drag-rightclick-and-transparency` fork branch (inert but
  harmless there, since nothing on Windows calls `BeginWindowDrag` — the
  Rust binding side already compiles fine against the patched signature
  per the CI research's "moot today only because Windows never calls it"
  note), or from plain upstream `chromiumembedded/cef` branch `7778` with
  no fork involvement at all? Recommending the former (one canonical
  source branch across all three platforms, and it costs nothing
  functionally on Windows today) but this is a real judgment call, not
  something to pick silently.
- **`scripts/resolve-cef-runtime-windows.ps1`** — new PowerShell resolver,
  same three-tier priority as the bash ones (`$AGENTMUX_CEF_RUNTIME_DIR_WINDOWS`
  env override → standard local build-output layout → stock
  cef-dll-sys cache fallback).
- **`Taskfile.yml` changes** — `build:host:windows` and `bundle:windows`
  currently have no override capability at all (unlike
  `build:host:linux`'s `--features patched-libcef` gate). Add an
  equivalent env-var-driven redirect so these tasks source DLLs from the
  resolver's output when set, falling back to today's
  `target/release/`-sourcing behavior otherwise — same "no behavior
  change when the override is absent" guarantee the Linux/macOS resolvers
  already provide.

### 3. CI wiring

- **New `build-windows.yml`** (standalone + reusable, mirroring
  `build-linux.yml`/`build-macos.yml`'s exact structure): resolve latest
  `cef-windows-x86_64-*` tag from `agentmuxai/cef` → cache → download →
  export `AGENTMUX_CEF_RUNTIME_DIR_WINDOWS` → package. This also finally
  gives Windows a dedicated single-platform rebuild workflow, which
  doesn't exist today (the deleted `agentmux-builder` repo's
  `build-windows.yml` had zero runs ever and isn't recoverable — this
  isn't resurrecting it, it's a fresh equivalent).
- **`release.yml`**: change the `build-windows` job to call the new
  reusable workflow the same way `build-linux`/`build-macos` jobs already
  do, instead of its current inline `runs-on: windows-latest` steps with
  no CEF provisioning.
- **`ci-nightly-artifacts.yml`**: same change to its Windows job (which
  currently duplicates plain stock-CEF Windows packaging inline, matching
  the pattern its Linux/macOS jobs already had before — those are inline
  copies too, not reused; this spec doesn't need to fix that pre-existing
  duplication, just add the Windows CEF-provisioning block consistent
  with how it's inlined for the other two platforms there today).
- **Deliberately unchanged: `ci-pr.yml`, `ci-nightly-build.yml`.** Per the
  "existing-design nuance" above — these stay stock-CEF-for-everyone,
  matching current Linux/macOS behavior in those same two gates.

### 4. What actually gets built, by whom

This spec's PR(s) against `agentmux` make all of the above *ready* —
GN args, Windows scripts/docs, CI wiring. **None of it produces a
codec-enabled binary by itself.** Three manual builds still need to
happen exactly as Linux/macOS builds do today (3-6h, ~100GB disk each,
one per platform — macOS specifically requires real Apple hardware),
followed by three `gh release create` calls on `agentmuxai/cef`. This
session's environment can write and verify all the code/config/doc
changes, but cannot itself execute those three builds — no compute for
a from-scratch Chromium compile on any of the three platforms, and macOS
additionally requires physical Apple hardware this environment doesn't
have. That gap is inherent to the existing, deliberate design (per
finding #5 in the CI research: nobody has ever proposed or built
automation for the build step itself) — not something introduced by this
plan, and not something this spec attempts to solve.

## Non-goals

- **Not** automating the Chromium build itself in CI (self-hosted runner,
  Docker-based build container, scheduled job). Confirmed nobody has ever
  proposed this for Linux/macOS either, despite the identical multi-hour/
  huge-disk cost existing there today — treating it as a durable, accepted
  constraint, matching existing precedent, not a gap this spec should
  close.
- **Not** the server-side transcode-on-serve alternative floated in the
  codec-gap report — that's an independent, smaller-scope option; this
  spec is specifically "do the full rebuild-based fix," per explicit
  direction to proceed with it now (with the CEF-fork step corrected as
  described above).
- **Not** re-litigating whether to enable proprietary codecs at all
  (licensing considerations already discussed in the codec-gap report) —
  taking that as a decided starting point for this spec.

## Open questions requiring a decision before implementation

1. **Windows build source**: fork branch `agentmux/7778-drag-rightclick-and-transparency`
   (recommended) vs. plain upstream `chromiumembedded/cef` `7778` — see
   "Plan" §2 above.
2. **Release tag scheme for codec-enabled binaries**: bump the existing
   per-platform tag suffix (e.g. `cef-linux-x86_64-148.0.20-4`) so CI's
   existing "latest tag by prefix" resolution just naturally picks up the
   new one, or introduce an explicit `-codecs` suffix
   (`cef-linux-x86_64-148.0.20-codecs`) so a codec build and a
   non-codec build could coexist as distinct, independently-selectable
   releases during a transition period? The former is simpler and matches
   existing convention; the latter allows a slower, safer rollout (verify
   the codec build widely before it becomes "the" default CI picks up).
3. **Rollout order**: build+verify Linux first (cheapest/fastest platform
   to iterate on, per the existing docs' own framing), then macOS, then
   Windows — or all three in parallel once someone with build capacity
   for each is lined up? Affects nothing about the code changes in this
   spec, only the sequencing of the manual-build step in §4.

## Files

| File | Relevance |
|------|-----------|
| `scripts/cef-build/args.gn` | Linux GN args — add codec flags |
| `scripts/cef-build/args-darwin.gn` | macOS GN args — add codec flags |
| `scripts/cef-build/args-windows.gn` (new) | Windows GN args — new file |
| `scripts/cef-build/configure-cef-build.sh` | Linux configure script — Windows needs an equivalent (or a cross-platform rewrite) |
| `docs/cef-build/build-patched-cef-windows.md` (new) | Windows build instructions |
| `docs/cef-build/build-patched-libcef.md`, `build-patched-framework-macos.md` | Existing Linux/macOS build docs — precedent to mirror |
| `scripts/resolve-cef-runtime.sh`, `scripts/resolve-cef-runtime-darwin.sh` | Existing resolvers — precedent for the new Windows one |
| `scripts/resolve-cef-runtime-windows.ps1` (new) | Windows resolver |
| `Taskfile.yml` (`build:host:windows`, `bundle:windows`, `build:host:linux`'s `patched-libcef` feature gate for reference) | Needs the same override capability Linux already has |
| `.github/workflows/build-linux.yml`, `build-macos.yml` | Precedent for the new `build-windows.yml` |
| `.github/workflows/build-windows.yml` (new) | New reusable workflow |
| `.github/workflows/release.yml` (`build-windows` job) | Switch to calling the new reusable workflow |
| `.github/workflows/ci-nightly-artifacts.yml` (Windows job) | Add the same inline CEF-provisioning block Linux/macOS jobs already have there |
| `agentmux-cef/Cargo.toml` (`patched-libcef` feature) | Reference for whether Windows needs an equivalent feature gate (likely not, since no FFI-level source patch is needed for codecs — worth confirming during implementation, not assumed here) |
| `docs/specs/SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md:283` | Stale Windows claim to correct |
