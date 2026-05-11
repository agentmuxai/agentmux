# Linux AppImage cold-launch tax

**Status:** Draft.
**Author:** runtime investigation 2026-05-08.
**Owner:** TBD.
**Affects:** Linux AppImage builds. macOS .app and Windows MSIX likely
unaffected (different distribution + mount semantics).

---

## Problem

Every AppImage launch — including the second / third / nth launch with the
cef-cache fully populated — pays a ~2.4-2.6 second tax between page-load
start and main window interactivity. Total perceived launch time from user
double-click to "AgentMux is usable" is ~3-3.5 seconds. Users coming from
`task dev` (Vite dev server) experience this as a regression because dev
mode is sub-second.

This is not a regression — it's the inherent overhead of how AppImage works
on Linux, and it has always existed for AppImage builds. But it's worth
documenting and worth fixing because the Windows portable ZIP starts in
under a second and we should match that target.

---

## Evidence

Three consecutive launches of the same `AgentMux_0.33.723_amd64.AppImage`
in one session, cef-cache stays populated across launches:

| launch | T+0 = page-load | setupCefApi-done | fonts-ready | mainwin-done |
|--------|----------------|-------------------|--------------|---------------|
| 19:29:24 (first)  |  0ms | 1957ms | 2296ms | 2425ms |
| 19:31:24 (second) |  0ms | 1908ms | 2268ms | 2408ms |
| (tear-off win)    |  0ms | 1207ms | 1606ms | 1746ms |

The cef-cache was 18MB on first launch, grows over the session, but second
launch timing is essentially identical to first. **Caching is not the
dominant variable.** The tear-off-window column is faster because the
launcher process is already paged in and the host process is already
running — only the renderer is cold.

Add ~1 second of pre-page-load CEF initialization (libcef.so dlopen, GPU
process spawn, network process spawn, frontend HTTP server bind) → ~3.4s
total. Matches user-perceived "3+ seconds".

---

## Root cause

AppImage execution model on Linux:

1. Launcher binary self-extracts a SquashFS archive into
   `/tmp/.mount_AgentM*` via `squashfuse` (FUSE).
2. SquashFS files are **decompressed on-demand** on first read, even
   though the FUSE mount has been there since the previous launch — but
   actually no, the mount is destroyed when the prior launch exits, so
   each launch starts with a *fresh* mount.
3. Every read of every file touches:
   - FUSE userspace round-trip → kernel.
   - SquashFS zstd block decompression.
   - Kernel page cache (helps for repeated reads within a single launch).
4. `libcef.so` alone is ~613MB stripped. dlopen + relocations on this
   library account for several hundred ms.
5. The frontend bundle (~5-10MB JS, ~10MB fonts, etc.) lives in the same
   SquashFS and is fetched by the in-process HTTP server on each cold
   start. V8 parses + compiles + runs this bundle.

V8 also has its own per-launch costs:

- No bytecode cache between launches by default. Each launch reparses
  `index-*.js` from source.
- Hot-spot JIT compilation profiles are not preserved.

Disk doesn't help here: the AppImage file itself sits on disk and is
mmaped for the SquashFS read; subsequent launches have the same disk-
sectors-already-cached situation but still pay the FUSE round-trip and
SquashFS decompression on read.

---

## Options considered

### Option A — `--appimage-extract-and-run`

AppImage built-in flag. On launch, extracts the entire SquashFS to a temp
directory, then exec's the binary from there. Subsequent file reads bypass
FUSE and SquashFS — they're plain-disk files in a tmpfs or ext4 dir.

**Pros:** Single-flag change. Eliminates FUSE + SquashFS overhead. Probably
brings cold-launch within 200-400ms of dev-mode parity.

**Cons:** Doubles disk usage (AppImage + extracted dir). `/tmp` may be
tmpfs (RAM-backed) on some distros — extraction allocates ~200MB of RAM
per launch, recycled at exit. Extraction itself is a one-time cost per
launch (~500ms-1s) → first interactive may not actually improve unless
extraction is amortized across launches. Negates AppImage's "one file"
distribution simplicity.

### Option B — Pre-extract to `~/.local/share/agentmux/extracted/<version>/`

On first launch, the AppImage extracts itself to a per-version folder
under the user's home. Subsequent launches detect the extracted folder
and exec from there directly. The AppImage file becomes a one-time
installer.

**Pros:** Eliminates per-launch extraction cost (only first launch pays
it). Preserves the "one file you can put anywhere" distribution model.
Works inside `flatpak`-style sandboxes if extraction target is in user
data dir.

**Cons:** Custom logic to write. Need version-aware cleanup so old
extracted versions don't leak. First-launch experience is slow then
self-improving — confusing for evaluators. Solo-user assumption: in
multi-user installs the extraction directory must be per-user.

### Option C — `.deb` package distribution

Stop shipping AppImage on Linux. Distribute via `.deb` (Ubuntu/Debian)
and `.rpm` (Fedora/RHEL). Files install directly to `/opt/agentmux/`,
mmaped from disk on each launch with kernel page cache fully effective.

**Pros:** Zero runtime overhead vs. native disk binaries. Same cold-launch
profile as Windows portable ZIP. Easier integration with system desktop
files / icons / mime types.

**Cons:** Loses AppImage's distro-agnostic single-file model. Need to
maintain separate `.deb` and `.rpm` build pipelines. AppImage is currently
the only Linux distribution channel — moving away from it is a strategic
call, not a perf fix.

### Option D — V8 code-cache persistence

Use `--js-flags=--code-cache=...` to write V8 bytecode to disk on first
parse and reuse on subsequent launches.

**Pros:** Cuts ~200-400ms off the parse/compile phase of subsequent
launches. Stays orthogonal to FUSE/SquashFS issues.

**Cons:** Only addresses the V8 phase, not the FUSE phase. Marginal on
its own. CEF has limited support for V8 code caching across browser
processes; needs investigation.

### Option E — Frontend bundle code-splitting + lazy load

Trim the synchronously-loaded JS at startup; defer non-critical code
(modals, settings, devtools panes) to dynamic imports.

**Pros:** Reduces parse/compile work in critical path. Pure frontend
change, no infra touch.

**Cons:** Significant refactor. Each split point needs UX consideration
(loading states, error boundaries). Doesn't help the FUSE I/O cost
because dynamic imports still go through SquashFS — they just defer the
cost from main-thread blocking to interaction-time.

### Option F — `usrmerge` portable directory

Ship a portable `.tar.gz` instead of AppImage. User extracts once,
launches the binary from the extracted folder. Same model as Windows
portable ZIP today.

**Pros:** Trivial: extract once, run forever. No FUSE, no overlay. Same
performance as `.deb` for non-installer use cases.

**Cons:** Loses AppImage's "double-click to run" UX. Power-user only.
Could ship alongside AppImage as a "fast launch" option.

---

## Recommendation

**Phase 1 (immediate):** Try `--appimage-extract-and-run` (Option A) as a
build-time default and measure end-to-end cold-launch time. If it brings
launch under ~1.5s, ship it as-is.

**Phase 2 (if Phase 1 is insufficient):** Implement Option B (pre-extract
on first launch, exec from cached extraction on subsequent launches).
This is the AppImage equivalent of "install once, launch forever".

**Phase 3 (longer term):** Stack Option D (V8 code cache) on top of
whatever Phase 1/2 delivers. Diminishing returns but cumulative.

**Defer:** Options C (deb/rpm), E (code-split), F (portable tarball).
These are larger projects with their own design questions; revisit if
Phases 1-3 still leave us above target.

### Why not just go to Phase 2 directly?

Phase 1 is a one-line build change. If `--appimage-extract-and-run`
already gets us where we want (sub-1.5s warm launch), Phase 2's complexity
is unjustified. Measure first.

---

## Implementation outline (Phase 1 spike)

`scripts/build-appimage-linux.sh` invokes `appimagetool` to produce the
`AgentMux_*.AppImage` artifact. The artifact's first arg parsing is
controlled by the runtime ELF embedded by `appimagetool` (the
`type2-runtime`).

`type2-runtime` already supports `--appimage-extract-and-run` as a CLI
flag — but we want it to be the *default*, with no user flag required.
Two options:

1. **Patch the runtime:** rebuild `type2-runtime` with the default flag
   set to "extract first". Heavy — would need a maintained fork.

2. **Wrap the AppImage:** ship a tiny shell script wrapper that invokes
   the AppImage with `--appimage-extract-and-run` baked in. Two-file
   distribution (the .AppImage + the .sh) breaks AppImage's
   one-file-distribution premise.

3. **Generate a wrapper at the user's `agentmux.desktop` Exec= line:**
   the desktop file we install via `scripts/install-linux-desktop.sh`
   already does `Exec=…AppImage`. Change to
   `Exec=…AppImage --appimage-extract-and-run`. Affects launch from
   GNOME Shell / KDE / shortcuts, but NOT from `./AgentMux_*.AppImage`
   directly in a terminal. Gradient migration: developers / CI still
   get fast paths via the desktop file; raw `.AppImage` execution stays
   "classic" for compatibility.

Recommendation: **(3)** as the spike. If it benchmarks well, then revisit
(1) for true universal default.

### Bench harness

The host already emits `[startup-bench]` events with millisecond offsets
relative to page load. To measure the full process-spawn → mainwin-done
window, also need:

- Timestamp at AppImage launcher exec (write to env or file before exec).
- Timestamp at host process `main()` entry.
- Existing `[startup-bench]` events from page-load onward.

A simple addition: the launcher already logs `[launcher]` events; add a
"launched at" timestamp + emit it via stderr early. Compare wall-clock
launch → mainwin-done before / after `--appimage-extract-and-run`.

---

## Validation plan

### Cold launch (worst case)

1. Reboot or `echo 3 > /proc/sys/vm/drop_caches`. Empty page cache.
2. Double-click `AgentMux_0.33.X_amd64.AppImage` from a file manager that
   uses the desktop file's `Exec=` line.
3. Stopwatch from click → main window first paint visible.
4. Compare to current 0.33.723 baseline (~3.5s) and target (~1.5s).

### Warm launch

1. Quit app.
2. Relaunch within 30s (page cache still warm).
3. Stopwatch.
4. Compare to current 0.33.723 baseline (~3.0s) and target (~1.0s).

### Regressions to watch

- **Disk usage:** `--appimage-extract-and-run` stages content in `/tmp`.
  If `/tmp` is tmpfs and small (<2GB), extraction may OOM. Guard: if free
  `/tmp` < 500MB, fall back to direct AppImage invocation.
- **Permissions:** extraction creates files owned by the launching user.
  Multi-user shared install scenarios — out of scope for AppImage anyway.
- **Update flow:** when a new AppImage is dropped, the per-version
  cached extraction directory becomes stale and should auto-clean. For
  Phase 1 (always-extract-fresh) this is moot. For Phase 2 (cached
  extraction) this is a real concern — design `~/.local/share/agentmux/
  extracted/<version>/` so old `<version>` dirs get reaped on next launch
  by a small cleanup pass.

### Metrics

Track in CI:

- `cold_launch_ms` (post-`drop_caches`, time-to-first-paint).
- `warm_launch_ms` (immediate relaunch).
- `extraction_overhead_ms` (Phase 1 only — extra cost per launch).

Goals after Phase 1:

- `cold_launch_ms` ≤ 2000.
- `warm_launch_ms` ≤ 1500.

After Phase 2:

- `cold_launch_ms` ≤ 1500.
- `warm_launch_ms` ≤ 800.

---

## Risks / open questions

- **`type2-runtime` flag interaction:** does `--appimage-extract-and-run`
  pass the rest of `argv` through cleanly? Verify by running `AgentMux*.
  AppImage --appimage-extract-and-run --version` and checking that
  `--version` is consumed by the host, not the runtime.
- **`/tmp` extraction collisions:** two parallel users / two parallel
  AppImages could collide on `/tmp/.mount_AgentMux-*` (FUSE) or
  `/tmp/appimage_extracted_*` (extract mode). Verify the runtime adds a
  unique suffix per launch.
- **CEF cache directory unrelated:** the `~/.agentmux/versions/*/cef-cache`
  directory is independent of how the binary is launched. Both Phase 1
  and Phase 2 leave it alone.
- **AppImage updater compatibility:** if AppImage zsync auto-update lands,
  is the extracted-and-cached version Phase 2 still updateable? Probably
  yes — the AppImage file itself updates, the extraction directory just
  rebuilds on next launch.

---

## Out of scope (referenced for completeness)

- **First-time user perception:** even a 1.5s cold launch may feel slow
  versus modern web apps. UX-side mitigations (launch splash, partial-
  paint of the chrome before bundle is ready) belong in a separate spec.
- **macOS .app cold launch:** macOS uses dyld-shared-cache pre-warming
  for system frameworks; the per-app cold-launch overhead is structurally
  different. Tracked separately.
- **Windows portable cold launch:** Windows portable ZIP launches in
  ~700ms today (per anecdotal report). Windows installer (MSIX) is
  out-of-tree; not relevant.

---

## See also

- `scripts/build-appimage-linux.sh` — current AppImage build pipeline.
- `scripts/install-linux-desktop.sh` — current desktop file install.
- `docs/specs/linux-pool-startup-fill-2026-05-08.md` — sister spec for
  Linux tear-off latency. **Note: pool startup fill solves *tear-off*
  cold-path; this spec solves *app launch* cold-path. Both contribute to
  user-perceived "Linux feels slow."**
