# Retro: CEF Build Extraction Race Condition (2026-04-24)

## What happened

`task package` regressed mid-day on 2026-04-24. v0.33.372 built successfully
at ~03:41; every attempt from ~05:43 onward failed with:

```
Error: File I/O error: Access is denied. (os error 5)
...
CMake Error at CMakeLists.txt:221 (add_subdirectory):
  add_subdirectory given source
  "C:/Systems/agentmux/target/release/build/cef-dll-sys-*/out/cef_windows_x86_64/libcef_dll"
  which is not an existing directory.
```

The CMake error is a symptom. The underlying failure is earlier: the
`download-cef` crate's extraction step leaves `cef_windows_x86_64/`
**partially populated** — runtime DLLs + `CMakeLists.txt` + `cmake/` are
present, but `include/`, `libcef_dll/`, and the `Resources/` subset did
not get moved into it. CMake then fails to find `libcef_dll`.

## Root cause

`download-cef 2.3.1` (pinned in `Cargo.lock` — unchanged since last
successful build) finishes its extraction with a series of
`fs::rename` calls that move subdirectories from the raw extract
(`cef_binary_*/`) into the CMake-expected layout
(`cef_windows_x86_64/`):

```rust
// download-cef-2.3.1/src/lib.rs line ~520 onward
fs::rename(extracted_dir.join(RELEASE_DIR), &cef_dir)?;     // runtime DLLs
for entry in fs::read_dir(&resources)? {                     // Resources/*
    fs::rename(entry.path(), cef_dir.join(entry.file_name()))?;
}
fs::rename(extracted_dir.join(CMAKE_LISTS_TXT), ...)?;      // CMakeLists.txt
fs::rename(extracted_dir.join(CMAKE_DIR), ...)?;            // cmake/
fs::rename(extracted_dir.join(INCLUDE_DIR), ...)?;          // include/   ← fails here
fs::rename(extracted_dir.join(LIBCEF_DLL_DIR), ...)?;       // libcef_dll/
```

On Windows, `fs::rename` calls `MoveFileExW`. Once `Release/` is moved
into `cef_windows_x86_64/`, that directory now contains `libcef.dll`
(~100 MB), `chrome_elf.dll`, `v8_context_snapshot.bin`, and other large
Chromium binaries. Windows Defender's real-time protection starts
scanning those files. While Defender holds open handles to the
contents (which, on Windows, also holds a shared reference to the
parent directory), subsequent `MoveFileExW` calls that would place
new entries into the same parent directory can return `ERROR_ACCESS_DENIED`.

The `cmake/` rename happens to land before Defender's scan reaches
steady state. The `include/` rename (hundreds of `.h` files) lands
during peak scan activity — and fails.

This is a **race**: identical code, identical inputs, different
outcomes based on scan timing. v0.33.372 won the race; v0.33.373+
lost it.

## Evidence

Post-failure state of the build directory:

```
target/release/build/cef-dll-sys-*/out/
├── cef_binary_146.0.12+.../    ← still intact (source)
│   ├── include/                ← never moved
│   ├── libcef_dll/             ← never moved
│   ├── Resources/              ← empty (contents were moved successfully)
│   └── (other bazel/build files)
├── cef_binary_*.tar.bz2        ← 161 MB, successfully downloaded
└── cef_windows_x86_64/         ← partial destination
    ├── CMakeLists.txt          ✓ moved
    ├── cmake/                  ✓ moved
    ├── libcef.dll              ✓ moved (from Release/)
    ├── chrome_100_percent.pak  ✓ moved (from Resources/ loop)
    ├── (other runtime files)   ✓
    ├── include/                ✗ MISSING — rename failed here
    └── libcef_dll/             ✗ MISSING — never reached
```

The partial state — Release moved, Resources moved, CMakeLists.txt
moved, cmake moved, include missing — is diagnostic: the failure is
after cmake and before include.

## Why this wasn't caught earlier

- v0.33.372 built successfully at 03:41 under the same toolchain.
  Previous builds in `VERSION_HISTORY.md` succeeded too. The race
  silently won. No one knew the extraction was fragile.
- `cargo clean --release` doesn't help: a fresh extract hits the
  same race window.
- The "Access is denied" error surfaces deep inside the `download-cef`
  build script, with no actionable hint about the extraction step
  that failed.

## Mitigation options (not yet applied)

**Option A (zero-friction, user-local) — Defender exclusion**

Add `C:\Systems\agentmux\target` to Windows Defender's exclusion list.
Stops the scan-during-rename race entirely.

```powershell
Add-MpPreference -ExclusionPath "C:\Systems\agentmux\target"
```

Cost: per-machine config, not reproducible for other contributors.

**Option B (upstream fix) — patch `download-cef`**

Wrap each `fs::rename` in a retry loop that backs off on
`ERROR_ACCESS_DENIED`. Upstream the fix to
https://github.com/chromiumembedded/rust-cef-dl (or whatever the
download-cef repo is — need to confirm).

Cost: upstream review cycle; pinning a fork until merged.

**Option C (in-tree workaround) — post-extraction repair**

Add a step to `Taskfile.yml` (or a `build.rs` in our host crate) that,
after `cargo build` completes cef-dll-sys's build script, verifies
`cef_windows_x86_64/{include,libcef_dll,Resources}` exist and, if
missing, copies them from `cef_binary_*/`. Re-runs CMake if repair was
needed.

Cost: ~20 lines of Rust or shell. Doesn't fix the extraction-time
race but makes it non-fatal. Most portable across developer machines.

**Recommendation:** Option C for the repo (doesn't require per-machine
config), Option A for this dev machine as an extra safety net.

## Action items

- [ ] Add Defender exclusion on this dev machine (user runs
  `Add-MpPreference` one-liner).
- [ ] Open a PR that adds Option C in the in-tree repair step.
  Suggested location: `Taskfile.yml` `build:backend` task, post-step
  shell hook that scans `target/release/build/cef-dll-sys-*/out/`.
- [ ] File upstream issue on `download-cef` repo referencing this
  retro.

## Open questions

1. Does the race also exist on Linux? (unlikely — no Defender; but
   other FS watchers may do similar things)
2. Has anyone patched this downstream? Worth checking
   `servo/cef` / `dev-cef-rs` forks before writing Option C.
3. Is there a `CEF_CACHE_DIR` env var we could set to a Defender-
   excluded path to avoid the issue at source?

## Timeline (2026-04-24, this dev machine)

| Time | Event |
|------|-------|
| 03:41 | v0.33.372 built successfully. Race won. |
| 05:43 | First `task package` attempt after SCSS merges. Race lost. Partial `cef_windows_x86_64/`. |
| 05:44 | `cargo clean --release` + retry. Same failure. |
| 05:47 | `rm -rf target/ dist/` + retry. Same failure. |
| 05:50 | Identified partial-extraction symptom (missing `libcef_dll/` etc.). |
| 06:00 | Traced to `download-cef-2.3.1/src/lib.rs` line 520 area. Confirmed `fs::rename` as failure locus. |
| 06:10 | Retro written. |
