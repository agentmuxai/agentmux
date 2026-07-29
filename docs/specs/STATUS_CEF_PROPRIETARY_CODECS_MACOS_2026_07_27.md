# Status: macOS codec-enabled patched CEF rebuild (issue #2311)

**Author:** AgentO
**Started:** 2026-07-27
**Plan:** `docs/specs/SPEC_CEF_PROPRIETARY_CODECS_MACOS_BUILD_2026_07_27.md`
**Issue:** [agentmuxai/agentmux#2311](https://github.com/agentmuxai/agentmux/issues/2311)

Live-updated as this runs. Not a design doc — see the spec above for the plan
and the "what/why" (root cause, GN flags, etc. are in
`docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md` and
`docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md`).

## Progress

- [x] Pulled `agentmux` to latest `main` (PR #2308 merged — codec GN flags
      already in `scripts/cef-build/args-darwin.gn`).
- [x] Fixed stale paths in `~/cef-build/rebuild-mac-cef.sh` (`ARGS_SRC`/`VERIFY`
      were pointing at a different, prior agent workspace `maop-06067`; now
      point at this workspace `masty-06136`).
- [x] Confirmed the resolver's actual tier-2 staging dir is
      `~/cef-build/darwin/aarch64` (not `arm64` — `arm64` exists on disk too but
      is dead weight; the resolver script explicitly requires Rust's
      `target_arch` naming). Corrected this in the plan doc.
- [x] Refresh patch branch (fork tip advanced `2720ba103` → `6c570e249`) +
      `gclient sync --nohooks` + `runhooks`.
- [x] Re-apply CEF patches — **hit a real bug**: `rebuild-mac-cef.sh`'s
      `patcher.py --root-dir=cef` invocation errors (`no such option:
      --root-dir` — that flag doesn't exist on this patcher.py). The script
      logged this as a non-fatal `WARN:` and continued, meaning **all 112 CEF
      patches (including `BeginWindowDrag`) silently failed to apply** and the
      build would have proceeded on unpatched upstream source. Caught before
      the long build started by checking `git status` in `chromium/src` (clean
      tree = no patches applied). Fixed by running the plain default form
      instead: `cd cef && python3 tools/patcher.py` (no args — it reads
      `patch/patch.cfg` itself). Result: **112 applied, 3 skipped, 0 failed.**
      `rebuild-mac-cef.sh` needs this same fix before its next use.
- [x] Install codec-enabled `args-darwin.gn` + `gn gen` (re-run after patching
      so patched `BUILD.gn` files are picked up — 30124 targets, `build.ninja`
      present).
- [x] Sanity-checked codec flags landed in generated `args.gn`
      (`proprietary_codecs=true` etc. all present).
- [x] Build `cef_framework` (ninja) — succeeded on the restart (run 2); see
      "Run 2" below.
  - Run 1 (started 2026-07-27 ~10:52 PDT): appeared to finish (background-task
    notification said "completed, exit code 0"), but that was a **false
    positive** — the invocation piped through `tee` without `pipefail`, so the
    reported exit code was `tee`'s (0), not ninja's. The actual log showed
    ninja died at **26388/29397 (89.7%)** on `LINK v8_context_snapshot_generator`
    (`clang++: error: no such file or directory:
    '.../librnn_vad.a'`), preceded by `ninja: warning: premature end of file;
    recovering` on a `-t query` — evidence the `.ninja_deps` bookkeeping file
    in this long-reused build tree had pre-existing corruption (unrelated to
    today's changes — this out-dir has been through several distinct builds
    over months per `BUILD_PLAN.md`'s own history).
  - Attempted repair: `ninja -t recompact`. Side effect (confirmed via
    `ninja -n cef_framework` dry-run afterward): ninja now considers ~26,000 of
    ~29,000 steps dirty — effectively a near-full rebuild. Checked: the actual
    `.o`/`.a` objects from run 1 are still on disk with correct timestamps
    (e.g. `rnn_vad/rnn.o`, `features_extraction.o` from 2026-07-27 11:3x) — this
    is a **lost-bookkeeping** problem (ninja's per-file header-dependency
    records), not lost compiled work, but vanilla ninja has no way to trust an
    existing output without that record, so it recompiles anyway. No backup of
    the pre-recompact `.ninja_deps` was taken first — a mistake; should have
    copied it before running `-t recompact`.
  - Run 2 (started 2026-07-28 ~01:05 PDT, this time with `set -o pipefail` so
    the real exit code is captured): `ninja -j6 -l8 -C out/Release_GN_arm64
    cef_framework`, log at `~/cef-build/ninja-cef-framework-codecs-run2.log`.
    **Completed successfully 2026-07-28 ~16:05 PDT** — `NINJA_REAL_EXIT_CODE=0`
    confirmed (real exit code this time, not masked by `tee`), all
    28797/28797 steps done, zero `FAILED` lines. Output: `Chromium Embedded
    Framework.framework`, 547 MB unstripped, Mach-O 64-bit arm64.
- [x] Verify `BeginWindowDrag` patch survived —
      `scripts/verify-cef-framework-darwin.sh` exit 0 on the fresh build.
- [x] Determine actual `CEF_VERSION` — **148.23.23** (`Info.plist`
      `CFBundleShortVersionString` = `148.23.23.0`; `cef_version.h`'s
      `CEF_VERSION` = `148.23.23-rebuild-7778-codecs.3533+g6c570e2+chromium-148.0.7778.180`,
      `CEF_COMMIT_HASH` = `6c570e2490c9cfdf5cd89dee7be4e475404d10b7`). No
      existing `cef-macos-arm64-148.23.23` tag on `agentmuxai/cef` (latest
      before this was `148.23.21`), so no numeric collision — used the
      `-codecs` suffix anyway for clarity, matching the fact that the
      internal `CEF_VERSION` string itself already embeds "codecs" from the
      `rebuild-7778-codecs` branch name.
- [x] Stage to `~/cef-build/darwin/aarch64` — done. The pre-codec framework
      that was there (148.23.23.0, no codec flags, unrelated to today's build
      despite the coincidentally-identical version number) was renamed to
      `Chromium Embedded Framework.framework.pre-codec-bak` rather than
      deleted, in case a fast revert is ever needed.
- [x] Codec playback sanity check — **passed**, see full write-up below.
- [x] Cut `agentmuxai/cef` release tag — **`cef-macos-arm64-148.23.23-codecs`**,
      https://github.com/agentmuxai/cef/releases/tag/cef-macos-arm64-148.23.23-codecs
      (asset `cef-macos-arm64-148.23.23-codecs.tar.gz`, 180,952,312 bytes,
      unstripped, symlink chain + patch verified intact post-tar-round-trip).
      Cut under the `AgentO-asaf` GitHub identity via plain `gh` (not
      `scripts/gh-agent.sh` — its `secrets` CLI dependency isn't installed in
      this session's environment; confirmed via `gh api user` that the
      already-active shared-keyring session was correctly `AgentO-asaf` before
      using it directly).
- [ ] Comment on issue #2311 — next.

## Codec playback sanity check — write-up

No existing tooling covers this (the patch-verify script only checks
`BeginWindowDrag`), so this was built fresh for this run.

**Test file:** `/Users/asafebgi/S-NDNfB8mwY24lBo.mp4` — confirmed via `mdls
-name kMDItemCodecs` to be H.264 video + MPEG-4 AAC audio, the exact codec
combo from the original bug report
(`docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`).

**Method:** Built an isolated local package
(`AGENTMUX_CEF_RUNTIME_DIR_DARWIN=~/cef-build/chromium/chromium/src/out/Release_GN_arm64
NOTARIZE=0 task package:macos -- /tmp/codec-test-package`) rather than `task
dev`, since a `task dev` instance from a different checkout was already
running on `main` and dev data dirs are keyed by branch name only (would have
collided on the single-instance pipe). Launched the built `.app` with `open
-n`, found its CEF `--remote-debugging-port` from `ps aux`, and drove it via
raw Chrome DevTools Protocol (`node` + the repo's own `ws` package — no
Playwright needed for a non-Electron CEF app) rather than clicking through the
UI.

**Gotcha hit:** a first attempt setting `<video src>` directly to the
`stream-local-file` URL failed with `MEDIA_ERR_SRC_NOT_SUPPORTED` — looked
like a codec failure but wasn't. `stream-local-file` lives in `authed_routes`
and requires an `X-AuthKey` header that `<video src>` can't set (see
`frontend/app/view/media/media.tsx`'s `fetchMediaBlob` comment). Fixed by
matching the real Media pane's actual approach: `fetch()` the bytes with
`window.api.getAuthKey()` in the header, then hand the video element a
`URL.createObjectURL()` blob URL.

**Result (real run):** fetch returned a real 26,992,509-byte `video/mp4`
blob. Video element reached `readyState=4` (`HAVE_ENOUGH_DATA`), `error:
null`, correctly parsed `videoWidth=1920 videoHeight=1080` (only possible
after actually decoding a frame) and `duration=258.709333` (matches the
source file). Confirmed genuine real-time decode, not just a metadata parse:
`currentTime` advanced `3.68s → 6.12s` over 2.5 real wall-clock seconds after
explicitly bringing the window to front and re-calling `.play()` (a
background-tab pause transiently made one intermediate reading look stalled —
resolved once focused). **No `DEMUXER_ERROR_NO_SUPPORTED_STREAMS`, no
`MEDIA_ERR_DECODE` — the original bug is fixed in this build.**

## Post-merge incident: CI's "latest tag" resolution doesn't reliably pick up this release

Found 2026-07-28 while answering "will future builds automatically pull in
this codec work?" — **the answer is currently no**, not without further
action. Reproduced the exact query `build-macos.yml` and
`ci-nightly-artifacts.yml` both use:

```bash
gh release list --repo agentmuxai/cef --limit 30 --json tagName \
  --jq '[.[].tagName | select(startswith("cef-macos-arm64-"))][0]'
# → cef-macos-arm64-148.23.21   (NOT cef-macos-arm64-148.23.23-codecs)
```

Root cause: `gh release list`'s default order is not reliably
publish-time-descending on this fork. Checked the raw API
(`created_at` vs `published_at`) for every `agentmuxai/cef` release: every
release cut without an explicit `--target` — including this one, and the
Linux/Windows codec releases cut around the same time by AgentU/Agent2 — got
`created_at` frozen at `2026-04-23T15:56:22Z` (the fork's default-branch HEAD
commit date, not the actual `gh release create` call time). Only
`cef-macos-arm64-148.23.21` (cut 2026-07-02) has a "real" `created_at`, and
`gh release list` appears to sort by `created_at`, so it sorts *ahead* of
every more-recently-published release including this one. This is a
pre-existing fragility in the shared CI resolution pattern (identical logic
across all three platforms' workflows), not something introduced by this
specific release — but it means the codec work is a no-op for CI until it's
addressed.

**Not yet remediated as of this write-up** — the reliable fix is deleting/
retiring the stale `cef-macos-arm64-148.23.21` tag so `[0]` has nothing wrong
to pick (a version bump or tag suffix alone doesn't fix a `created_at`-based
sort). That's a destructive action on a shared distribution repo other
workflows depend on, so it's being confirmed with the user rather than done
unilaterally. Whoever picks this up: check `gh release list --repo
agentmuxai/cef --limit 30 --json tagName --jq '[.[].tagName |
select(startswith("cef-macos-arm64-"))][0]'` again first — if it already
prints `cef-macos-arm64-148.23.23-codecs`, this has been handled.

## Notes / findings as they come up

(appended above as the run progressed; kept here as the running log)
