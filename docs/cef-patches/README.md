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

## The fix (two parts)

1. **Patched CEF framework** — `agentmux_disable_mach_rendezvous_validation.patch` forces
   `base/apple/mach_port_rendezvous_mac.cc::GetPeerValidationPolicy()` to return `kNoValidation`,
   disabling the peer validation at the source (the only place it reliably applies). Acceptable for
   a local single-user desktop app (the check is anti-injection hardening; AgentMux's threat model
   doesn't require it — same class of tradeoff as the OSCrypt switches).
2. **Per-type helper apps** — the from-source CEF framework uses CEF's standard macOS layout:
   renderer/GPU/utility run as `AgentMux Helper (<Type>).app` (Renderer/GPU/Plugin/Alloy + generic).
   `scripts/package-macos.sh` now creates all five.

Result: the signed + **notarized** `.app` launches with a **working, stable renderer** and a live UI.

## How to rebuild the framework

```bash
# 1. depot_tools + automate-git (CEF branch 7778 = Chromium 148.0.7778, matches the `cef` crate)
mkdir -p ~/cef-build && cd ~/cef-build
git clone https://chromium.googlesource.com/chromium/tools/depot_tools.git
curl -o code/automate-git.py https://raw.githubusercontent.com/agentmuxai/cef/master/tools/automate/automate-git.py
export PATH="$PWD/depot_tools:$PATH"
export GN_DEFINES="is_official_build=false is_debug=false symbol_level=1"
python3 code/automate-git.py --download-dir=$PWD/chromium --depot-tools-dir=$PWD/depot_tools \
  --branch=7778 --arm64-build --no-debug-build --no-build --no-distrib

# 2. apply the patch
cd chromium/chromium/src
git apply <agentmux>/docs/cef-patches/agentmux_disable_mach_rendezvous_validation.patch

# 3. generate + build (just the framework)
cd cef && GN_DEFINES="is_official_build=false is_debug=false symbol_level=1 target_cpu=\"arm64\"" \
  ./cef_create_projects.sh && cd ..
ninja -C out/Release_GN_arm64 -j8 cef_framework
# → out/Release_GN_arm64/Chromium Embedded Framework.framework

# 4. use it from AgentMux
export AGENTMUX_CEF_RUNTIME_DIR_DARWIN=$PWD/out/Release_GN_arm64
cd <agentmux> && task package:macos     # bundles the PATCHED framework, signs, notarizes, staples
```

## ⚠️ Metal caveat (important for a SHIP build)

This first framework was built with **`angle_enable_metal=false`** to bypass a broken Xcode-26 Metal
toolchain (`xcodebuild -downloadComponent MetalToolchain` fails on an outdated DVTDownloads from the
old Command Line Tools). The app **works** (ANGLE falls back to CGL/SwiftShader), but the GPU is not
Metal-accelerated.

**For a production framework with Metal:**
```bash
sudo softwareupdate -i 'Command Line Tools for Xcode 26.5'   # fixes the Metal toolchain
# then remove angle_enable_metal=false from out/Release_GN_arm64/args.gn, re-gen, rebuild.
```

## Fork persistence

The patch is staged at `src/cef/patch/patches/agentmux_process_requirement.patch`. To make builds
reproducible, push it to `agentmuxai/cef` as a branch (e.g. `agentmux/7778-process-requirement`)
+ register in `cef/patch/patch.cfg`, so `cef_create_projects.sh` applies it automatically.
