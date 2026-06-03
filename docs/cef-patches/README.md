# Patched CEF framework for macOS 26 (process_requirement / renderer crash)

## The problem

On **macOS 26 (Tahoe)**, a signed, packaged AgentMux `.app` crash-looped its **renderer**
subprocess. cef-debug logged:

```
process_requirement.cc:165 Unable to derive validation category for current process.
  Signature validation … failed … Error -67030 (errSecCSReqFailed)
```

Chromium's **Mach-port rendezvous** validates that subprocesses satisfy a code-signing
`ProcessRequirement` before handing them their IPC ports. Our self-reexec CEF helper fails that
check on macOS 26, so the rendezvous **denies the renderer its ports** → the renderer can't connect
→ crash-loop. Notarization, stapling (app + helper), entitlement changes, and the runtime
`--disable-features=MachPortRendezvous*` flag all **failed** to fix it — the policy is read before
the FeatureList initializes, so the runtime flag never applies.

## The fix (three parts)

1. **Patched CEF framework** — `agentmux_disable_mach_rendezvous_validation.patch` forces
   `base/apple/mach_port_rendezvous_mac.cc::GetPeerValidationPolicy()` to return `kNoValidation`,
   disabling the peer validation at the source (the only place it reliably applies). Acceptable for
   a local single-user desktop app (the check is anti-injection hardening; AgentMux's threat model
   doesn't require it — same class of tradeoff as the OSCrypt switches).
2. **`dcheck_always_on=false` (CRITICAL build flag).** A from-source Chromium with
   `is_official_build=false` defaults `dcheck_always_on=true`, enabling DCHECKs — developer-only
   assertions that production CEF (and the cef-dll-sys prebuilt) compile out. On macOS 26 several
   macOS-specific DCHECKs fail (`ScopedSendingEvent` CrAppControlProtocol conformance on every drag
   event, `CefShutdownChecker` on exit, `util_mac::BasicStartupComplete`), causing SIGABRT on pane
   drag and window close. Building with `dcheck_always_on=false` matches the production config and
   eliminates them. See `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md`.
3. **Per-type helper apps** — the from-source CEF framework uses CEF's standard macOS layout:
   renderer/GPU/utility run as `AgentMux Helper (<Type>).app` (Renderer/GPU/Plugin/Alloy + generic
   + Alerts). `scripts/package-macos.sh` creates all six.

Result: the signed + **notarized** `.app` launches with a **working, stable renderer** and a live UI.

## How to rebuild the framework

```bash
# 1. depot_tools + automate-git (CEF branch 7778 = Chromium 148.0.7778, matches the `cef` crate)
mkdir -p ~/cef-build && cd ~/cef-build
git clone https://chromium.googlesource.com/chromium/tools/depot_tools.git
curl -o code/automate-git.py https://raw.githubusercontent.com/agentmuxai/cef/master/tools/automate/automate-git.py
export PATH="$PWD/depot_tools:$PATH"
export GN_DEFINES="is_official_build=false is_debug=false symbol_level=1 dcheck_always_on=false"
python3 code/automate-git.py --download-dir=$PWD/chromium --depot-tools-dir=$PWD/depot_tools \
  --branch=7778 --arm64-build --no-debug-build --no-build --no-distrib

# 2. apply the patch
cd chromium/chromium/src
git apply <agentmux>/docs/cef-patches/agentmux_disable_mach_rendezvous_validation.patch

# 3. generate + build (just the framework)
cd cef && GN_DEFINES="is_official_build=false is_debug=false symbol_level=1 dcheck_always_on=false target_cpu=\"arm64\"" \
  ./cef_create_projects.sh && cd ..
ninja -C out/Release_GN_arm64 -j8 cef_framework
# → out/Release_GN_arm64/Chromium Embedded Framework.framework

# 4. use it from AgentMux
export AGENTMUX_CEF_RUNTIME_DIR_DARWIN=$PWD/out/Release_GN_arm64
cd <agentmux> && task package:macos     # bundles the PATCHED framework, signs, notarizes, staples
```

## Metal (resolved — framework is Metal-accelerated)

The framework is built **with Metal enabled** (`angle_enable_metal=true`, the default) — verified at
runtime: the GPU process loads `Metal.framework` + `AGXMetalG14G` via ANGLE-Metal (hardware accel).

Getting Metal to build required a one-time Xcode-26 toolchain repair. Xcode 26 ships the Metal
compiler as a *separate* downloadable component, and `xcodebuild -downloadComponent MetalToolchain`
was broken by a stale `DVTDownloads.framework` (old `XcodeSystemResources` pkg) whose missing symbol
crashed the download plugin. Fix (run once, with sudo):

```bash
sudo installer -pkg /Applications/Xcode.app/Contents/Resources/Packages/XcodeSystemResources.pkg -target /
xcodebuild -downloadComponent MetalToolchain
xcrun metal --version   # confirm
```

(The first framework was a stopgap built with `angle_enable_metal=false` → CGL/SwiftShader; once the
toolchain was repaired we removed that arg, re-ran `gn gen`, and did an incremental rebuild — only
the ANGLE-Metal objects + `.air` shaders + the `libcef` relink, ~minutes.)

## Fork persistence

The patch is staged at `src/cef/patch/patches/agentmux_process_requirement.patch`. To make builds
reproducible, push it to `agentmuxai/cef` as a branch (e.g. `agentmux/7778-process-requirement`)
+ register in `cef/patch/patch.cfg`, so `cef_create_projects.sh` applies it automatically.
