# CEF 148 — Linux Drag/Right-Click/Transparency Forward-Port

**Status:** Spec only — no PRs yet
**Date:** 2026-06-04
**Author:** AgentU
**Tracking:** Will land as a sequence of PRs against `agentmuxai/cef`, `a5af/cef-dll-sys`, and `agentmuxai/agentmux`. See §8.

---

## 1. The problem

PR #1221 (`feat(macos): CEF 148 notarized DMG`) bumped the workspace from CEF 146 → CEF 148 in `Cargo.lock` + `agentmux-cef/Cargo.toml`. The bump was driven by a macOS-26 renderer crash (`process_requirement.cc … -67030`) that is fixed by a single-file patch on `agentmuxai/cef@agentmux/7778-process-requirement`.

That CEF 148 branch contains **only the macOS-26 patch**. The other agentmux patches that are load-bearing on Linux still live on `agentmuxai/cef@agentmux/7680-drag-rightclick-and-transparency` (CEF 146) and have **not** been forward-ported.

### What broke in the bump (Linux only — macOS + Windows are fine on CEF 148)

The agentmux patches add `_cef_window_t::begin_window_drag` to the CEF Window vtable. AgentMux's renderer calls it for native title-bar drag on Linux via `start_window_drag` IPC → `StartWindowDragTask::execute` (`agentmux-cef/src/ui_tasks.rs:215`). The cef-dll-sys 146 binding exposes the field; cef-dll-sys 148 does not. After the bump:

- `cargo build --release -p agentmux-cef --features patched-libcef` fails with `error[E0609]: no field begin_window_drag on type _cef_window_t`.
- `task build:host:linux` (which passes `--features patched-libcef`) failed silently in CI / on dev machines; the workaround used by official AppImage builds was to **drop the feature flag**, which compiles the `#[cfg(not(feature = "patched-libcef"))]` no-op branch.
- Every Linux AppImage built since #1221 logs `[start_window_drag] patched-libcef feature disabled — native drag is a no-op` on every drag attempt. Title-bar drag silently does nothing regardless of ozone platform (Wayland or X11).

Discovered via the user's bug report 2026-06-04 ("top drag not working") plus log inspection (`muxlog host begin_window_drag`).

### What we tried first that did not work

| Attempt | Outcome |
|---|---|
| PR #1241 (forced `--ozone-platform=x11` default) | Mitigated typing perf (rAF stalls), but title-bar drag still broken because the host's no-op branch fires regardless of platform. Closed. |
| PR #1260 (workspace pin `cef = "146"`) | Compiled + worked on Linux, but **broke macOS + Windows** — both ship CEF 148 libcef binaries (`agentmux/7778-process-requirement` for the macOS DMG and the Windows `bundle:windows` task), and 146 source against 148 runtime crashes at startup with `unsupported CEF API version 14800`. Reverted in PR #1265. |
| Per-target Cargo deps (`[target.cfg(target_os="linux").dependencies] cef = "146"`) | Rejected by Cargo's resolver: `cef-dll-sys` is `links`-tagged, only one version can appear in the dependency graph globally. |

The correct fix is the forward-port — bring the patches onto CEF 148 so the workspace can stay on `cef = "148"` everywhere.

---

## 2. Out of scope

- Frontend changes. `useWindowDrag.linux.ts` and `frontend/app/workspace/floating-pane-workspace.tsx` already call the IPC correctly; no FE changes needed.
- The X11 ozone default. That is tracked separately (PR #1241 / superseded #1261). Once this spec is implemented, X11 default is straightforward and can be re-shipped under its own PR.
- macOS-26 process_requirement patch (`agentmuxai/cef@agentmux/7778-process-requirement`). That's already on CEF 148 and unchanged here.
- CEF 148 → 150 / 152 future bumps. The pattern this spec establishes should apply, but each bump may need re-port of conflicting hunks.

---

## 3. Inventory of patches to forward-port

Commits on `agentmuxai/cef@agentmux/7680-drag-rightclick-and-transparency` that are not on upstream `7778`:

| SHA | Title | Why we need it on Linux |
|---|---|---|
| `b921ffe1` | `Support transparent window in Views framework (follow up issue #2315)` | Frameless window background respects CSS `rgba()` alpha. Required for `window:transparent` setting + magnify backdrop. |
| `41802fe6` | `Patch A: Let right-clicks on HTCAPTION fall through to renderer` | The pane-header right-click context menu only fires if right-clicks on the HTCAPTION region (drag handle) reach the renderer instead of being eaten by the WM. |
| `af485ed2` | `views: Add CefWindow::BeginWindowDrag()` | The function we call from `StartWindowDragTask`. Dispatches through `WmMoveResizeHandler::DispatchHostWindowDragMovement` (Ozone: Wayland `xdg_toplevel.move` / X11 `_NET_WM_MOVERESIZE`). |
| `010f616f` | `views: Address PR review on BeginWindowDrag` | Critical X11 fix: use `display::Screen::Get()->GetCursorScreenPoint()` instead of `gfx::Point()` so `_NET_WM_MOVERESIZE` has a valid anchor (Mutter ignores the request otherwise). Also clarifies the `static_cast<aura::WindowTreeHostPlatform*>` invariant. |
| `130af663` | `views: Annotate CefWindow::BeginWindowDrag with added=14600` | Marks the API as added in CEF 14600 for the version-annotated binding generator. **The version number must be updated** when porting to 148. |
| `68e0dc66` | `views: complete the transparency cascade — propagate to WebContents` | `SetBackgroundColor(SK_ColorTRANSPARENT)` on the BrowserView also runs `SetPageBaseBackgroundColor` on the underlying WebContents so the renderer's first paint is alpha-aware (closes the white-flash hole). |
| `2f69aabc` | `views: also call RWHView::SetBackgroundColor for renderer alpha cascade` | Belt-and-suspenders: cover the case where WebContents already has a RenderWidgetHostView. |
| `6e0e93ed` | `views: install WebContentsObserver for late-binding transparency cascade` | When the renderer process is swapped (cross-origin nav, crash recovery), the new RWHV needs its background re-set. Observer hook does it on `RenderViewReady`. |
| `95ade0ba` | `views: keep WebContentsObserver alive across renderer process swaps` | The observer was getting torn down with the original renderer. Now lives on the BrowserViewImpl for its lifetime. |
| `3e041ad2` | `views: deferred top-level transparent bg + observer cleanup` | Top-level (parent-less) windows need the cascade ON CREATION before the first browser is attached — deferred via initial pending flag. Plus cleanup of the observer in dtor. |

**11 commits total.** All are small (< 30 lines of C++ each except `b921ffe1` and the BeginWindowDrag pair). The 5 transparency cascade commits are tightly coupled and should be ported as a unit.

Plus one supporting change on `agentmuxai/cef@agentmux/api-version-annotation` (if applicable to 148) — annotation infrastructure for the version-tagged binding generator. Confirm whether this is needed before porting.

---

## 4. Patch survey: expected merge conflicts vs upstream 7778

This needs to be done *empirically* by attempting cherry-pick onto upstream `7778`, but a-priori the high-risk areas are:

| File | Risk | Reason |
|---|---|---|
| `libcef/browser/views/window_impl.cc` | **High** — Chromium 146 → 148 includes Aura/Views changes that touch this file. The `BUILDFLAG(IS_OZONE)` include block at the top and the `BeginWindowDrag()` implementation must apply cleanly. | Cherry-pick will likely conflict; manual fix-up needed. |
| `libcef/browser/views/window_view.cc` | Low-medium | Smaller transparency hook surface. |
| `libcef/browser/views/browser_view_impl.cc` | **High** | The transparency cascade observer + cleanup hooks are added in multiple commits across this file. |
| `include/views/cef_window.h` | Low | API declaration is stable. |
| `libcef_dll/template_util.h` | Low | Helper for binding generation. |
| `patch/patches/views_caption_rightclick_passthrough.patch` | Low | Standalone CEF source patch file; should apply if Chromium's `Widget`/`WindowEventDispatcher` code didn't move. |

Mitigation: port `af485ed2` (BeginWindowDrag) **first**, prove the build, then layer the transparency cascade on top. Don't squash — each commit's intent is needed for review.

---

## 5. cef-dll-sys binding update

The cef-dll-sys crate auto-generates the FFI binding from CEF's C headers. After the cef source-side patches add `begin_window_drag` to `_cef_window_t`, the binding regeneration must include the new field at the *end* of the struct (so it's appended, not interleaved — important for ABI compatibility with unpatched libcef.so builds that lack it).

Two paths:

**A. Fork cef-dll-sys with the patch (preferred — matches current 146 approach).**
The 146 line of work uses a fork at `a5af/cef-dll-sys` with the field appended. Repeat for 148:

1. Fork `cef-dll-sys` 148.3.0+148.0.9 (the current public version).
2. Edit `cef_dll_sys/build.rs` or the bindgen invocation to ensure the patched header (with `begin_window_drag`) is the input.
3. Verify the generated `_cef_window_t` struct has `begin_window_drag` appended at the end.
4. Publish the fork as `a5af/cef-dll-sys` branch (e.g. `agentmux/148`) and tag a version like `148.3.0+148.0.9-agentmux`.
5. Add a `[patch.crates-io]` block to workspace `Cargo.toml`:
   ```toml
   [patch.crates-io]
   cef-dll-sys = { git = "https://github.com/a5af/cef-dll-sys", branch = "agentmux/148" }
   ```

**B. Use cef-dll-sys's `links` override + a workspace shim crate.**
More invasive; rejected unless A turns out to be infeasible.

The **runtime ABI guard** in `agentmux-cef/src/ui_tasks.rs:201` already catches binding/runtime mismatch by comparing `_cef_window_t.size`. So a stale binding against a fresh libcef.so (or vice-versa) degrades gracefully to a logged warning rather than UB. Keep that guard intact through this port — it's the safety net.

---

## 6. CEF 148 libcef binary build

Once §3 + §5 are done, build a CEF 148 libcef.so + libcef.dll from the new `agentmuxai/cef@agentmux/7778-drag-rightclick-and-transparency` branch.

### Linux

The local cef-build setup at `~/cef-build/chromium_git/chromium/src/cef` already targets a4 CEF 146 source tree. Switching it to 148 requires:

1. Update the cef source tree symlinks/checkouts to the new branch HEAD.
2. Run `automate-git.py` with the appropriate Chromium 148 ref.
3. `cd ~/cef-build/chromium_git/chromium/src` then `autoninja -C out/Release_GN_x64 cef`.
4. Sanity-check the output is ~600 MB stripped (matches CEF 146 size profile).
5. Update `scripts/resolve-cef-runtime.sh` to also accept 148 builds.

**Time:** First build from scratch is 6–12 h on the local machine. Incremental rebuild from current 146 cache: estimate 1–3 h (lots of Chromium-side files change between 146 → 148).

### macOS

Replace `agentmuxai/cef@agentmux/7778-process-requirement` (current macOS DMG source) with `agentmuxai/cef@agentmux/7778-drag-rightclick-and-transparency` once it's ready. The process_requirement patch must be on that branch too — cherry-pick `5c9a1b08` onto it after the drag/transparency patches. macOS build pipeline (PR #1243) consumes from the new branch unchanged otherwise.

### Windows

Currently consumes cef-dll-sys's bundled libcef.dll via the `bundle:windows` task (no separate libcef build). When we publish the fork of cef-dll-sys (§5), the bundled libcef.dll will come from that fork's prebuilts — which means the fork must produce a Windows libcef.dll too, or alternatively the `bundle:windows` task can pull from a separate Windows artifact.

This part is the most awkward and may require a separate CEF Windows-build follow-up. Confirm before committing to the full forward-port.

---

## 7. Validation criteria

The forward-port is "done" when **all** of the following hold:

1. `cargo build --release -p agentmux-cef --features patched-libcef` succeeds on `main` (no Cargo pin, no per-target hack) on Linux, macOS, Windows.
2. `task build:host` on Linux produces a host binary whose `strings` output contains the `BeginWindowDrag returned` info string (not the disabled-feature warn string).
3. `scripts/verify-cef-version.sh` passes on all three platforms (asserts workspace Cargo `cef = "148"` matches bundled libcef major version 148).
4. Manual: title-bar drag works on Linux under both `--ozone-platform=wayland` and `--ozone-platform=x11`.
5. Manual: pane-header right-click context menu fires on Linux (proves `41802fe6` HTCAPTION pass-through).
6. Manual: `window:transparent` setting + magnify backdrop render correctly on Linux (proves transparency cascade).
7. Manual: macOS DMG build still succeeds + macOS-26 renderer crash stays fixed (no regression of `agentmuxai/cef@agentmux/7778-process-requirement`).
8. Manual: Windows portable build still launches + no `unsupported CEF API version` crash.
9. `muxlog host` shows no `libcef.so ABI mismatch` warnings on first window create.

---

## 8. Sequencing — concrete PR plan

| Order | Repo | PR | Blocks |
|---|---|---|---|
| 1 | `agentmuxai/cef` | New branch `agentmux/7778-drag-rightclick-and-transparency`. Cherry-pick the 11 commits onto upstream `7778`, fix up conflicts file-by-file (§4). | §1 of §5 (cef-dll-sys needs the headers). |
| 2 | `a5af/cef-dll-sys` | New branch `agentmux/148`. Update bindgen against patched headers from PR #1. Tag `148.3.0+148.0.9-agentmux`. | §1 of §6 (libcef build needs the binding fork pinned). |
| 3 | local `~/cef-build` | Rebuild CEF 148 libcef.so for Linux from PR #1's branch. Stage at `~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/libcef.so`. Multi-hour. | None — independent timeline. |
| 4 | macOS CEF build host | Rebuild CEF 148 libcef framework for macOS from PR #1's branch + cherry-pick `5c9a1b08` (process_requirement). | None — independent. |
| 5 | `agentmuxai/agentmux` | Workspace `Cargo.toml` adds `[patch.crates-io]` pointing at PR #2's cef-dll-sys fork. `task build:host:linux` succeeds with `--features patched-libcef`. | PRs #1 + #2. |
| 6 | `agentmuxai/agentmux` | Bundle script updates (`scripts/resolve-cef-runtime.sh`) point at the new Linux libcef.so. macOS DMG pipeline points at the new branch. | PRs #3 + #4. |
| 7 | `agentmuxai/agentmux` | Re-open the X11 ozone default (was #1261, currently reverted via #1265). Now safe because drag works on X11 too. | PRs #5 + #6. |

PR #1 (the cef source forward-port) is the most labor — the rest is downstream of it. Start there.

---

## 9. Estimated effort

| Phase | Hours (wall clock) | Notes |
|---|---|---|
| §3 + §4: source forward-port + conflict resolution | 3–6 | Cherry-pick + manual fix-up of `window_impl.cc` + the transparency cascade commits. Most uncertainty here. |
| §5: cef-dll-sys 148 binding fork | 1–3 | Mechanical — copy the 146 fork's pattern. |
| §6: Linux libcef.so build (incremental) | 1–3 | Best case; could be 6–12 h if 148 invalidates more of the build cache than expected. |
| §6: macOS libcef framework build | 4–8 | Requires macOS hardware (not this machine). Hand off. |
| §7 + §8: bundle script updates, workspace bump, X11 default re-ship | 1–2 | Quick after the binaries land. |
| **Total** | **10–24** | Plus macOS build out-of-band. |

Spread across 1–2 weeks of calendar time is realistic, contingent on macOS build availability.

---

## 10. Open questions

- Should we publish prebuilt `libcef.so` for Linux as a release artifact in `agentmuxai/cef` so devs don't all need to rebuild locally? Today the resolve script picks up an arbitrary local build, which is fragile.
- Does the macOS-26 process_requirement patch (`5c9a1b08`) merge cleanly on top of the drag/transparency stack, or vice-versa? Order may matter; pick one and document.
- Is there a Chromium 148-side ABI break in `WmMoveResizeHandler::DispatchHostWindowDragMovement`'s signature? If yes, `af485ed2`'s call site needs updating. Suspect not — that handler is stable across 146–148 — but verify in §4.
- Long-term: should AgentMux upstream `CefWindow::BeginWindowDrag()` to chromiumembedded/cef? It is a generally useful API for any CEF embedder doing custom title-bar drag without `-webkit-app-region: drag`. Reduces our patch surface to just the right-click and transparency pieces.

---

## 11. References

- PR #1188 (linux floating-pane tear-off Phase A) — depends on native drag working.
- PR #1221 (macOS notarized DMG, CEF 148 source bump) — introduced the version mismatch.
- PR #1241 (linux X11 ozone default — closed) — initial attempt, blocked on this work.
- PR #1242 (predict-echo stall cooldown) — orthogonal but in the same typing-perf story.
- PR #1260 (Cargo pin to 146 — merged then reverted in #1265) — what got us here.
- PR #1265 (revert of #1260) — current state; ready to merge to restore main.
- `docs/cef-patches/` — patch files in the AgentMux CEF fork.
- `~/cef-build/` — local CEF source/build tree.
- Memory: `cef_build_in_progress.md` — pre-existing CEF 146 build notes.

---

## 12. Decision point

Two ways to proceed:

**Path A — bias to action.** Start PR #1 (source forward-port) immediately. Accept that drag stays broken on Linux for the 1–2 week window. Recover by shipping the spec's full sequence.

**Path B — bias to stability.** Keep Linux drag broken in the official AppImage, but document loudly in CLAUDE.md + Taskfile.yml that `--features patched-libcef` is intentionally dropped on Linux until this spec lands. No work starts until macOS build capacity is confirmed (§6).

Path A is what I'd recommend — the spec breaks naturally into independent PRs and Linux drag has been broken since #1221 anyway. Path B's only benefit is freezing scope, and we don't need to freeze.
