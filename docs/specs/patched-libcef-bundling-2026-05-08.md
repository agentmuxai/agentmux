# Patched libcef.so Bundling for Linux

**Date:** 2026-05-08
**Status:** Spec / Proposal
**Repo state:** main @ `d7dc58c1`, AgentMux v0.33.703
**Author:** AgentC

---

## Problem

AgentMux's Linux build needs **two patches to `libcef.so`** that are not in upstream CEF:

1. **`CefWindow::BeginWindowDrag()`** — added by [a5af/cef PR #1](https://github.com/a5af/cef/pull/1) (merged into the `agentmux/7680-drag-rightclick-and-transparency` branch). Needed by AgentMux PR #663 for left-click window drag on Linux/Wayland (`agentmux-cef/src/ui_tasks.rs::StartWindowDragTask` calls `cef::sys::_cef_window_t::begin_window_drag` via raw FFI; the slot doesn't exist in the upstream `_cef_window_t` struct).
2. **Transparency broadening** for views-hosted browsers (cherry-pick of Chad Nelson's `SetBackgroundOpaque(false)` change) — needed for the eventual transparency feature.

The patches live in a built `libcef.so` at `~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/libcef.so` (642 MB stripped, head `5ab41b6` on the `agentmux/7680-...` branch — see `~/.claude/projects/-home-snowbark/memory/cef_build_in_progress.md`).

**The pain:** `task bundle:linux` and `scripts/build-appimage-linux.sh` both source `libcef.so` from cef-dll-sys's cargo cache (`target/debug/build/cef-dll-sys-*/out/cef_linux_x86_64/`), which is the **upstream unpatched** prebuilt downloaded by cef-dll-sys's build script. Every clean build silently regresses to upstream libcef, breaking left-click drag (and, in the future, transparency).

Workaround today: manually overlay the patched libcef + matching V8 paks after every `task bundle` run. Easy to forget, time-consuming, and the only diagnostic when forgotten is "left-click drag silently does nothing" (the runtime ABI guard logs a warning but the user-visible failure mode looks like an unrelated bug).

**Until the BeginWindowDrag patch is upstreamed to chromiumembedded/cef and cef-dll-sys is bumped past it**, we need a sticky way to inject our patched runtime into the build pipeline.

---

## TL;DR

- Add a `CEF_RUNTIME_DIR` resolver to `bundle:linux` and `scripts/build-appimage-linux.sh` with three sources, in priority order:
  1. `$AGENTMUX_CEF_RUNTIME_DIR` env var (explicit override).
  2. `~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/` (the cef-build standard layout for anyone following `docs/cef-build/...`).
  3. The cef-dll-sys cargo cache (`target/.../cef_linux_x86_64/`) — current behavior.
- Add a sanity-check at copy time: warn (and exit non-zero in CI mode) if the chosen `libcef.so` is the unpatched upstream — detected by the same `_cef_window_t.size` field the runtime ABI guard reads, or by a simpler heuristic (file size: patched is ~613 MB stripped, unpatched debug is ~1.3 GB).
- Factor the resolver into one shell helper (`scripts/resolve-cef-runtime.sh`) so the Taskfile bundle command and the AppImage build script share one source of truth.
- Document the build-from-source path for libcef in `docs/cef-build/build-patched-libcef.md` (existing build notes from `cef_build_in_progress.md` memory) and link from this spec.
- Estimated change: 1 new ~50-line shell helper, 1 new docs page, ~10 line edits to `Taskfile.yml` (`bundle:linux`) and `scripts/build-appimage-linux.sh`. No Rust changes. No new versions to bump.

---

## Why a script-level resolver, not a cef-dll-sys vendor

Three architectural alternatives were considered and rejected:

### A. Vendor the patched libcef in-tree

A `vendored/cef/linux-x64/libcef.so` (~600 MB) committed to the repo. Pros: dead-simple bundle path. Cons: 600 MB binary in git history, infeasible. Even with git-lfs, fetch cost on every clone is unacceptable. Rejected.

### B. Patch cef-dll-sys's build.rs to swap in our libcef

Override the cef-dll-sys build script to fetch our patched binary instead of upstream. Pros: single source of truth, all downstream consumers get the right libcef automatically. Cons: requires forking cef-dll-sys (or vendoring + patching), version-pinning becomes painful, every `cargo update` could blow it away (we already see this with the `_cef_window_t` size assert patch we maintain in cef-dll-sys's binding cache). Adds a third place to track. Rejected.

### C. Custom cef-runtime crate

A new crate that knows how to locate / download the patched libcef and is invoked from the build pipeline. Pros: clean abstraction. Cons: overkill for one binary file with one platform; adds a Rust crate, a build dependency, and a new versioning surface. Rejected.

### D. (Chosen) Shell resolver invoked from Taskfile + AppImage build script

The bundle path is already shell-driven (`bundle:linux` is a Taskfile shell command, `build-appimage-linux.sh` is a shell script). Adding a `resolve-cef-runtime.sh` that prints a path to stdout is the smallest possible change. The resolver knows three locations, prefers the patched ones, falls back gracefully. Easy to read, easy to override per developer. **No new dependencies.**

---

## Design

### `scripts/resolve-cef-runtime.sh` (new)

```bash
#!/usr/bin/env bash
# Print the absolute path of the directory containing libcef.so + paks
# that the bundle / AppImage build should use. Resolution order:
#
#   1. $AGENTMUX_CEF_RUNTIME_DIR — explicit override
#   2. $HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64
#      (the standard cef-build layout; documented in
#       docs/cef-build/build-patched-libcef.md)
#   3. The cef-dll-sys cargo cache — first match of
#      target/{debug,release}/build/cef-dll-sys-*/out/cef_linux_x86_64
#
# After resolution: if the libcef.so at the chosen path lacks the
# AgentMux patches (heuristic: file size ≥ 1 GB suggests an unpatched
# upstream debug build, AND cef::sys::_cef_window_t was bumped 888 → 896
# in our cef-dll-sys binding patch — the runtime ABI guard will catch
# the mismatch but the build path can warn earlier).
#
# Exit 0 with the resolved dir on stdout. Exit 1 with a clear error if
# nothing matches.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

candidates=()
if [ -n "${AGENTMUX_CEF_RUNTIME_DIR:-}" ]; then
    candidates+=("$AGENTMUX_CEF_RUNTIME_DIR")
fi
candidates+=("$HOME/cef-build/chromium_git/chromium/src/out/Release_GN_x64")
# cef-dll-sys cache — find first match (debug or release).
while IFS= read -r d; do
    candidates+=("$d")
done < <(find "$REPO_ROOT/target" -maxdepth 6 -type d -name 'cef_linux_x86_64' 2>/dev/null)

for dir in "${candidates[@]}"; do
    if [ -f "$dir/libcef.so" ] && [ -f "$dir/icudtl.dat" ]; then
        # Sanity: warn if libcef.so is suspiciously large (>1GB suggests
        # unstripped upstream debug build, almost certainly unpatched).
        size_bytes=$(stat -c %s "$dir/libcef.so" 2>/dev/null || stat -f %z "$dir/libcef.so")
        size_mb=$((size_bytes / 1024 / 1024))
        if [ "$size_bytes" -gt 1073741824 ]; then
            echo "WARNING: libcef.so at $dir is ${size_mb} MB (>1 GB) — this is likely the unpatched upstream debug build." >&2
            echo "         AgentMux's BeginWindowDrag FFI override will silently no-op (left-click drag broken)." >&2
            echo "         Build the patched libcef per docs/cef-build/build-patched-libcef.md, then set" >&2
            echo "         AGENTMUX_CEF_RUNTIME_DIR=/path/to/your/Release_GN_x64 to override." >&2
            # Don't exit — let the caller decide. (CI can re-check by passing --strict.)
        fi
        echo "$dir"
        exit 0
    fi
done

echo "ERROR: could not find libcef.so in any of these locations:" >&2
for c in "${candidates[@]}"; do echo "  - $c" >&2; done
echo "Build the patched libcef (docs/cef-build/build-patched-libcef.md) or run \`cargo build\` to populate cef-dll-sys cache." >&2
exit 1
```

### `Taskfile.yml::bundle:linux` (edit)

```yaml
bundle:linux:
    internal: true
    platforms: [linux]
    cmds:
        - |
            CEF_DIR="$(bash scripts/resolve-cef-runtime.sh)"
            echo "Using CEF runtime from: $CEF_DIR"
            mkdir -p dist/cef/locales
            cp -f "$CEF_DIR/libcef.so" dist/cef/
            cp -f "$CEF_DIR/libEGL.so" dist/cef/ 2>/dev/null || true
            cp -f "$CEF_DIR/libGLESv2.so" dist/cef/ 2>/dev/null || true
            cp -f "$CEF_DIR/chrome-sandbox" dist/cef/ 2>/dev/null || true
            cp -f "$CEF_DIR/chrome_crashpad_handler" dist/cef/ 2>/dev/null || true
            cp -f "$CEF_DIR/icudtl.dat" dist/cef/
            cp -f "$CEF_DIR/snapshot_blob.bin" dist/cef/ 2>/dev/null || true
            cp -f "$CEF_DIR/v8_context_snapshot.bin" dist/cef/ 2>/dev/null || true
            cp -f "$CEF_DIR"/*.pak dist/cef/
            cp -f "$CEF_DIR/locales/en-US.pak" dist/cef/locales/ 2>/dev/null || true
            echo "✓ Bundled runtime for Linux"
```

Replaces the inline `find target -name "libcef.so" ...` lookup. The shell helper keeps the resolution logic in one place.

### `scripts/build-appimage-linux.sh` (edit)

The AppImage script doesn't currently re-pull from cef-dll-sys — it copies from `dist/cef/` after `task bundle` already ran. So the AppImage path is fine as long as `bundle:linux` is fixed. **Verify post-fix** by running `task package:linux` clean and inspecting the AppImage's `usr/bin/libcef.so` size.

### `docs/cef-build/build-patched-libcef.md` (new)

Build instructions consolidating the existing notes from the `cef_build_in_progress.md` memory:

- Clone `agentmux/cef` (the AgentMux fork) at the `agentmux/7680-drag-rightclick-and-transparency` branch.
- Configure with `gn gen out/Release_GN_x64 --args='is_debug=false symbol_level=1 ...'` (full args from the build memory).
- `autoninja -C out/Release_GN_x64 cef -j 12 -l 16` (with the `-j 12 -l 16` discipline that avoided OOM-cascade reboots — see memory).
- Set `AGENTMUX_CEF_RUNTIME_DIR=$(pwd)/out/Release_GN_x64` for downstream Taskfile commands, or move the directory to `~/cef-build/.../Release_GN_x64` to use the auto-detect path.
- Note: this is a multi-hour build; expect ~600 MB output; intended for AgentMux maintainers, not every contributor.

For non-maintainers contributing without local libcef builds, the resolver's fallback to cef-dll-sys cache means they get a usable (if drag-broken) build. The README will note this trade-off.

---

## Design decisions

### D1. Three-tier resolution order, env var first

Env var wins so a maintainer with multiple cef builds can switch between them without moving directories. Default path (`~/cef-build/...`) wins when env var is unset because that's the canonical layout the build doc produces. cef-dll-sys cache is the floor — anyone who runs `cargo build` gets *something* that boots, even if drag is broken.

### D2. Warn, don't fail, on size heuristic

The size check (`>1 GB → likely unpatched`) is a heuristic, not a contract. There exist legitimate scenarios where size doesn't predict patched-ness (a stripped upstream build would be small). Hard-failing on size would be wrong. Print to stderr, continue. CI can grep for the warning and fail there if desired (out of scope for this PR).

### D3. No content fingerprint

We considered `nm -D libcef.so | grep BeginWindowDrag` to verify the patch is actually in the binary. Two reasons against:

1. The symbol is **not** exported as a dynamic symbol (it's an internal C++ method on `CefWindowImpl`, called via the `_cef_window_t` function-pointer slot). `nm -D` won't see it. Using `nm` (without `-D`) requires the binary not be stripped, which our patched build IS. So a name-based check doesn't work.
2. The runtime ABI guard (`agentmux-cef/src/browser_pane/creation_views.rs`-style code, also in `ui_tasks.rs::StartWindowDragTask`) already catches the mismatch by reading `_cef_window_t.size` and comparing against `size_of::<_cef_window_t>()`. If the user runs an unpatched libcef, the runtime warns and silently no-ops the drag — same UX failure mode as today. Adding a build-time fingerprint adds maintenance for marginal value.

The size heuristic is a soft signal; the runtime guard is the contract.

### D4. Shell, not Rust

A `cargo xtask` Rust binary could do the same job with stronger types and easier testing. Not chosen because:

- The bundle pipeline is already shell. Adding a Rust step means a `cargo build --bin xtask` upstream of the bundle, which is slower and harder to invoke from `Taskfile.yml`.
- The logic is ~30 useful lines of POSIX shell. Easier to read and modify than the equivalent Rust + clap + std::process plumbing.
- No persistent state, no concurrency, no complex conditions.

If the resolver grows non-trivially (e.g. checksum verification, automatic download from a build server), revisit.

### D5. Out of scope: macOS / Windows

The same problem theoretically exists on macOS once that build comes online (we'd need a patched `Chromium Embedded Framework.framework` for `BeginWindowDrag` there too). Windows uses the upstream libcef.dll today because the Windows pane path doesn't need `BeginWindowDrag`. **This spec is Linux-only.** macOS will need a parallel resolver when its build flow lands; the shell helper can grow a `case "$(uname)"` then.

### D6. Versioning the patched libcef

Today the patched libcef has no version of its own — it's whatever `5ab41b6` on `agentmux/7680-...` produced. If we bump cef-dll-sys to a new chromium release, we'd need to rebase our patches on the new base and rebuild libcef. The resolver doesn't help with that — it just locates whatever's there. **Long-term cure**: upstream `BeginWindowDrag` to chromiumembedded/cef so cef-dll-sys's prebuilt becomes sufficient. Track in a follow-up.

---

## Implementation plan

1. Add `scripts/resolve-cef-runtime.sh` as specified above. `chmod +x`.
2. Replace the inline `CEF_DIR=$(find target ...)` lookup in `Taskfile.yml::bundle:linux` with `CEF_DIR="$(bash scripts/resolve-cef-runtime.sh)"`.
3. Smoke-test:
   - `unset AGENTMUX_CEF_RUNTIME_DIR && task bundle` → resolver picks `~/cef-build/...` (patched), libcef.so in dist/cef is ~613 MB.
   - `AGENTMUX_CEF_RUNTIME_DIR=/some/other/path task bundle` → resolver picks override.
   - `mv ~/cef-build /tmp/hide && task bundle && mv /tmp/hide ~/cef-build` → resolver falls back to cef-dll-sys cache, prints "WARNING ... unpatched upstream" if size > 1 GB.
4. `task package:linux` end-to-end: build AppImage, extract, verify `usr/bin/libcef.so` is the patched 613 MB version.
5. Run agentmux from the AppImage; verify left-click drag works (the symptom that originally surfaced this issue).
6. Commit `docs/cef-build/build-patched-libcef.md` consolidating the build memory.
7. Bump patch version, open PR.

---

## Test plan

- [ ] `bash scripts/resolve-cef-runtime.sh` with `AGENTMUX_CEF_RUNTIME_DIR=/tmp/no-such-dir` falls through to next candidate (silently — env var pointing at a nonexistent dir is a soft signal, treat as unset).
- [ ] With env var set to a valid patched build dir, resolver prints that path and exits 0.
- [ ] Without env var, resolver finds `~/cef-build/.../Release_GN_x64` and prints it.
- [ ] After `cargo clean -p agentmux-cef && cargo build --release -p agentmux-cef && task bundle`, dist/cef/libcef.so is the patched 613 MB version, not cef-dll-sys's 1.3 GB upstream.
- [ ] `task package:linux` builds an AppImage; extracted AppImage contains the patched libcef (size + size-of-libcef-content cross-check).
- [ ] `task dev` no longer regresses the patched libcef on every restart (its bundle step uses the resolver too).
- [ ] On a machine without the patched build, resolver prints a clear warning and falls back to cef-dll-sys cache; agentmux launches but left-click drag is broken (acceptable degraded mode for non-maintainer contributors).

---

## Risks / non-goals

- **Risk: cef-dll-sys cargo cache invalidation.** If `cargo clean` wipes `target/`, the cef-dll-sys cache is gone too — but the resolver's first two candidates are independent of cargo, so the patched build is still found. Acceptable.
- **Risk: developer with broken or stale `~/cef-build/...`.** Resolver picks it first if it exists. If the libcef.so there is a half-built or wrong-branch artifact, agentmux will crash or run with wrong behavior. Mitigation: the size warning catches the most common stale state (un-stripped debug build); content fingerprint is out of scope per D3.
- **Non-goal: automated download of patched libcef** for non-maintainer contributors. They'll get the cef-dll-sys fallback (drag broken). Solving this requires a build server, hosting, signing — separate work.
- **Non-goal: macOS / Windows resolver.** This PR is Linux-only.
- **Non-goal: upstreaming `BeginWindowDrag`.** Real long-term cure but out of scope. Track separately.

---

## File-by-file change summary

**New:**
- `scripts/resolve-cef-runtime.sh` (~50 lines, executable).
- `docs/cef-build/build-patched-libcef.md` (consolidate `cef_build_in_progress.md` memory; ~80 lines).

**Edited:**
- `Taskfile.yml` — `bundle:linux` task body, replace inline find with `bash scripts/resolve-cef-runtime.sh`.

**Untouched:**
- agentmux-cef Rust source (the runtime ABI guard already handles the mismatch case).
- frontend / TS / SCSS.
- macOS / Windows packaging.
- `scripts/build-appimage-linux.sh` (uses `dist/cef/` post-bundle; correct as long as bundle is fixed).
