# SPEC: Window Transparency on macOS and Linux

**Date:** 2026-07-01
**Author:** analysis session (iTerm, macOS, outside AgentMux)
**Status:** Proposed
**Supersedes / builds on:** `docs/retro/cef-linux-transparency-consolidated.md` (2026-06-25), issue #1335 (macOS, shelved 2026-06-10), `docs/specs/SPEC_WINDOW_OPACITY_GPU_2026_05_21.md`, agentmuxai/cef#3
**Baseline:** agentmux main @ `a0040b9e`, fork `agentmux/7778-drag-rightclick-and-transparency` @ `2720ba103`

---

## 0. Executive summary

> **User-visible state:** transparency works on Windows, not on macOS or Linux.

The single most important fact this spec is built on:

> **Windows does not do per-pixel transparency.** It does **whole-window uniform alpha** —
> `WS_EX_LAYERED` + `SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA)`
> (`agentmux-cef/src/commands/window/transparency.rs:70-86`). The OS compositor fades the
> *finished, opaque* Chromium frame over the desktop. It needs **zero** cooperation from
> Chromium, works on **stock upstream CEF**, and is applied post-render.

macOS and Linux never got the analogous OS-level mechanism. The non-Windows branch of
`set_window_transparency` is a literal no-op (`transparency.rs:60-61`:
`let _ = (transparent, opacity);`). Instead, all macOS/Linux effort went into the
categorically harder **per-pixel renderer-alpha** route (the CEF fork's "transparency
cascade"), which is ~90% built but blocked on two known Chromium-level gaps plus one
genuinely hard one (promoted-layer opacity).

**Both routes are worth finishing — but they are different features:**

| | Track 1 — uniform window alpha | Track 2 — per-pixel "glass" |
|---|---|---|
| What it looks like | Whole window (content included) fades over the desktop — **exactly what Windows ships today** | Window chrome/background translucent per-pixel, panes keep their own alpha — the glassmorphic end-goal |
| Needs CEF patches? | **No** — works on stock CEF | Yes — fork cascade + 2 known missing patches + promoted-layer fix |
| Effort | ~1–2 days (host Rust only) | days–weeks (CEF/Chromium builds + verification) |
| Risk | Very low | Medium-high (one open research question) |

**Recommendation: ship Track 1 immediately for user-visible parity, then land Track 2
incrementally behind the same settings.** Track 1 also de-risks Track 2: once the OS
window itself can fade, every Track 2 improvement is purely additive.

---

## 1. Verified current state (2026-07-01, this machine)

### 1.1 Host code (agentmux main @ `a0040b9e`)

| Fact | Evidence |
|---|---|
| `set_window_transparency` effect body is `#[cfg(target_os = "windows")]`; macOS/Linux discard args | `agentmux-cef/src/commands/window/transparency.rs:35-61` |
| `set_window_opacity` (per-window slider) side-effect loop likewise Windows-only | `transparency.rs:132` |
| `window:blur` argument is sent by the frontend but read by **no** platform (April-era DWM Mica/Acrylic code no longer exists on main) | `frontend/util/cef-api.ts:430-433`; grep of agentmux-cef/src |
| Global `CefSettings.background_color = 0x00000000` unconditionally (arms the fork cascade) | `agentmux-cef/src/lib.rs:810` |
| Main window `BrowserSettings.background_color` = `0x00000000` iff `window:transparent` | `app.rs:1036` |
| Secondary windows/floaters (mac/linux path) = `0x00000000` unconditionally since #1313 | `ui_tasks.rs:1870` |
| `--background-color=00000000` + `--disable-lcd-text` switches gated on `window:transparent`, read pre-CefInitialize | `app.rs:856-877`, `app.rs:579-589` |
| Frontend CSS layer correct: `--window-opacity` on `:root`, pre-paint hint via `window_transparent` URL param, scss default 0.45 | `app.tsx:125-200`, `index.html:68-78`, `theme.scss:20,26` |
| All windows frameless on all platforms (except DevTools popups) | `app.rs:1101-1123`, `creation.rs:407-410` |
| Host already has a raw-libobjc FFI idiom on macOS (no new deps needed for NSWindow work) | `ui_tasks.rs:814+` (`run_macos_native_drag_loop`), `app.rs` (`ensure_macos_native_window_buttons`) |

### 1.2 CEF binaries (the provenance trap)

| Platform | Release/CI | Dev on this machine |
|---|---|---|
| Windows | **Stock upstream CEF** — sufficient because LWA_ALPHA is post-render | stock |
| Linux | Patched `libcef.so` from fork release `cef-linux-x86_64-148.0.20-2` (hard gate in AppImage packaging) | patched only if `~/cef-build` staging resolves; cargo-cache **stock** fallback warns only |
| macOS | Patched-framework wiring exists in CI (#1849/#1859) but release job gated on `APPLE_CEF_AVAILABLE` secret | **VERIFIED UNPATCHED** — `verify-cef-framework-darwin.sh` on `dist/Frameworks` → "NO BeginWindowDrag slot", exit 1 |

**Why macOS dev is unpatched — concrete wiring bug:** the freshly built patched framework
(see 1.3) is staged at `~/cef-build/darwin/arm64` (both `rebuild-mac-cef.sh` and
`rebuild-dcheck-off` stage there, and `BUILD_PLAN.md` says `arm64`), but
`scripts/resolve-cef-runtime-darwin.sh` tier-2 checks `~/cef-build/darwin/aarch64`
(it maps `uname -m` arm64 → Rust's `aarch64`). Tier 2 therefore **always misses** and the
resolver silently falls through to the cargo-cache **stock** framework
(`148.0.9+g0d9d52a`, zero patches — verified by symbol check). `docs/cef-build/`'s
build doc says `aarch64`, the build scripts say `arm64`. One of them must move (§5.1).

### 1.3 What was built on this machine (fresh!)

- `~/cef-build/darwin/arm64/Chromium Embedded Framework.framework` — built **2026-07-01 11:34**,
  version `148.23.20-amux-transp-mac.3530+gc87bca4+chromium-148.0.7778.180`,
  dcheck OFF (safe per `macos_cef_dcheck_build_config` lesson), BeginWindowDrag verify PASSED.
  **Contains the full transparency cascade** (fork commits through `c87bca497`).
  Published today as fork release **`cef-macos-arm64-148.23.20`** (asset tarball, "Latest").
- Whether it contains the frameless-gate fixup `2720ba103` (fork tip; Linux's `-148.0.20-2`
  has it) is **uncertain**: the embedded version string says `+gc87bca4` (11/12 commits),
  while `rebuild-mac-cef.log` shows a `2720ba103` checkout whose ninja step then FAILED —
  the successful build (`build-and-stage.log`, Jun 30) most plausibly used the
  `amux-transp-mac` mirror @ `c87bca497`. Functionally OK for AgentMux either way (the gate
  only *narrows* translucency to frameless windows, and all AgentMux windows are frameless);
  pick up `2720ba103` at the next rebuild for parity.
- **Fork tag hygiene:** all four release tags (`cef-linux-x86_64-148.0.20{,-2}`,
  `cef-macos-arm64-{148.0.9,148.23.20}`) are lightweight tags pointing at the SAME unrelated
  upstream commit (`05d7a2476`, not an ancestor of the fork branch, contains none of the 12
  commits). Provenance must be read from release notes + the binary's embedded version
  string, never the tag.
- It does **NOT** contain the two macOS Chromium patches from #1335 (see 1.4) — those were
  working-tree edits that `rebuild-mac-cef.sh` step 5 (`git reset --hard HEAD`) has since
  **destroyed**. They must be re-authored (they are small and precisely described).
- `rebuild-mac-cef.sh` is currently broken at step 8 (`ninja: unknown target 'cef'` — target
  should be `cef_framework` as the dcheck-off variant uses). Fix when touching (§5.4).

### 1.4 The three known missing pieces for per-pixel (Track 2)

From `docs/retro/cef-linux-transparency-consolidated.md` (Linux, CONFIRMED 2026-06-08) and
issue #1335 (macOS, diagnosed + locally proven, patches lost since):

1. **Renderer-side Blink page-base override** — platform-agnostic, THE confirmed Linux
   root cause. `libcef/renderer/` never calls
   `WebViewImpl::SetBaseBackgroundColorOverrideTransparent(true)`; Blink's
   `page_base_background_color_` defaults `SK_ColorWHITE`, so **every promoted layer that
   paints page background clears to opaque white**. Patch sketch exists in the retro
   (gate on `CefContext::GetBackgroundColor == TRANSPARENT`; must re-apply across renderer
   swaps; either extend public `blink::WebView` or go through `blink_glue`).
2. **`RenderWidgetHostViewMac::GetBackgroundColor()` white substitution** — macOS-only.
   `content/browser/renderer_host/render_widget_host_view_mac.mm:1683`:
   `return (color && *color == SK_ColorTRANSPARENT) ? SK_ColorWHITE : color;`
   (crbug.com/735407). `IsBackgroundColorOpaque()` reads WHITE → re-asserts
   `SetBackgroundOpaque(true)`, clobbering the cascade. Patch: return the real color for
   explicitly-transparent views.

   > ⚠️ **Do not confuse these with the May-era blink patches.** The session-2/3 Linux-VM
   > edits (`web_view_impl.cc UpdateBaseBackgroundColor` re-push,
   > `content_layer_client_impl.cc isOpaque()`) are **documented dead ends** — the
   > consolidated retro records "no visual change at pane interiors" for both. Neither
   > they nor the two patches above were ever in `patch/patches/` (verified:
   > `git diff 0d9d52a65..HEAD -- patch/` shows only the right-click passthrough patch —
   > the ONLY AgentMux patch-system entry in the fork). **No binary from any pipeline has
   > ever contained any blink-side transparency patch.**
3. **Promoted layers rasterize opaque** — the open research question. With #1+#2 applied
   locally on macOS (June 10), the window ROOT composited transparent (hamburger menu
   see-through — first real macOS transparency), but the `transform`-tiled panes stayed
   opaque. Every pane is transform-positioned (`frontend/layout/lib/utils.ts:78`) → every
   pane is its own compositing layer. Candidates ranked in #1335:
   a. `SetContentBackgroundColor(SK_ColorTRANSPARENT)` in the cascade observer (Electron
      parity — Electron sets it alongside `SetBackgroundColor`; RWHV prefers
      `content_background_color_`). **Try first.**
   b. Frontend de-promotion — tile panes with `top/left` instead of `transform` (diagnostic
      first: hot-patch in DevTools; if panes go transparent, promotion is confirmed as the
      trigger and we choose between a frontend layout change vs. a cc-level fix).
   c. cc compositor work (hard route).

### 1.5 History traps this spec encodes (do not re-learn)

- **#947 regression:** an unrelated opacity-slider change killed transparency with green
  bots; only human eyes caught it. There is still no automated transparency smoke test (§5.3).
- **146→148 bump (#1221)** silently dropped the patched-libcef path once already.
- **macOS `screencapture` under this process tree re-composites windows opaque** (proven:
  identical captures at alpha 0.1 vs 1.0) and CEF DevTools `Page.captureScreenshot` returns
  opaque RGB — **all Track 2 macOS verification needs a working capture story or a human** (§5.2).
- Nine documented dead ends live in the consolidated retro — check before re-investigating
  anything Wayland-protocol- or raster-pipeline-shaped.
- `SPEC_WINDOW_OPACITY_GPU_2026_05_21.md` claims LWA_ALPHA is a no-op under healthy
  DirectComposition (Windows worked because GPU was accidentally disabled at the time).
  The user reports Windows works today, GPU state now healthy (#1354) — so either the
  05-21 claim was environment-specific or partially wrong. **Track 1 must therefore be
  pixel-verified on each platform's real compositor path, not assumed** (§3.4).

---

## 2. Root-cause chains (why each platform is where it is)

### 2.0 Why the same code behaves so differently per platform (fork internals)

The fork's transparency commits are all platform-generic (`libcef/browser/views/**`); the
per-platform outcomes differ because of what each OS windowing stack gives for free:

- **Windows** gets two freebies: `DesktopNativeWidgetAura::UpdateWindowTransparency` makes
  the browser compositor transparent for essentially every native-framed window regardless
  of CEF settings (`desktop_window_tree_host_win.cc:620-628`), and DWM composites per-pixel
  alpha unconditionally (`hwnd_message_handler.cc:1848-1855`: sheet-of-glass
  `DwmExtendFrameIntoClientArea` + premultiplied DirectComposition surface). No opaque-region
  negotiation, no NSWindow flag to get right.
- **macOS** window-level plumbing is COMPLETE in-tree once `kTranslucent` is set:
  `native_widget_mac_ns_window_host.mm:652-676` gives the browser compositor a transparent
  clear color, and `native_widget_ns_window_bridge.mm:1496-1498` auto-applies
  `[window setOpaque:NO] + clearColor`. (The `c87bca497` deferred-SetBackgroundColor hack is
  Linux-specific in effect; redundant-but-harmless on mac.) The `2720ba103` gate means
  `kTranslucent` requires `IsFrameless()==true` AND global `CefSettings.background_color`
  alpha==0 — both already true in AgentMux. **So with the patched framework actually
  bundled, macOS window-level transparency should materialize; what remains is the renderer
  (P-A/P-B below) and promoted layers.**
- **Linux** (DesktopNativeWidgetAura + ozone) needed the hand-wired compositor color
  (`c87bca497`) and still needs the renderer-side page-base patch (P-A).
- Footnote: `BeginWindowDrag` is `BUILDFLAG(IS_OZONE)`-gated — it **returns false on
  macOS**. The mac framework verify script keys on that symbol: it proves the fork built
  the binary, not that drag (or transparency) works on mac (§5.4).

```
WINDOWS  ✅  settings → IPC → WS_EX_LAYERED + LWA_ALPHA → DWM fades whole window
             (stock CEF, post-render, per-pixel never attempted)

MACOS    ❌  1. IPC handler: no-op (no NSWindow.alphaValue / setOpaque code exists)
             2. Bundled framework: stock upstream (resolver tier-2 arm64/aarch64 miss;
                release job gated on missing secret) → cascade absent at runtime
             3. Even with patched framework: RWHVMac WHITE substitution (#1335 patch 2,
                lost) + Blink white page base (#1335 patch 3 / Linux patch 1, lost)
             4. Even with those: promoted pane layers rasterize opaque (open question)

LINUX    ❌  1. IPC handler: no-op (no _NET_WM_WINDOW_OPACITY code exists)
             2. Patched libcef.so shipped (148.0.20-2) and cascade verified firing:
                window borders/gaps DO show the desktop already
             3. Pane interiors opaque: renderer-side Blink page-base patch NOT YET
                IMPLEMENTED (confirmed root cause, design written, needs rebuild → -3)
             4. Promoted-layer question presumed shared with macOS
             5. Default ozone is X11/XWayland — nearly all evidence gathered on native
                Wayland; X11-path ARGB never validated
```

---

## 3. Track 1 — OS-level uniform window opacity (parity with Windows, ~1–2 days)

Implement the non-Windows branches of the *existing* IPC commands with each OS's
whole-window alpha primitive. No CEF changes, no frontend changes (the IPC already fires
with the right values), no settings changes.

### 3.1 macOS: `NSWindow.alphaValue`

In `transparency.rs` (+ a small UI-thread task in `ui_tasks.rs`, since AppKit calls must run
on the main thread — use the existing CEF UI-task post pattern):

1. Resolve the target window: the label→window map already exists for window commands;
   `CefWindow::get_window_handle()` on macOS returns the content `NSView*`.
2. `[view window]` → NSWindow; `[nswindow setAlphaValue: opacity]` (f64).
   Restore = `setAlphaValue: 1.0`.
3. Use the established raw-libobjc idiom (`objc_msgSend` transmute per signature — see
   `run_macos_native_drag_loop`). No new crates.
4. Wire both `set_window_transparency` (all windows / by label) and `set_window_opacity`
   (per-window slider) through the same helper, mirroring the Windows arms including the
   `WindowOpacityApplied`/`WindowOpacityCleared` reducer events (reagent #868 lesson: handle
   BOTH arms or windows stick translucent).

Notes:
- `alphaValue` fades window + shadow at the WindowServer — the exact analogue of LWA_ALPHA.
- Do NOT touch `setOpaque:`/`backgroundColor` in Track 1 — that's the per-pixel knob and
  interacts with the cascade; keep tracks orthogonal.
- Works identically on stock and patched frameworks.

### 3.2 Linux: `_NET_WM_WINDOW_OPACITY`

Default ozone platform is X11/XWayland (`docs/linux.md`), where the EWMH property is the
standard uniform-alpha mechanism and is honored by Mutter, KWin, picom, xfwm4 — including
for XWayland clients.

1. `CefWindow::get_window_handle()` on Linux returns the X11 `Window` id.
2. Set property `_NET_WM_WINDOW_OPACITY` (CARDINAL/32) = `(u32)(opacity * 0xFFFFFFFF)` on the
   **toplevel** window; delete the property to restore opaque.
3. Implementation: add a linux-only `x11rb` dependency (tiny, pure-Rust XCB) in
   `agentmux-cef`; one connect + ChangeProperty call. (Alternative: some WMs want the
   property on the WM frame — Mutter/KWin re-read it from the client window; verify on
   Mutter first, add the frame-walk only if needed.)
4. Native-Wayland ozone (`AGENTMUX_OZONE_PLATFORM=wayland`, opt-in): there is **no**
   client-side uniform-alpha protocol. Log once + no-op; per-pixel (Track 2) is the only
   route there. (This asymmetry is why the renderer-alpha work must still be finished.)

### 3.3 Shared

- Keep the IPC contract unchanged (`transparent`, `opacity`, `label`; `blur` stays dead —
  see §6 open questions).
- Clamp opacity to the same [0.35, 1.0] the slider uses.
- Reducer/audit flow (`HostCommand::SetWindowOpacity`) is already platform-neutral — only
  the side-effect arms grow mac/linux variants.

### 3.4 Verification (required, per the #947 lesson)

- **macOS:** run packaged/dev app with `window:transparent=true, window:opacity=0.5` over a
  saturated wallpaper. Capture caveat from #1335 applies to *per-pixel*; `alphaValue` is a
  WindowServer effect and **should** appear in `screencapture` — but verify by eye first,
  from this iTerm session (outside the AgentMux process tree, with iTerm's Screen Recording
  permission) `screencapture -R<rect>` + pixel-compare is expected to work.
- **Linux:** X11 session or XWayland under Mutter + KWin; `xprop` to confirm the property;
  pixel-compare a desktop-visible region. Confirm no interaction with the already-working
  border/gap per-pixel transparency from the patched libcef.
- Add the smoke check from §5.3 while here.

---

## 4. Track 2 — per-pixel renderer alpha (the glass endgame)

### 4.1 Re-author + land the two lost Chromium patches into the fork patch system

Both are precisely described; re-authoring is mechanical:

- **P-A (platform-agnostic, Linux root cause): renderer-side page-base override.**
  Per the consolidated retro design: in `libcef/renderer/` (render-frame init path +
  re-apply on renderer swaps), when transparency is armed, call
  `WebViewImpl::SetBaseBackgroundColorOverrideTransparent(true)` (via `blink_glue` or a
  small public-API addition). Land as a proper `patch/patches/` entry (or libcef source
  commit) on `agentmux/7778-drag-rightclick-and-transparency`.
- **P-B (macOS-only): RWHVMac white substitution.**
  `render_widget_host_view_mac.mm:1683` — don't substitute WHITE for explicitly-transparent
  views. Must go into the CEF `patch/patches/` system (it's a Chromium-side file) so it
  survives `patcher.py` runs and is reproducible cross-machine — the June loss happened
  precisely because it lived only in a working tree.

### 4.2 Rebuild + release

- **macOS:** incremental rebuild on this machine (dcheck-off script path; fix
  `rebuild-mac-cef.sh` ninja target while there). Publish `cef-macos-arm64-148.23.21`.
  Include `2720ba103` (frameless gate) for parity with Linux.
- **Linux:** needs the Linux build box (`~/cef-build` there). Publish
  `cef-linux-x86_64-148.0.20-3`. The design + patch inventory are in the retro; expected
  1–2 days including verification.

### 4.3 Attack the promoted-layer wall (the open question)

Ordered experiment ladder (each step cheap → expensive, macOS first since the framework
rebuild loop is local):

1. **Electron-parity probe:** add `SetContentBackgroundColor(SK_ColorTRANSPARENT)` to the
   cascade observer (`browser_view_impl.cc` `TransparencyApplyOnRenderReady`), rebuild,
   observe pane interiors. (#1335 candidate 1 — "small, targeted, research-backed".)
2. **De-promotion diagnostic (no rebuild):** in DevTools on a transparency-armed build,
   rewrite the tile layout's pane `transform: translate(x,y)` to `top/left` positioning
   (`frontend/layout/lib/utils.ts:78` produces the transform). If pane interiors go
   transparent → promotion confirmed as the trigger; evaluate a real layout-mode switch
   (perf risk: layout thrash on drag/resize; terminal canvases may re-promote anyway).
3. **If both fail:** instrument `cc` tile rasterization (the session-2/3 diagnostic patches
   are all documented with their outcomes — start from `SafeOpaqueBackgroundColor` /
   `PictureLayerImpl` missing-tile fallback, but re-read the dead-ends list first).
4. Cross-check Electron source for macOS transparent-window specials beyond
   `SetContentBackgroundColor` (e.g. `kCALayerContentsOpaque`, guest-view flags) — Electron
   on the same Chromium does full transparent windows with composited content, so a finite
   diff of switches/calls exists to be found.

### 4.4 Definition of done (Track 2)

Over a saturated wallpaper with `window:transparent=true, window:opacity≈0.45`:
- window borders/gaps AND pane interiors pixel-sample with wallpaper bleed (both platforms);
- no white flash at startup (the 0.45 scss default + pre-paint hint already guard this);
- opaque mode (`window:transparent=false`) pixel-identical to today;
- verified on macOS (ANGLE-Metal GPU), Linux Wayland+Mutter, Linux X11 (the default path!),
  and a `--disable-gpu` run of each (the compositing regime changes the failure modes).

---

## 5. Infrastructure fixes (small, do alongside Track 1)

### 5.1 Fix the macOS framework staging/resolution mismatch
Pick ONE canonical dir — recommendation: keep the resolver as-is (`darwin/aarch64`, matches
cef-dll-sys naming) and change the two build scripts + BUILD_PLAN to stage `aarch64`, or
stage BOTH via symlink `arm64 → aarch64` for muscle-memory compatibility. Also `ditto` the
existing 2026-07-01 build into the canonical dir NOW so the next `task dev` picks it up.

### 5.2 macOS transparency capture story
Grant iTerm (or the invoking shell's parent) Screen Recording permission and verify
`screencapture` from OUTSIDE the AgentMux process tree captures true window alpha; if not,
fall back to `CGWindowListCreateImage` from a helper binary with the permission, or a
second machine/phone photo for the human loop. Required before any Track 2 iteration —
the June session burned enormous time on human-in-the-loop verification.

### 5.3 Transparency smoke test (the #947 gap)
Scriptable check per platform: launch with transparency on over a solid-magenta desktop,
sample N fixed pixels (border/gap + pane interior), assert wallpaper bleed within
tolerance; run in CI where a compositor is available (Linux: Xvfb won't composite —
use `weston --headless` + screenshot, or mutter nested; macOS: runner permitting).
Even a manual `task verify:transparency` script beats eyes-only.

### 5.4 Dev-bundle patch gate (macOS) + meaningful verify symbol
`bundle:darwin` should run `verify-cef-framework-darwin.sh` and **warn loudly** (not fail)
when bundling an unpatched framework — mirroring Linux's advisory tier. Today it silently
clobbers (the #1335 trap 1). Additionally, the verify script keys on `BeginWindowDrag` —
an OZONE-gated stub that does nothing on macOS and says nothing about transparency. Add a
transparency-cascade symbol (e.g. `TransparencyApplyOnRenderReady`) to the check so "verified
patched" actually covers the feature being shipped.

### 5.5 Fix `rebuild-mac-cef.sh`
ninja target `cef` → `cef_framework`; otherwise the canonical-tip refresh path stays broken
and future rebuilds keep silently building stale trees.

---

## 6. Open questions

1. **Product intent for `window:opacity` once Track 2 lands:** uniform alpha and per-pixel
   glass are visually different. Proposal: `window:transparent=true` + patched runtime →
   per-pixel (CSS-driven, Track 2); `window:opacity < 1` additionally applies the OS-level
   uniform fade (Track 1) on all platforms — matching what Windows users already see.
2. **`window:blur`:** dead on all platforms today. Windows DWM Acrylic/Mica code was
   deleted; macOS vibrancy (`NSVisualEffectView`) never existed in the CEF host; Wayland has
   no protocol. Either implement per-platform (mac vibrancy is easy; Windows re-add
   `DwmSetWindowAttribute`; Linux KDE-only hint) or remove the setting from the UI.
3. **Windows LWA_ALPHA vs healthy DirectComposition** (05-21 spec contradiction): if user
   reports Windows still works post-#1354 GPU fix, document that and close the contradiction;
   if it regressed for some users, Track 1's Windows arm may eventually need the same
   per-pixel path (fork has no Windows binary today).
4. **Native-Wayland uniform alpha:** none exists; acceptable to document per-pixel as the
   only Wayland route?
5. **Fork hygiene:** ALL four release tags point at the same unrelated upstream commit
   (`05d7a2476`) — retag from the actual build commits or document the binary's embedded
   version string as the only authoritative provenance.

---

## 7. Sequencing

```
Week 0 (now):
  [1d]  Track 1 macOS (NSWindow.alphaValue) + Linux (_NET_WM_WINDOW_OPACITY)
  [.5d] §5.1 staging fix + §5.4 dev-bundle warning + §5.5 script fix
  [.5d] §5.2 capture story + §5.3 smoke script (macOS first)
        → USER-VISIBLE PARITY: transparency "works" on all three platforms

Week 1:
  [1d]  P-A renderer page-base patch → fork branch, patch/patches entry
  [.5d] P-B RWHVMac patch → patch/patches entry
  [1d]  mac rebuild 148.23.21 + pixel-verify root/gaps AND pane interiors
        (P-A may fix pane interiors outright on Linux-style symptoms — the
         consolidated retro predicts it; #1335's macOS evidence says maybe not)
  [1-2d] linux rebuild 148.0.20-3 on the Linux box + verify (X11 AND Wayland)

Week 2+ (only if pane interiors still opaque):
  experiment ladder §4.3 (SetContentBackgroundColor → de-promotion → cc/Electron diff)
```

## 8. Effort/risk assessment — can this be fixed?

**Yes, with high confidence, in two independently shippable stages.**

- Track 1 is essentially risk-free and delivers the user-visible ask ("works like
  Windows") in a day or two of host-Rust work. Nothing about it has ever been tried and
  failed — it simply was never implemented (`transparency.rs:60`).
- Track 2's Linux blocker is CONFIRMED with a written patch design (retro §"The patch");
  the macOS deltas are precisely documented in #1335. The only genuine research risk is
  the promoted-layer behavior, which has a ranked, evidence-based experiment ladder and an
  existence proof (Electron) that it is solvable on the same Chromium.
