# Spec: CEF milestone 156

**Status:** active — approved 2026-11-10 (operator, relayed by AgentY: "yes proceed with best recommendations"); the three decisions in §6 are the author's recommendations, taken as decided. The three runtimes (agentmuxai/cef `8078` @ `20ec5c6ec`), the `agentmux/156` binding and the pins ship in the CEF 156 pin PR; what remains is the look checks on macOS and Linux and a functional check of each packaged app. Verified 2026-11-11. Owner: Korp. Task #46843 (this spec), #46844 (the build).
**Order (operator):** 154.0.8037.98 on all three platforms first (task #31598, in flight), then this. Not instead.
**Checked against:** `agentmuxai/cef` `8037` @ `e6be25fea`, upstream CEF `8078` @ `a10dae340` (Chromium `refs/tags/156.0.8078.88`), `tauri-apps/cef-rs` tag `cef-dll-sys-v156.4.0+156.0.26`, this repo's `origin/main` `b4fa1c400`.

## 0. Summary

The move from 154 to 156 is smaller than a milestone jump usually is, and slower than it looks:

- **Source work is small.** Our CEF fork is 12 commits (33 files, about 900 lines) plus 3 Chromium-side patches. A dry-run port of all 12 commits onto upstream `8078` hit two textual conflicts, both trivial (§1). The three Chromium patches apply to the 156.0.8078.88 sources with line offsets only (§1.2). The public CEF C API changed by 79 added lines in four headers, and nothing we use was removed (§2).
- **Build time is the real cost.** The patch-level move to .98 already invalidated nearly the whole tree on every platform. Measured on that move: Windows 5 h 54 min (`-j 8`, shared machine), Linux about 11 h, macOS estimated 13 h by its builder (§3). A milestone move is at least that.
- **Estimate: 3 to 4 working days elapsed, about 1.5 days of hands-on work** (§5). The serial chain is: fork branch and binding port (1 day) → three builds in parallel (one night) → verification and look checks (1 day) → one pin PR.

## 1. What the fork carries, and what conflicts on 156

### 1.1 Inventory (agentmuxai/cef `8037`, over upstream `8037`)

Layer B, CEF source (12 commits, 33 files, +925/-46):

| Item | Commits | Files | Used by the app (files in `crates/cef/src`) |
|---|---|---|---|
| Drag + right-click + transparency, `BeginWindowDrag` (annotation 15400) | `7c9733d79`, `f45d07456` | `cef_window.h`, `cef_types.h`, `window_impl.*`, `window_view.cc`, `widget_impl.*`, `browser_view_impl.*`, `blink_glue.*`, `render_manager.cc`, `browser_config.h`, `browser_platform_delegate*`, `cef.mojom`, `cefclient` | 2 (`begin_window_drag`) |
| ThemeService observed once per Widget (#11) | `4227cee14` | `widget_impl.*` | none (internal) |
| `CefRequestContext::GetAutocompleteSuggestions` (#12) | `7110fba5d`, `a357fc935`, `f2cc33834` | `cef_request_context.h`, `request_context_impl.*` | 1 |
| `CefBrowserHost::WasHidden` for windowed browsers (#13) | `c8fd42d7b` | `alloy_browser_host_impl.cc`, `browser_contents_delegate.cc`, `browser_host_base.*`, `browser_platform_delegate.*` | 1 |
| `CefBrowserHost::Discard`, `IsDiscarded`, `LoadIfNecessary` (#14) | `138de7a00`, `de9d065ee` | `cef_browser.h`, `browser_host_base.*` | 13 (hibernation) |

Layer A, Chromium-side patches (3, in `patch/patches`): `rwhv_background_opaque_check` (macOS-critical), `views_caption_rightclick_passthrough` (Linux), `agentmux_process_requirement` (macOS 26).

Things the task text names that are **not** fork source patches:

- **Codecs** are GN arguments (`proprietary_codecs`, HEVC, AC3, EAC3, Dolby Vision), not a source change. They carry over by reusing the per-platform `args.gn`.
- **Input-owner hooks and the pane window wrapper** live in this repo (`crates/cef`), against the stock CEF API.
- **Overlay clipping** is partly pending: `agentmux/overlay-shape-8037` (`CefOverlayController::SetShape`, 1 commit) is **not merged** into `8037`. Decided: it is not carried (§6.1). The Views overlay opt-in (#6371) is app code against the stock `CefOverlayController`.

### 1.2 Dry run on `upstream/8078`

Method: a scratch branch on `upstream/8078`, the 12 commits cherry-picked in order (the runbook's "one commit per item" port, `CEF_FORK_MAINTENANCE.md` §6).

| Result | Where | Fix |
|---|---|---|
| Conflict | `patch/patch.cfg` (3 commits append to the end of the list) | Keep both sides. Mechanical. |
| Conflict | `libcef/renderer/blink_glue.{cc,h}` in the drag/transparency commit: upstream `8078` no longer has `SetUseExternalPopupMenus` next to `StartNavigation`, so our two added glue functions land beside a different tail | Keep both functions. Chromium 156 still has `ChromeClient::SetUseExternalPopupMenus` and `WebViewImpl::SetBaseBackgroundColorOverrideTransparent` (checked in the 156.0.8078.88 sources), so they compile. Re-check at build time. |
| Clean | the other 9 commits | none |

Merging `origin/8037` into `8078` is **not** the way (it conflicts in about 30 upstream patch files); port by cherry-pick, as the runbook says.

Layer A against Chromium `156.0.8078.88` (the three target files fetched at the tag, `patch -p0 --dry-run`): all three apply, with offsets (+39 lines, -1 line, none). The patcher run on the real tree can still disagree where an upstream `8078` patch now touches the same file; that shows up at the first sync, not before (§4, risk 2).

## 2. API changes that matter

**CEF C API, 8037 → 8078:** 4 headers change (`cef_preference.h` +23, `cef_types.h` +32/-1, `cef_types_content_settings.h` +8, `cef_types_wrappers.h` +17), new API version `15600`. Nothing we call (`OnFileDialog`, Views `BrowserView`, `Window`, focus and IME, `OverlayController`, `RequestContext`) changed in the C API. Our 4 header additions are annotated `added=15400`; they get `15600`-compatible neighbours and keep their own annotation.

**cef-rs:** upstream publishes `156.4.0+156.0.26`. Our fork `agentmuxai/cef-rs` carries 5 small commits over `cef-dll-sys` 154: the `links` rename, `begin_window_drag` (struct offset 888), `get_autocomplete_suggestions`, and the Discard fields (with the offset pins). They are hand edits to the generated bindings of each platform file; there is no code beyond that. Plan: branch `agentmux/156` from the upstream 156.4.0 tag, re-apply the 5 commits (the struct layouts must match the C headers of the fork commit, which is what the offset assertions check), and re-point `[patch]` in this repo's `Cargo.toml` at it.

Outside the bindings, the upstream `cef` crate changed between 154.2 and 156.4 in `osr_texture_import/*` (rewritten, not used by us), `wrapper/message_router.rs` (10 lines) and the generated resources. Expect a compile pass in `crates/cef` for the few call sites that move; sized at half a day, with the Windows build of `agentmux-cef` as the check.

**Toolchain:** Chromium 156 keeps the same Windows SDK (`10.0.28000.0`) and VS 2022 toolset; the clang revision moves (`llvmorg-24-init-7747-g62397f8b`), which `gclient runhooks` downloads. Upstream `8078` also pins `depot_tools_checkout` (`132599e…`): the sync must use that depot_tools revision (the 154 branch did not pin one).

## 3. Build plan

Same recipe as the 154 and .98 builds (`docs/cef-build/*`, `CEF_FORK_MAINTENANCE.md` §6), per platform, all from one fork commit:

1. **Fork:** `agentmuxai/8078` from `upstream/8078`; port the carry-set by cherry-pick (one commit per item, message `agentmux: port <item> to 8078 (Chromium 156)`); overlay-shape is not carried (§6.1); extend `scripts/cef-verify.sh` to the new branch (expect 33 OK, 0 MISS); muxreview as for #15.
2. **Each platform, in parallel:** `gclient sync` to `refs/tags/156.0.8078.88` (with `-D`, patches reset out of `v8`, `angle`, `dawn`, `depot_tools` first, `third_party/dawn/tools/golang` deleted: the 152 → 154 lessons), patcher (count, no `.rej`), `gn gen` with the existing args, ninja, probes, package, draft release `cef-<platform>-156.0.8078.88-<codecs->r1`.
3. **Detached and resumable:** Start-Process or nohup, a resume-only script that never re-runs the patcher (the patcher is not idempotent). Run builders hands-off overnight.
4. **One pin PR** after the three drafts exist (P1/P2 in `CEF_FORK_MAINTENANCE.md` §8).

| Platform | Builder | Measured on the .98 move | Expect for 156 | Disk to have free |
|---|---|---|---|---|
| Windows x64 | Korp | 35.7k steps, 5 h 54 min at `-j 8` with another agent's Rust build holding 16 GB of commit | 8 to 10 h at `-j 8`; 5 to 6 h at `-j 14` on a quiet machine | 250 GB |
| Linux x64 | Opaz | 52.8k steps, about 11 h (started the evening before, done by 06:50) | 11 to 13 h | 250 GB |
| macOS arm64 | Maop | 29.8k steps, 13 h by the builder's own estimate on a shared Mac | 13 to 15 h | 250 GB |

The task text assumed about 4 h each. The .98 move shows that is not realistic: even a patch-level `gclient sync` invalidates most objects, so budget a night and treat the macOS build as the long pole.

## 4. Risks and how each is checked before the pin lands

1. **Carry-set incomplete on the new branch** (the 2026-07 transparency gap). `scripts/cef-verify.sh --ref <sha>` must print 33+ OK and 0 MISS, against the exact commit built (P3).
2. **An upstream `8078` patch now overlaps one of our three Chromium patches.** Caught at the first patcher run; a `.rej` fails the build script. The Linux and macOS builders report the patcher count (119 patches at .98) and no `.rej`.
3. **Dropped or moved Blink internals** behind our glue (`blink_glue`, `SetBaseBackgroundColorOverrideTransparent`). Checked: both exist in 156.0.8078.88. A compile is the final check.
4. **Binding/struct drift between the fork headers and `agentmuxai/cef-rs`:** the `offset_of!` assertions in the bindings fail the Rust build, not at runtime. Run `cargo check -p agentmux-cef` on all three platforms (CI does).
5. **Hibernation** (`Discard`, `IsDiscarded`, `LoadIfNecessary`, `WasHidden`): the PDB/symbol probes from `CEF_FORK_MAINTENANCE.md` §7.2, then the end-to-end check used for r4 (hide a tab, the renderers exit, show it, URL/history/scroll/sessionStorage unchanged).
6. **Look and input checks on all three platforms**, per the visual-change rule: the browser pane (render, zoom, scrollbars, select popups), the Views overlay opt-in (#6371), the window drag/right-click behaviour, transparency on macOS, the address bar autocomplete, downloads (the strip and the agent-pane downloads), tear-off windows, IME and the pane-focus owner. Windows by the builder; macOS by the platform tester (Clare, or Masty/Maop); Linux by Maricon. A message per host with the branch/commit and the list, answered with pass/fail per item.
7. **Security or behaviour changes in Chromium 156 itself** (permissions, downloads, the file chooser path, third-party cookie defaults). Read the 155 and 156 release notes for the pane-relevant ones before the look checks; the browser seat's own tests (`crates/cef`, `crates/mcp` browser tools) run on the new runtime.
8. **The macOS build is slow and the Mac is shared.** Start it first; do not wait for the Windows build to finish before asking for it.
9. **Crash-reporter and sandbox flags** (Linux AppArmor profile, macOS 26 process requirement): the Linux sandbox tests and the `agentmux_process_requirement` probe run on the new runtime.

## 5. Estimate

| Step | Hands-on | Elapsed | Serial? |
|---|---|---|---|
| Fork branch, port, verify, review (§3.1) | 0.5 d | 0.5 d | **Yes**: every build waits for the commit |
| cef-rs `agentmux/156` and the `crates/cef` compile pass (§2) | 0.5 d | 0.5 d | Parallel with the fork work; needed before the app is tested |
| Three runtimes (§3.2) | 1 h each to start, 1 h each to probe and package | one night, set by macOS at 13 to 15 h | Parallel across platforms, serial inside each |
| App checks, look checks on three platforms (§4) | 0.5 d | 1 d with waiting on the other hosts | Needs all three runtimes for the cross-platform part |
| Pin PR, review, merge (§3.4) | 1 h | 0.5 d | **Yes** |

**Total: 3 to 4 working days elapsed, about 1.5 days of hands-on work.** The serial parts are the fork commit before any build, the longest platform build, and the pin PR after the three checks. A Windows-only early pin is not proposed: the three tags move together (P2).

## 6. Decisions (approved 2026-11-10)

1. **Overlay shape: not carried.** `agentmux/overlay-shape-8037` (`CefOverlayController::SetShape`, one commit from 2026-10-07) has been unmerged for five weeks, nothing in `crates/` calls it, and `agentmuxai/cef-rs` has no binding for it. Carrying it would add an untested patch to the milestone move. The branch stays in the fork; if the Views overlay work (#6371) needs it later, it is a normal carry-set addition on `8078` (one cherry-pick, its own binding commit, its own build).
2. **The macOS build starts from the fork commit that passes `scripts/cef-verify.sh`, before muxreview has approved the fork PR.** It saves up to half a day on the long pole. If review changes anything beyond comments or tests, the commit is re-verified and macOS is told at once; a source change means a rebuild.
3. **Tag names:** `cef-windows-x86_64-156.0.8078.88-r1`, `cef-linux-x86_64-156.0.8078.88-codecs-r1`, `cef-macos-arm64-156.0.8078.88-codecs-r1`, matching the .98 scheme. A rebuild from a changed fork commit is `r2`.

## 7. After approval

Task #46844 (build) starts: the fork branch and PR, `agentmux/156` in cef-rs, the three builds by message to the Linux and macOS builders (their time is a loan outside their seat), the pin PR with the look checks recorded in the body, and the record updated in this repo (`docs/cef-build/*` status lines).

## 8. r2 (2026-11-12)

The same Chromium 156.0.8078.88, rebuilt from `agentmuxai/cef` 8078 at `82cdb1cee` with two fork changes, tags `...-r2` / `...-codecs-r2`:

- **`CefOverlayController::SetClipRects`** (cef#19, `added=15600`, cef-rs#4): limits an overlay to the union of rectangles (overlay-local DIP), neither drawn nor hit-tested outside them. Aura only (Windows, Linux); ignored on macOS. This replaces decision 1: the old `SetShape` is still not carried, but Views overlay panes on Windows need a clip where other UI covers a pane. Checked on a Windows build: the page behind shows through and clicks in the cut-out reach it. It is a `sys` field only: callers check `base.size` (a stock or r1 libcef allocates the 192-byte struct, r2 200).
- **Chrome's browser autofill is not attached** (cef#20): `AttachTabHelpers(web_contents, /*enable_browser_autofill=*/false)` in the Alloy delegate. A host on CEF 154 died with a null read in libcef after a payment card form was saved; the 156 host did not. The attached autofill client has no Chrome window or prompts behind it in an embedder.

Pins: Windows libcef SHA-256 `8277020ca6cad1fdf4c07686600d1acc365eff66cece9c6e3e66d7e5c11c3649`, Linux archive `c1f2b78b87341e8ba34a422e4c53f204aecae7f5ebe3944c8181dc98e846d877`, macOS archive `e8a04d208b8042fdf8ecfd90b300e316a56240ace8ba52102f1e7fa7648285c8`.
