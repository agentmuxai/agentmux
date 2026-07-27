# Building the Codec-Enabled CEF for AgentMux (Windows x64)

**Audience:** AgentMux maintainers building a Windows CEF binary with
proprietary codec support (H.264/AAC/HEVC/AC3/EAC3/Dolby Vision) —
see `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md` for why,
`docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md` for
the full 3-platform plan this is one piece of.
**Time:** First build ~3-6 hours wall-clock (CPU-bound chromium compile) —
same order of magnitude as the Linux/macOS builds; not yet verified with
a real run on this exact machine (see "Status" below).
**Disk:** ~99 GB chromium working tree + build output. **Confirm you have
this before starting** — `Get-PSDrive -PSProvider FileSystem`.
**Output:** A `libcef.dll` + runtime files with proprietary codec support.
Unlike the Linux/macOS builds, this is **not** carrying the
`BeginWindowDrag`/transparency C++ patches for any functional reason —
Windows never needed them (native window drag already works via Win32).
It's built from the same fork branch anyway for one-canonical-source-branch
consistency across all three platforms (see the linked spec's open
question #1) — the patch is present but inert on Windows.

**Status (2026-07-26): this doc's Windows-specific steps are a
first-pass draft**, written by combining the proven Linux process
(`build-patched-libcef.md`) with official Chromium Windows build docs —
**not yet validated end-to-end with a real build on this machine**. Update
this doc (and `scripts/cef-build/args-windows.gn`'s header comment) with
real numbers/corrections once a build actually completes.

---

## Prerequisites

- Windows 10/11 x64 host.
- ≥ 32 GB RAM (peak usage during Linux's equivalent build hits ~25 GB at
  `-j 12 -l 16`; scale job count to your core count — see step 5).
- ≥ 120 GB free disk.
- **Visual Studio 2022 with the "Desktop development with C++" workload**,
  including the Windows 11 SDK. The Build Tools SKU (no full IDE) is
  sufficient — this is what `C:\Program Files (x86)\Microsoft Visual
  Studio\2022\BuildTools` already provides on this machine.
- `python3`, `git` — already present on this machine.
- **`depot_tools`** — Chromium's own build tooling (gclient, gn, ninja
  wrappers). Already present on this machine at `C:\depot_tools`. If
  setting up fresh: `git clone
  https://chromium.googlesource.com/chromium/tools/depot_tools.git`, then
  put it **first** on PATH (it bundles a pinned Python that must win over
  any other Python install).
- **The single most common external-contributor failure point on
  Windows:** depot_tools defaults to trying to fetch a Google-internal
  toolchain package. Set this before anything else:
  ```powershell
  $env:DEPOT_TOOLS_WIN_TOOLCHAIN = "0"
  ```
  Without it, `gclient sync`/`gn gen` fail trying to authenticate against
  an internal Google package server external contributors can't reach.
  If GN still can't locate Visual Studio after setting this, also set
  `$env:vs2022_install` to the exact install path (e.g. `"C:\Program
  Files (x86)\Microsoft Visual Studio\2022\BuildTools"` on this machine).
- **Long path support.** Chromium's source tree nests deeply enough to
  exceed the classic 260-character `MAX_PATH` limit. Enable both:
  ```powershell
  git config --global core.longpaths true
  # Also needs Windows' own long-path support enabled (one-time, needs an
  # admin PowerShell + restart to take effect):
  # New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
  #   -Name "LongPathsEnabled" -Value 1 -PropertyType DWORD -Force
  ```

---

## Build steps

### 1. Initial setup

```powershell
$env:DEPOT_TOOLS_WIN_TOOLCHAIN = "0"
$env:PATH = "C:\depot_tools;$env:PATH"

New-Item -ItemType Directory -Force -Path "$HOME\cef-build" | Out-Null
Set-Location "$HOME\cef-build"

New-Item -ItemType Directory -Force -Path "chromium_git" | Out-Null
Set-Location "chromium_git"
Invoke-WebRequest -Uri "https://bitbucket.org/chromiumembedded/cef/raw/master/tools/automate/automate-git.py" -OutFile "automate-git.py"

# First sync (downloads chromium ~99 GB, takes hours)
python3 automate-git.py `
  --download-dir="$(Get-Location)" `
  --branch=7778 `
  --no-distrib `
  --no-build
```

### 2. Switch to the AgentMux fork

Same fork/branch as Linux/macOS — see the header note above on why
Windows uses this branch despite not needing its patches.

```powershell
Set-Location "$HOME\cef-build\chromium_git\cef"
git remote add agentmuxai https://github.com/agentmuxai/cef.git
git fetch agentmuxai agentmux/7778-drag-rightclick-and-transparency
git checkout agentmuxai/agentmux/7778-drag-rightclick-and-transparency

# Mirror to the chromium-side cef checkout
robocopy "$HOME\cef-build\chromium_git\cef" "$HOME\cef-build\chromium_git\chromium\src\cef" /MIR /XD .git
```

(`robocopy /MIR` is the Windows equivalent of `rsync -a --delete` used in
the Linux doc — mirrors the tree, deletes anything not in the source,
excludes `.git`. Note `robocopy` exit codes 0-7 are all "success" —
treat anything ≥8 as a real error if scripting this.)

### 3. Apply CEF patches to chromium

```powershell
Set-Location "$HOME\cef-build\chromium_git\chromium\src"
# Reset to clean state -- patcher is NOT idempotent; double-apply produces .rej files
git reset --hard HEAD
Get-ChildItem -Recurse -Filter "*.rej" | Remove-Item
python3 cef\tools\patcher.py --root-dir=cef
```

If you see "class member cannot be redeclared" compile errors later, the
patcher likely double-applied — repeat the reset and re-run (same gotcha
as Linux).

### 4. Configure the build

The canonical GN args are version-controlled at
**`scripts/cef-build/args-windows.gn`**. Use the configure script:

```powershell
# Run from your agentmux repo checkout. Defaults to
# ~\cef-build\chromium_git\chromium\src; set $env:AGENTMUX_CEF_SRC if
# your tree is elsewhere.
pwsh scripts\cef-build\configure-cef-build-windows.ps1
```

See `scripts/cef-build/args-windows.gn`'s own header comment for the
codec-flag rationale (shared with Linux/macOS) and the
official-build/size-lever note (same `is_official_build=true` mechanism
as Linux — expect a similar roughly-halving effect on `libcef.dll` size,
unverified with real numbers until this build completes).

### 5. Build

No `systemd-run`-equivalent isolation mechanism exists on Windows (that
Linux step exists specifically to protect against OOM-killing the whole
shell/session under cgroups) — this machine has ~62 GB RAM / 32 logical
processors, comfortably above the ~25 GB peak the Linux build's `-j 12 -l
16` config hits, so a proportionally scaled job count should be safe
without needing an isolation wrapper. Start conservative and watch memory
pressure on the first run:

```powershell
Set-Location "$HOME\cef-build\chromium_git\chromium\src"
ninja -j 20 -l 28 -C out\Release_GN_x64 cef
```

Expect ~3-6 hours on first build with a cold build cache. If ninja OOMs
or the machine becomes unresponsive, lower `-j`/`-l` and retry — same
tuning tradeoff as the Linux doc describes, just without the cgroup
safety net, so watch Task Manager during the first run rather than
walking away entirely on an unverified job count.

### 6. Verify it boots

```powershell
Set-Location "$HOME\cef-build\chromium_git\cef\tests"
out\Release_GN_x64\cefsimple.exe --url=https://example.com
```

A window with example.com proves the libcef.dll is functional.

### 7. Package + upload as a GitHub release (for CI)

Local AgentMux builds will resolve the CEF runtime from
`~\cef-build\...\Release_GN_x64` directly once
`scripts/resolve-cef-runtime-windows.ps1` and the `Taskfile.yml` wiring
land (see the linked spec §2-3) — this step is for **CI**, which has no
build tree and pulls from a release in `agentmuxai/cef`, exactly like
Linux/macOS already do.

```powershell
$CefOut = "$HOME\cef-build\chromium_git\chromium\src\out\Release_GN_x64"
# CEF API version from cef_version.h's CEF_VERSION (e.g. 148.0.20), NOT
# the chromium build number -- same convention as the Linux/macOS tags.
$CefVersion = "148.0.20"  # confirm against the actual built cef_version.h

Set-Location $CefOut
Compress-Archive -Path @(
    "libcef.dll", "chrome_elf.dll", "libEGL.dll", "libGLESv2.dll",
    "vk_swiftshader.dll", "vk_swiftshader_icd.json", "vulkan-1.dll",
    "d3dcompiler_47.dll", "dxcompiler.dll", "dxil.dll",
    "icudtl.dat", "v8_context_snapshot.bin",
    "chrome_100_percent.pak", "chrome_200_percent.pak", "resources.pak",
    "locales"
) -DestinationPath "cef-windows-x86_64-$CefVersion.zip"

gh release create "cef-windows-x86_64-$CefVersion" --repo agentmuxai/cef `
  --title "Codec-enabled CEF -- Windows x86_64 CEF $CefVersion" `
  --notes "proprietary_codecs + HEVC/AC3/EAC3/Dolby Vision. Branch: agentmux/7778-drag-rightclick-and-transparency (inert on Windows -- built for codec flags only)." `
  "cef-windows-x86_64-$CefVersion.zip"
```

Note the `.zip` here vs. Linux/macOS's `.tar.gz` — `build-windows.yml`
(the new CI workflow, see the linked spec §3) extracts whichever format
matches; keep this consistent with whatever that workflow actually
expects when it's implemented.

---

## Using the built CEF in AgentMux

Once `scripts/resolve-cef-runtime-windows.ps1` and the `Taskfile.yml`
Windows-task wiring exist (spec §2-3, not yet landed as of this doc's
writing):

### Option A: Default location

Build at `~\cef-build\chromium_git\chromium\src\out\Release_GN_x64\` and
`task bundle:windows` / `task package` will pick it up automatically.

### Option B: Explicit override

```powershell
$env:AGENTMUX_CEF_RUNTIME_DIR_WINDOWS = "C:\path\to\your\Release_GN_x64"
task bundle:windows
```

---

## Known gotchas (Windows-specific, beyond the Linux doc's list)

- **`DEPOT_TOOLS_WIN_TOOLCHAIN=0` must be set in every new shell** that
  runs `gclient`/`gn` — it's not persisted anywhere by these steps.
  Consider setting it as a permanent user environment variable if you'll
  be iterating on this build repeatedly.
- **Long paths** — if `gclient sync` fails with path-length errors deep
  in `third_party/`, this is almost certainly the cause; see
  Prerequisites above.
- **depot_tools auto-update** can silently change tool versions between
  sessions; if a previously-working command starts failing for no
  apparent reason, check `depot_tools`' own changelog / consider pinning.
- Everything else (patcher non-idempotency, `.gclient`'s `managed: False`
  laziness, `--reset --delete_unversioned_trees` reverting all CEF
  patches) applies identically to the Linux doc's "Known gotchas" —
  not repeated here, see that doc.

---

## Future

This doc's Windows-specific steps need validation against a real build
before being treated as equally trustworthy to the Linux/macOS docs —
update the "Status" note at the top once that's happened, with real
timing/size numbers replacing the estimates above.
