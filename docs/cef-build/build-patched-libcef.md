# Building the Patched libcef.so for AgentMux

**Audience:** AgentMux maintainers building a Linux release binary that needs left-click window drag (and, eventually, transparency).
**Time:** First build ~3-6 hours wall-clock on a 32-core box (CPU-bound chromium compile).
**Disk:** ~99 GB chromium working tree + ~8 GB build output.
**Output:** A `libcef.so` (~613 MB stripped) with two AgentMux-specific patches that aren't in upstream CEF or its prebuilt binary distribution.

---

## What this libcef contains that upstream's doesn't

1. **`CefWindow::BeginWindowDrag()`** — appended to the `_cef_window_t` C struct, called via raw FFI by `agentmux-cef/src/ui_tasks.rs::StartWindowDragTask` to dispatch `xdg_toplevel.move` (Wayland) / `_NET_WM_MOVERESIZE` (X11/XWayland) on the user's left-click drag of the title-bar area.
2. **Right-click passthrough on HTCAPTION** — lets right-clicks on the drag region reach the renderer (for the pane-header context menu).
3. **Transparency broadening** — deferred `SetBackgroundOpaque(false)` + WebContents transparency cascade. Foundation for Wayland window transparency. Root cause identified (views::SolidBackground / kColorPrimaryBackground); fix in progress.

Patches live in the AgentMux fork of CEF:
- **Repo:** https://github.com/agentmuxai/cef (canonical) / https://github.com/a5af/cef (personal fork, kept in sync)
- **Branch:** `agentmux/7778-drag-rightclick-and-transparency`
- **Base:** Chromium 148 (CEF branch 7778)
- **HEAD:** `c87bca497` ("views: deferred top-level transparent bg + observer cleanup")
- **Rust binding:** `AgentU-asaf/cef-rs@agentmux/148-begin-window-drag` — adds `begin_window_drag` field to `_cef_window_t` in the linux_x86_64 binding
- **Workspace patch in `Cargo.toml`:** `[patch.crates-io] cef-dll-sys = { git = "…AgentU-asaf/cef-rs", rev = "515b3ac5…" }`

> **Annotation history:** `BeginWindowDrag` was annotated `added=14600` on the old CEF 146 branch, then briefly changed to `added=NEXT` during the 148 port (this caused a CppToC type-tag mismatch making all drags silently no-op), then corrected to `added=14800`. The current branch has the correct `added=14800` annotation.

---

## Prerequisites

- Linux x86-64 host (Ubuntu 22.04+ tested).
- ≥ 32 GB RAM. Build peak hits ~25 GB at `-j 12 -l 16`. Lower job counts work but slower.
- ≥ 120 GB free disk.
- `python3`, `git`, `clang`, GCC. The chromium `build/install-build-deps.sh` will list anything missing.
- `systemd-run` available (default on systemd distros).

---

## Build steps

### 1. Initial setup

```bash
mkdir -p ~/cef-build
cd ~/cef-build

# Get the chromium depot_tools
git clone https://chromium.googlesource.com/chromium/tools/depot_tools.git
export PATH="$PWD/depot_tools:$PATH"

# Get the CEF automate-git script
mkdir -p chromium_git
cd chromium_git
wget https://bitbucket.org/chromiumembedded/cef/raw/master/tools/automate/automate-git.py

# First sync (downloads chromium ~99 GB, takes hours)
python3 automate-git.py \
  --download-dir=$(pwd) \
  --branch=7778 \
  --no-distrib \
  --no-build
```

### 2. Switch to the AgentMux fork

```bash
cd ~/cef-build/chromium_git/cef
git remote add agentmuxai https://github.com/agentmuxai/cef.git
git fetch agentmuxai agentmux/7778-drag-rightclick-and-transparency
git checkout agentmuxai/agentmux/7778-drag-rightclick-and-transparency

# Mirror to the chromium-side cef checkout
rsync -a --delete --exclude=.git ~/cef-build/chromium_git/cef/ ~/cef-build/chromium_git/chromium/src/cef/
```

### 3. Apply CEF patches to chromium

```bash
cd ~/cef-build/chromium_git/chromium/src
# Reset to clean state — patcher is NOT idempotent; double-apply produces .rej files
git reset --hard HEAD
find . -name '*.rej' -delete
# Now apply
python3 cef/tools/patcher.py --root-dir=cef
```

If you see "class member cannot be redeclared" compile errors later, the patcher likely double-applied — repeat the reset and re-run.

### 4. Configure the build

The canonical GN args are version-controlled at **`scripts/cef-build/args.gn`** in
the agentmux repo — the exact configuration that built the shipped v0.45.0 libcef.
Use the configure script: it regenerates the gitignored C-API wrappers (gotcha #1
below), installs those args into the build tree, and runs `gn gen`:

```bash
# Run from your agentmux repo checkout. Defaults to ~/cef-build/chromium_git/chromium/src;
# set AGENTMUX_CEF_SRC=/path/to/chromium/src if your tree is elsewhere.
bash scripts/cef-build/configure-cef-build.sh
```

Equivalent manual steps, if you'd rather drive `gn` yourself:

```bash
cd ~/cef-build/chromium_git/chromium/src
( cd cef && python3 tools/translator.py --root-dir . )           # regen wrappers FIRST
cp /path/to/agentmux/scripts/cef-build/args.gn out/Release_GN_x64/args.gn
./buildtools/linux64/gn gen out/Release_GN_x64
```

> **`is_official_build=true` is the size lever — do not omit it.** It enables
> thin-LTO + identical-code-folding + full optimization. Without it (a plain
> `is_debug=false` Release), `libcef.so` is **~2x** the official size:
> ~414 MB stripped vs **~263 MB** with it — and the Linux AppImage ~190 vs
> **~135 MB**. `is_cfi=false` and `chrome_pgo_phase=0` are disabled only to skip
> their data/setup deps (they don't drive size). `symbol_level=1` keeps the
> unstripped `.so` large (~1.5 GB of debug info) but `strip --strip-all` removes
> all of it; ship the stripped binary. Verified 2026-06-14 on CEF 148.0.7778.180.
>
> Two gotchas after changing args on an existing tree:
> 1. **Regenerate the gitignored wrappers first** or the build dies instantly on
>    `cef/libcef_dll/ctocpp/views/window_ctocpp.cc` "missing and no known rule":
>    `cd cef && python3 tools/translator.py --root-dir .` (the tree stays clean —
>    identical API rewrite — so no `version_manager.py` needed).
> 2. **Copy the new `snapshot_blob.bin` + `v8_context_snapshot.bin`** alongside
>    `libcef.so` — `is_official_build` rebuilds V8, and stale snapshots crash the
>    host on a checksum mismatch.

### 5. Build (use the OOM-resistant wrapper)

The default `ninja -j$(nproc)` peak-allocates 40+ GiB → OOM-kills the terminal cgroup → can cascade into a system reboot. Use `-j 12 -l 16` AND launch under `systemd-run --user --scope` to isolate the build from your shell:

```bash
# First, set up the wrapper script (one-time)
cat > ~/cef-build/ninja-with-retry.sh <<'EOF'
#!/bin/bash
set -uo pipefail
cd ~/cef-build/chromium_git/chromium/src
for attempt in 1 2 3; do
    if ninja -j 12 -l 16 -C out/Release_GN_x64 cef; then
        echo "==================== NINJA SUCCESS on attempt $attempt at $(date) ===================="
        exit 0
    fi
    echo "==================== NINJA FAILED attempt $attempt — retrying in 60s ===================="
    sleep 60
done
exit 1
EOF
chmod +x ~/cef-build/ninja-with-retry.sh

# Build under an isolated cgroup
systemd-run --user --scope --collect --unit=cef-build.scope \
  ~/cef-build/ninja-with-retry.sh
```

Expect ~3-6 hours on first build with cold ccache. Subsequent rebuilds (modifying just the patches) are 5-30 minutes depending on what touched.

### 6. Strip the output

The default build produces a `libcef.so` ~1.3 GB with full debug info. Strip it for distribution:

```bash
cd ~/cef-build/chromium_git/chromium/src/out/Release_GN_x64
strip --strip-debug libcef.so   # → ~613 MB
```

(The AgentMux AppImage build script does this strip step automatically when assembling the package, but doing it once after the source build keeps the bundling step cheap.)

### 7. Verify it boots

```bash
cd ~/cef-build/chromium_git/cef/tests
out/Release_GN_x64/cefsimple --no-sandbox --url=https://example.com
```

A window with example.com proves the libcef is functional. (The `BeginWindowDrag` patch isn't exercised by cefsimple — verification of THAT happens via `task package:linux` + the AgentMux app.)

To check the patch is present without launching anything:

```bash
bash scripts/verify-cef-patch.sh ~/cef-build/chromium_git/chromium/src/out/Release_GN_x64
# exit 0 = patched · exit 1 = unpatched upstream · exit 2 = stripped (run on the unstripped build)
```

`task bundle:linux` runs this **advisorily** (a warning, so `task dev` still works on the
upstream cef-dll-sys fallback); `scripts/build-appimage-linux.sh` runs it as a **hard
release gate** that refuses to package an AppImage whose libcef.so lacks the patch.
The symbol it keys on (`window_begin_window_drag_<apiver>`) lives in `.symtab` — present
in the unstripped build, gone after `strip` — so the gate must (and does) run **before**
the packaging strip. Override the gate with `AGENTMUX_SKIP_CEF_PATCH_CHECK=1` (emergency only).

---

## Using the built libcef in AgentMux

The AgentMux build pipeline finds your patched libcef automatically if it's in the standard location. Two options:

### Option A: Default location

If you built at `~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/`, you're done — `task bundle:linux` and `task package:linux` will pick it up via the resolver in `scripts/resolve-cef-runtime.sh`.

### Option B: Explicit override

If your build is somewhere else, set the env var:

```bash
export AGENTMUX_CEF_RUNTIME_DIR=/path/to/your/Release_GN_x64
task bundle:linux
```

The resolver prints which path it picked at the start of the bundle step:

```
Using CEF runtime from: /home/you/cef-build/chromium_git/chromium/src/out/Release_GN_x64
```

If you see a `WARNING: libcef.so at ... is 1272 MB (>1 GB) — likely unpatched upstream debug build`, the resolver fell through to the cef-dll-sys cargo cache because no patched build was found at either of the higher-priority locations. The bundle will still produce a runnable AgentMux, but left-click window drag will silently no-op (the runtime ABI guard logs a warning).

---

## Re-building after patch changes

You don't need to re-run `patcher.py` if you're modifying within the already-patched tree. Just edit, mirror, ninja:

```bash
# Edit in the cef checkout (where you actually develop)
cd ~/cef-build/chromium_git/cef
# ... edit ...

# Mirror to the chromium-side
rsync -a --delete --exclude=.git ~/cef-build/chromium_git/cef/ ~/cef-build/chromium_git/chromium/src/cef/

# Re-build (incremental; usually 5-30 min)
systemd-run --user --scope --collect --unit=cef-build.scope \
  ~/cef-build/ninja-with-retry.sh

# Re-strip
strip --strip-debug ~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/libcef.so
```

---

## Known gotchas

- **`automate-git.py --fast-update` does not always trigger `gclient sync`** if the top-level checkout already matches. Run `gclient sync --nohooks --with_branch_heads` directly to force.
- **Don't pass `--reset --delete_unversioned_trees`** to `gclient sync` — it reverts all 114 applied CEF patches.
- **`tools/patcher.py` is NOT idempotent.** Always precede with `git reset --hard HEAD && find . -name '*.rej' -delete`.
- **`.gclient` has `'managed': False`** — gclient is lazy. Dirs in broken half-clone state get silently skipped. Symptom: a missing dep at compile time. Fix: `rm -rf` the broken dep dir and re-sync.

---

## Future: upstreaming `BeginWindowDrag`

The whole `~/cef-build/...` machinery exists to host one (eventually two) patches not in upstream CEF. The long-term cure is to upstream `BeginWindowDrag` to chromiumembedded/cef and bump cef-dll-sys past it; then this whole document becomes obsolete. There's no open upstream PR yet — tracked separately.
