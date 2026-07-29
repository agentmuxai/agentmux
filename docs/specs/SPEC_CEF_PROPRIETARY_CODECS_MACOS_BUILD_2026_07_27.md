# Spec: Execute the macOS leg of issue #2311 (codec-enabled patched CEF)

**Status:** Draft — execution runbook, no build run yet.
**Author:** AgentO
**Date:** 2026-07-27
**Issue:** [agentmuxai/agentmux#2311](https://github.com/agentmuxai/agentmux/issues/2311)
("Rebuild patched CEF with proprietary codecs — Linux + macOS"), assigning
**macOS** to AgentO (this agent) and Linux to AgentU. Windows is Agent2's,
already implemented in PR #2308 (merged).
**Related:** `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`
(root cause), `docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md`
(3-platform design, PR #2308 — already merged, GN args already updated),
`docs/cef-build/build-patched-framework-macos.md` (the existing manual build
process this extends), `docs/specs/SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md`.

## What's already done (no action needed here)

PR #2308 is merged into `main` (confirmed, pulled today). `scripts/cef-build/args-darwin.gn`
already carries the codec block:

```
proprietary_codecs=true
ffmpeg_branding="Chrome"
enable_hevc_parser_and_hw_decoder=true
enable_platform_ac3_eac3_audio=true
enable_platform_dolby_vision=true
```

Nothing in `agentmux` needs to change for this leg — this spec is purely the
**execution** (rebuild → verify → cut release) that PR #2308 explicitly left as
a manual follow-up (§4 of the all-platforms spec).

## Current machine state (this Mac already has a warm build tree — not a cold start)

This is the same Mac referenced in `[[macos-release-packaging]]` (signing/notarizing).
It turns out it *also* already has a full prior CEF build tree at `~/cef-build/`,
left over from earlier work (`BUILD_PLAN.md` there documents an unrelated
`-67030` process-requirement fix, already shipped in 0.40.1 — not to be confused
with this task). Confirmed today:

| Item | Finding |
|---|---|
| `~/cef-build/depot_tools`, `~/cef-build/chromium/chromium/src` | Present, 140 GB. Chromium/CEF already checked out and previously built successfully — **this is a rebuild, not a fresh 3-6h/100GB checkout+build.** |
| `~/cef-build/chromium/chromium/src/cef` branch | `amux-transp-mac`, tracking `agentmux/agentmux/7778-drag-rightclick-and-transparency` (the correct patch branch — HEAD is a Views-transparency commit, unrelated to codecs, as expected: codecs are pure GN flags, no source patch needed). |
| `~/cef-build/darwin/arm64` (staged framework `resolve-cef-runtime-darwin.sh` tier-2 picks up) | Version **148.23.23.0** — pre-codec, current production framework. |
| `~/cef-build/chromium/chromium/src/out/Release_GN_arm64/args.gn` | Currently installed args **predate the codec block** (last written by `rebuild-mac-cef.sh` on 2026-06-30, before PR #2308 existed) — must be refreshed from the repo's `args-darwin.gn`. |
| `gh release list --repo agentmuxai/cef` (macOS tags) | Latest is `cef-macos-arm64-148.23.21` (2026-07-02) — **not** `148.0.9` as `build-patched-framework-macos.md`'s "verified artifact" table still claims. That doc is stale; worth a follow-up correction (out of scope here, noted for later). |
| Last rebuild attempt (`~/cef-build/rebuild-mac-cef.log`, 2026-06-29) | **Failed all 3 attempts**: `ninja: error: unknown target 'cef', did you mean 'ced'?`. Root cause below. |

### Bug found: wrong ninja target name, in two places

`docs/cef-build/build-patched-framework-macos.md`'s own "Build" section says:

> Build the framework target (**NOT** the phony `cef` meta-target, which won't
> relink after a source-only change).

...then gives the command `ninja ... -C out/Release_GN_arm64 cef` — using the
exact target the comment says not to use. This is a real inconsistency in the
doc (the correct target, confirmed by `~/cef-build/rebuild-mac-cef.sh`'s own
`build_once()`, is **`cef_framework`**, not `cef`), and it's likely what killed
the 2026-06-29 rebuild attempt (its log shows target `cef` was invoked, matching
the doc's broken command rather than the script's own correct one — the script
must have been edited after that run, or a manual command was used instead).
**Action:** always target `cef_framework`; fix the doc's command line as a
one-line correction alongside this work.

### Stale paths in the existing rebuild script

`~/cef-build/rebuild-mac-cef.sh` hardcodes:
```
ARGS_SRC="/Users/asafebgi/.agentmux/agents/maop-06067/agentmux/scripts/cef-build/args-darwin.gn"
VERIFY="/Users/asafebgi/.agentmux/agents/maop-06067/agentmux/scripts/verify-cef-framework-darwin.sh"
```
`maop-06067` was a **different, prior agent workspace** (not this one,
`masty-06136`) — presumably still valid on disk, but pointing at a stale
checkout of `agentmux` rather than the one just pulled to latest `main` in this
session. **Action:** repoint both to this workspace
(`/Users/asafebgi/.agentmux/agents/masty-06136/agentmux/scripts/...`) before
running, so the codec-updated `args-darwin.gn` is actually the one installed.

## Plan

1. **Fix `~/cef-build/rebuild-mac-cef.sh`** — update `ARGS_SRC`/`VERIFY` to this
   workspace's paths (above).
2. **Run the script's stages 1–6, with one fix** (refresh patch branch to fork
   tip, `gclient sync --nohooks` non-destructively, `runhooks`, mirror `cef/`
   into `src/cef`, re-apply the CEF patches, regenerate C-API wrappers). This
   is the same "pull latest, re-apply patches" step the doc already documents
   as safe/idempotent-ish; it's what keeps this a *rebuild* rather than a
   fresh checkout. **Do not run the patch-apply stage as currently written in
   `rebuild-mac-cef.sh`** — its `python3 cef/tools/patcher.py --root-dir=cef`
   call uses a nonexistent flag, errors immediately, and the script's
   `|| echo WARN` swallows that as non-fatal, silently applying **zero**
   patches (including `BeginWindowDrag`) while the build proceeds anyway. Use
   the plain default form instead: `cd cef && python3 tools/patcher.py` (no
   args — it reads `patch/patch.cfg` itself). Confirmed working: 112 applied,
   3 skipped, 0 failed. Verify with `git status` in `chromium/src` before
   starting the long build — a clean tree there means no patches landed.
3. **Install the refreshed `args-darwin.gn`** (with the codec block) →
   `gn gen out/Release_GN_arm64`. Verify with `grep` that `proprietary_codecs`
   etc. actually landed in the generated `args.gn` before starting the build
   (cheap sanity check the script doesn't currently do).
4. **Build with `ninja -j6 -l8 -C out/Release_GN_arm64 cef_framework`** (never
   the bare `cef` target — see bug above). RAM constraint from the script's own
   header comment stands: this box is 8 cores / 24 GB, so `is_official_build`
   stays `false` (LTO link OOMs at 24 GB) — codec flags don't change that
   constraint, only the media/ffmpeg subtree needs recompiling, so this should
   be meaningfully shorter than a full fresh build, though the exact time isn't
   known until it's run.
5. **Verify the existing patch is intact:**
   `scripts/verify-cef-framework-darwin.sh` on the fresh unstripped output —
   must still exit 0 (`BeginWindowDrag` present). This only checks the window-drag
   patch, not codecs — expected, see step 6.
6. **New: codec playback sanity check** (issue's explicit new ask, no existing
   tooling covers this):
   - Point a local dev/package build at the fresh framework:
     `AGENTMUX_CEF_RUNTIME_DIR_DARWIN=~/cef-build/chromium/chromium/src/out/Release_GN_arm64 task dev`
     (or `task package:macos` for a fuller check, per
     `[[macos-release-packaging]]` for the packaging invocation).
   - Open a Media pane (or any `<video>`) against a real H.264/AAC MP4 (the
     codec-gap report's repro used a plain consumer MP4 — any similar file
     works; a WebM won't exercise anything new since that path already worked).
   - Pass: video plays. Fail signature to watch for:
     `PipelineStatus::DEMUXER_ERROR_NO_SUPPORTED_STREAMS` (the exact error from
     the codec-gap report) — if this reappears, the codec flags didn't take
     effect in this build and the `args.gn` sanity check from step 3 should be
     re-checked first.
7. **Determine actual `CEF_VERSION`** from the fresh build's
   `out/Release_GN_arm64/gen/cef/include/cef_version.h` (or the packaged
   framework's `Info.plist` `CFBundleShortVersionString`) — do not assume
   `148.0.9` or `148.23.23.0`; read it from this build.
8. **Cut the release**, following `build-patched-framework-macos.md`'s existing
   "Package + upload" section as-is (unstripped tar.gz, same naming
   convention): tag `cef-macos-arm64-<CEF_VERSION>`. Since the source tree is
   already synced past `148.23.21` (the latest existing tag), this build will
   likely land on the **same or a very close** CEF version as that already-published
   release — **use the `-codecs` suffix**
   (`cef-macos-arm64-<CEF_VERSION>-codecs`) rather than risk clobbering it,
   per the issue's own disambiguation instruction.
9. **Stage locally too:** `ditto` the fresh framework into
   `~/cef-build/darwin/aarch64` — **not** `arm64`. Confirmed by reading
   `scripts/resolve-cef-runtime-darwin.sh`: it hard-requires `aarch64` (Rust's
   `target_arch` naming) at tier 2 and explicitly calls out `arm64` as the
   wrong name that "silently misses" resolution — `rebuild-mac-cef.sh` already
   gets this right (`STAGE="$ROOT/darwin/aarch64"`), but the pre-codec
   framework currently sitting at `~/cef-build/darwin/arm64` is *not* on the
   resolver's tier-2 path at all (only `~/cef-build/darwin/aarch64` is used;
   `arm64` is dead weight from an earlier mistake — worth pruning separately).
10. **Comment on issue #2311** (and/or PR #2308) once the release tag is cut,
    per the issue's step 6 — no CI changes needed, `build-macos.yml` picks up
    the new tag automatically on next run.

## Follow-up doc fixes (small, unrelated to the main plan, found during this pass)

- `docs/cef-build/build-patched-framework-macos.md`: fix the `ninja ... cef`
  command to `ninja ... cef_framework` (bug above).
- Same doc's "verified artifact" table and "Version skew with Linux" section
  are stale (claims `148.0.9`; actual latest published tag is `148.23.21`) —
  update once the new codec build's real version is known.

## Non-goals

- Not touching Linux (AgentU's leg) or Windows (Agent2's, already done).
- Not re-deriving the GN flag rationale — already settled in PR #2308's spec.
- Not automating this build in CI — same accepted constraint as the original
  all-platforms spec's non-goals (manual build, by design).

## Open item before executing

Steps 1–10 above are a multi-hour, real-resource operation on this machine
(rebuild + local dev/package verification + a `gh release create` against
`agentmuxai/cef`, a shared distribution channel other CI depends on). This
spec documents the plan; **executing it (especially cutting the release) should
get an explicit go-ahead** rather than running unattended, consistent with how
release cuts are handled elsewhere in this project.
