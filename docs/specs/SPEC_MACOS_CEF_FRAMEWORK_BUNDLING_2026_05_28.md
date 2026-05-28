# macOS CEF Framework Bundling for `task dev` and `task package:macos`

**Date:** 2026-05-28
**Status:** Spec / Proposal
**Repo state:** `main` @ `2abe5968` (after PR #1131 lands the macOS compile fix)
**Author:** AgentO-asaf (a5af)
**Related:** #1130 (compile fix, closed by #1131) — this is the next layer

---

## Problem

A fresh `git clone` of `agentmuxai/agentmux` on macOS can now (post-#1131) compile `agentmux-cef` cleanly, but `task dev` still cannot launch the host: it panics within milliseconds of starting at

```
thread 'main' (36953425) panicked at /Users/.../cef-146.7.0+146.0.12/src/library_loader.rs:20:14:
called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound,
                                                  message: "No such file or directory" }
task: Failed to run task "dev": task: Failed to run task "dev:serve": exit status 101
```

Verified by inspecting `~/.cargo/registry/src/.../cef-146.7.0+146.0.12/src/library_loader.rs:8-23`: on macOS, cef-rs's `LibraryLoader::new(current_exe_path, helper=false)` resolves the framework relative to the executable as

```
<dir-of-exe>/../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework
```

and `.canonicalize()`s the path — so if any link in the chain is missing, it panics on unwrap. For the dev layout that places the host at `dist/cef/agentmux-cef`, cef-rs is looking for `dist/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework`. Nothing in the current build pipeline ever puts a Framework there.

`Taskfile.yml::bundle:darwin` is a no-op stub:

```yaml
bundle:darwin:
    internal: true
    platforms: [darwin]
    cmds:
        - echo "macOS bundling not yet implemented"
```

So `task dev` on macOS is structurally incapable of launching the host today, even with `agentmux-cef` compiling. `task package:macos` is also a `[TODO]` stub (`echo "macOS packaging not yet implemented."`), so there is no path to a runnable AgentMux on macOS from this repo.

This is the second of two adjacent macOS gaps. The first (compile) is fixed by #1131. This spec covers the second (runtime CEF framework discovery + bundling), which has to be solved before any new macOS-relevant work — drag, transparency, packaging, even smoke-testing IPC — can proceed.

---

## TL;DR

- Implement `bundle:darwin` to place a real `Chromium Embedded Framework.framework` at `dist/Frameworks/` so the host launched at `dist/cef/agentmux-cef` finds it via cef-rs's `../Frameworks/...` relative lookup.
- Resolve the Framework via a `scripts/resolve-cef-runtime-darwin.sh` helper that mirrors the Linux pattern (`scripts/resolve-cef-runtime.sh`): three-tier lookup with explicit override, standard cef-build layout, and the cef-dll-sys cargo cache as last-resort fallback (unpatched, no drag support — same caveat as Linux).
- Add an architecture detection so Apple Silicon and Intel each get the right Framework binary. cef-rs's `Cargo.lock` resolves to the same crate version (`146.7.0+146.0.12`) for both, but the prebuilt Framework binary differs.
- Defer `task package:macos` (the full `.app`/`.dmg` flow) to a follow-up spec — the Framework bundling is the load-bearing piece, and packaging is mechanical layering once `task dev` boots.
- Estimated change: 1 new ~50-line shell helper, ~30 lines added to `Taskfile.yml::bundle:darwin`, no Rust changes, no new dependencies.

---

## Why this is a separate spec from `patched-libcef-bundling-2026-05-08.md`

That spec covers **Linux** runtime resolution and assumes a patched `libcef.so` is required (for `BeginWindowDrag`). The macOS situation is different in two ways:

1. **No patched libcef.dylib is required for `task dev` to launch.** The compile-time `begin_window_drag` field access on macOS is now feature-gated to `patched-libcef` (default off) by #1131. The runtime `LibraryLoader::new(...).canonicalize().unwrap()` panic will happen with *any* CEF framework — patched or upstream. Solving the panic is independent of the patched-libcef workstream.
2. **macOS uses a Framework bundle, not a flat directory of .so + paks.** The Linux resolver returns a directory; the macOS resolver needs to return a Framework path (or a directory containing one). The on-disk shape, the relative-path expectation, and the eventual `.app` bundle layout are all different.

So while the *shape* of the resolver helper script will be similar to Linux's, the artifact it locates is different and the code path it feeds is also different. Two specs, two helpers, one shared design philosophy.

---

## Current state by platform (for context)

| Platform | Compile | Bundle step | Where libcef comes from | Status |
|---|---|---|---|---|
| **Windows** | OK | `bundle:windows` (Taskfile.yml:473–505) | cef-dll-sys cargo cache (`target/.../out/cef_windows_x86_64/`); `repair-cef-extract.sh` retries on Defender races | Working |
| **Linux** | Broken at HEAD (PR #1131 fixes) | `bundle:linux` (Taskfile.yml:513–536) | `scripts/resolve-cef-runtime.sh` (3-tier) | Working post-#1131 with feature gate, drag deferred |
| **macOS** | Fixed by PR #1131 | `bundle:darwin` (Taskfile.yml:507–511) — **stub** | **Nothing** | **This spec** |

cef-rs already supports the macOS Framework loading model natively. It is only the AgentMux build pipeline that has nothing in place.

---

## Design

### `scripts/resolve-cef-runtime-darwin.sh` (new)

```bash
#!/usr/bin/env bash
# Print the absolute path of the directory that contains a
# `Chromium Embedded Framework.framework` suitable for AgentMux to
# load on macOS.
#
# Resolution order:
#
#   1. $AGENTMUX_CEF_RUNTIME_DIR_DARWIN — explicit override.
#   2. $HOME/cef-build/darwin/<arch>/                       — standard
#      cef-build layout for patched/custom Frameworks
#      (analogous to Linux's $HOME/cef-build/chromium_git/...).
#   3. cef-dll-sys cargo cache: first match of
#      <repo>/target/{debug,release}/build/cef-dll-sys-*/out/cef_macos_<arch>/
#
# Each candidate is validated by checking that
# `<candidate>/Chromium Embedded Framework.framework/Chromium Embedded Framework`
# exists.
#
# stdout: absolute path of the chosen directory (the one that holds
# the .framework, not the .framework itself).
# stderr: progress info + actionable errors.
# exit 0 on success, 1 if no candidate found.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

case "$(uname -m)" in
    arm64)   ARCH="arm64" ;;
    x86_64)  ARCH="x86_64" ;;
    *) echo "❌ unsupported macOS arch: $(uname -m)" >&2; exit 1 ;;
esac

validate_dir() {
    local dir="$1"
    if [ -e "$dir/Chromium Embedded Framework.framework/Chromium Embedded Framework" ]; then
        printf '%s\n' "$dir"
        exit 0
    fi
    return 1
}

# 1. Explicit override
if [ -n "${AGENTMUX_CEF_RUNTIME_DIR_DARWIN:-}" ]; then
    if validate_dir "$AGENTMUX_CEF_RUNTIME_DIR_DARWIN"; then :; fi
    echo "❌ AGENTMUX_CEF_RUNTIME_DIR_DARWIN set but Framework not found at: $AGENTMUX_CEF_RUNTIME_DIR_DARWIN" >&2
    exit 1
fi

# 2. Standard cef-build layout
if validate_dir "$HOME/cef-build/darwin/$ARCH"; then :; fi

# 3. cef-dll-sys cargo cache
for profile in release debug; do
    for cand in "$REPO_ROOT/target/$profile/build/cef-dll-sys-"*/out/"cef_macos_$ARCH"; do
        [ -d "$cand" ] || continue
        if validate_dir "$cand"; then :; fi
    done
done

echo "❌ No Chromium Embedded Framework.framework found in any candidate location:" >&2
echo "   1. \$AGENTMUX_CEF_RUNTIME_DIR_DARWIN (unset)" >&2
echo "   2. \$HOME/cef-build/darwin/$ARCH" >&2
echo "   3. cef-dll-sys cargo cache (target/{debug,release}/build/cef-dll-sys-*/out/cef_macos_$ARCH/)" >&2
echo "" >&2
echo "   Build the cef-dll-sys download step (cargo build -p agentmux-cef will" >&2
echo "   trigger it) or set AGENTMUX_CEF_RUNTIME_DIR_DARWIN to a directory" >&2
echo "   containing the Framework." >&2
exit 1
```

### `Taskfile.yml::bundle:darwin` (replace stub)

```yaml
bundle:darwin:
    internal: true
    platforms: [darwin]
    cmds:
        - |
            # Resolve macOS CEF runtime via shell helper. Three-tier order:
            #   1. $AGENTMUX_CEF_RUNTIME_DIR_DARWIN (explicit override)
            #   2. ~/cef-build/darwin/<arch> (custom builds)
            #   3. cef-dll-sys cargo cache (fallback — public upstream)
            # See docs/specs/SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28.md.
            CEF_DIR="$(bash scripts/resolve-cef-runtime-darwin.sh)"
            echo "Using CEF Framework from: $CEF_DIR"
            mkdir -p dist/Frameworks
            rm -rf "dist/Frameworks/Chromium Embedded Framework.framework"
            # Use ditto to preserve symlinks + metadata correctly (cp -R can
            # break the Versions/Current symlink that the loader follows).
            ditto "$CEF_DIR/Chromium Embedded Framework.framework" "dist/Frameworks/Chromium Embedded Framework.framework"
            echo "✓ Bundled CEF Framework for macOS (dist/Frameworks/)"
```

### Layout produced by `task dev` on macOS

```
dist/
├── cef/
│   └── agentmux-cef            ← host binary
├── Frameworks/
│   └── Chromium Embedded Framework.framework/
│       ├── Chromium Embedded Framework      ← cef-rs loads this
│       ├── Resources/
│       ├── Libraries/
│       └── ...
├── bin/
│   └── agentmux-srv-X.Y.Z-darwin.arm64
└── schema/...
```

cef-rs's path resolution from `dist/cef/agentmux-cef` walks:

```
parent: dist/cef
join("../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework")
        → dist/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework
.canonicalize() → resolves to the absolute path
```

— matches what `bundle:darwin` places. No code change in `agentmux-cef` needed; the layout is exactly what cef-rs expects out of the box.

---

## Alternatives considered

### A. Put the Framework next to the binary (no `Frameworks/` parent)

Skip the `../Frameworks` resolver and embed the Framework directly in `dist/cef/`. **Rejected** — would require patching cef-rs, which we'd then have to maintain as a fork, defeating the point of being on the public crate.

### B. Construct a full `.app` bundle for `task dev`

Generate `dist/AgentMux.app/Contents/{MacOS,Frameworks,Resources}/` for dev too. **Rejected for now** — `.app` bundles add Info.plist authoring, codesign considerations, and Launch Services registration paths that are irrelevant for hot-reload dev. The flat `dist/cef + dist/Frameworks` layout works because cef-rs's relative-path resolver doesn't actually care whether the parent is an `.app` bundle, just whether the relative path resolves. Defer the `.app` layout to `package:macos`, where it's mandatory for Gatekeeper / notarization anyway.

### C. Skip `bundle:darwin` and download/extract the Framework inside the Rust build script

Have `agentmux-cef/build.rs` invoke a `download-cef-darwin` step. **Rejected** — duplicates what cef-dll-sys's build script already does (it downloads the Framework into the cargo cache); we only need to *find* and *copy* the result, which is a build-pipeline concern, not a Rust crate concern. Keeping it in the Taskfile mirrors how Windows and Linux work.

### D. Single cross-platform `resolve-cef-runtime.sh`

Add macOS branches to the existing Linux helper. **Rejected** — the function shape diverges (Linux returns a flat dir with libcef.so + paks; macOS returns a dir containing a Framework). Two scripts is clearer than one with `case "$(uname)" in` everywhere; the helpers stay independent and the Taskfile picks the right one via `{{OS}}`.

---

## Acceptance criteria

A fresh clone on macOS (Apple Silicon, default toolchain):

```bash
git clone git@github.com:agentmuxai/agentmux.git
cd agentmux
task dev
```

…compiles, bundles, launches, and shows a window. No manual setup. Two ticks specifically:

1. `task bundle:darwin` succeeds and produces `dist/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework` as a real file (not a broken symlink), with `Versions/Current` and `Versions/A` intact (verified via `ditto`, not `cp -R`).
2. The host process started by `dev:serve` does not panic in `cef-146.7.0+146.0.12/src/library_loader.rs:20`. A window opens. Vite hot reload remains functional.

Intel Macs: same flow, with `cef_macos_x86_64` candidate paths matched instead of `cef_macos_arm64`.

---

## Out of scope (explicitly)

- **`task package:macos`** (the `.app`/`.dmg` flow). Tracked separately. Depends on this spec landing first.
- **Patched libcef.dylib on macOS**. The macOS analogue of the `BeginWindowDrag` patch isn't authored yet (`a5af/cef`'s `agentmux/7680-drag-rightclick-and-transparency` branch is Linux/Wayland-focused). Macs don't need it for window drag — AppKit handles title-bar drag natively for windows with the standard title bar mask, and the AgentMux borderless-window drag IPC can route through a macOS-native code path when the time comes.
- **Codesigning / notarization**. Pure dev workflow; relevant only at package time.
- **CI build matrix**. Filed separately (the `release-consistency.yml`-only gap is well documented in #1131 and in the `bug(macOS)` issue #1130).
- **Restoring Linux native drag**. Tracked via PR #1131 reviewer thread; needs a re-published patched `cef-dll-sys` fork, out of scope here.

---

## Open questions

1. Does cef-dll-sys 146.7.0's `download-cef` step actually download a macOS Framework into the cargo cache on `cargo build`? Spot-check on first implementation. If yes, candidate #3 in the resolver is real; if no, the resolver effectively becomes two-tier (override + custom cef-build layout) and we need to either bring back option C (Rust-side download) or document a one-time manual download step.
2. Universal binary support — should `bundle:darwin` produce a universal Framework via `lipo` so a single binary runs on both arm64 and x86_64? Probably no for `task dev` (devs are on a specific arch); maybe yes for `package:macos`. Defer to that spec.
3. The Framework binary is ~600 MB on disk. Worth thinking about whether `task clean` should keep `dist/Frameworks/` to avoid re-copying ~600 MB on every clean rebuild. Mirrors the same tradeoff Linux already navigates with `dist/cef/libcef.so`.
