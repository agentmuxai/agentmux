# Retro: CEF extraction corruption recurred twice in one session; repair script's own gap; a new suspect

**Date:** 2026-08-05
**Severity:** Low (self-heals with a manual `rm -rf` + rebuild; no data loss) but recurring and time-costly
**Related:** `docs/retro/RETRO_CEF_BUILD_RACE_2026_04_24.md` (original root cause: `download-cef`'s `fs::rename` chain races Windows Defender's real-time scan during extraction)

## What happened

Ran `task dev` from a fresh checkout twice in one session (once after merging a frontend-only PR, once again minutes later after pulling that merge back in). Both times, the Rust/CEF host build failed identically:

```
CMake Error at libcef_dll/CMakeLists.txt:818 (add_library):
  Cannot find source file: ../include/cef_id_mappers.h
CMake Error at libcef_dll/CMakeLists.txt:818 (add_library):
  No SOURCES given to target: libcef_dll_wrapper
```

Both times, the Taskfile's own built-in fallback (`cargo build ... || { bash scripts/repair-cef-extract.sh && cargo build ...; }`) ran automatically and did **not** fix it — the retry hit the exact same error. Both times, manually deleting `target/release/build/cef-dll-sys-*/out/cef_windows_x86_64/` (keeping the already-downloaded `.tar.bz2` archive) and rebuilding fixed it cleanly.

## Root cause, confirmed

Same underlying class of bug as `RETRO_CEF_BUILD_RACE_2026_04_24.md`: `download-cef`'s extraction moves `include/` into place and something concurrent interferes. But the *shape* of the corruption is different, and that difference matters:

- The original 2026-04-24 retro's corruption pattern was a **whole subdirectory missing entirely** (`include/` never existed at the destination — the `fs::rename` never happened at all).
- This session's corruption was a **partially populated `include/`** — the directory existed, contained several real files (`base/`, `capi/`, `cef_accessibility_handler.h`, `cef_api_hash.h`, `cef_api_versions.h`, …), but was missing exactly one file (`cef_id_mappers.h`).

That distinction is why the auto-repair didn't fire: `repair-cef-extract.sh`'s condition for restoring a subdirectory is `[ ! -e "$dest" ] || { [ -d "$dest" ] && [ -z "$(ls -A "$dest")" ]; }` — **missing, or fully empty.** A directory that's present and non-empty but short exactly one file reads as "already fine" to that check and the script silently no-ops. Confirmed by reading the script directly: nothing in it compares file *counts* or *contents* between the source (`cef_binary_*/`) and destination — only presence/emptiness.

## Is this related to AgentMux's own long-running processes?

Plausibly yes, and this is the new finding this session. `task dev` runs two long-running processes concurrently against the same working tree: the Vite dev server (file-watching, for HMR) and the Cargo/CEF build (writing hundreds of thousands of files into `target/`, including the CEF extraction). Checked `vite.config.ts`:

```ts
server: {
    watch: {
        ignored: ["dist/**", "**/*.md", "**/*.json"],
    },
},
```

`root: "."` (the whole repo) with no exclusion for `target/**`. Vite's watcher is very likely walking/statting/opening every file created under `target/` during a build, including the rapid burst of file creation during CEF extraction — a second plausible actor racing the same `fs::rename`/copy operations the original retro already attributed to Windows Defender, and one specific to this repo's own tooling rather than a general Windows antivirus quirk. Not proven to be *the* cause of this session's specific corruption over Defender alone (no Defender scan-history forensics were available — checking `Get-MpPreference`/exclusions requires admin rights this session didn't have), but it's a real gap regardless of which actor is responsible for any given occurrence, and it's the one AgentMux itself controls.

## Fixes

**Applied this session (`vite.config.ts`):** added `target/**` to the dev-server watcher's `ignored` list. Removes AgentMux's own dev-server process as a possible racing actor, independent of whichever external factor (Defender, something else) also contributes.

**Recommended, not applied (needs machine-admin rights this session doesn't have):** add a Windows Defender exclusion for the repo's `target/` directory, per the original retro's Option A:
```powershell
Add-MpPreference -ExclusionPath "<repo>\target"
```

**Recommended follow-up (not applied — a more delicate change to an already carefully-reasoned script):** harden `repair-cef-extract.sh`'s detection to catch a *partially* populated destination, not just missing/empty — e.g. compare file counts (or do an `rsync`-style reconciliation) between `cef_binary_*/include` and `cef_windows_x86_64/include` instead of a presence/emptiness check. Without this, the exact corruption pattern hit this session will keep silently surviving the auto-repair and require a manual `rm -rf` + rebuild every time it recurs.

## Timeline (this session, both incidents)

| Event | Outcome |
|---|---|
| `task dev` #1, fresh checkout | CMake error; Taskfile's automatic repair-and-retry ran, same error persisted |
| Manually inspected `include/` — present, non-empty, missing exactly `cef_id_mappers.h` | Diagnosed repair script's missing/empty-only check as the reason auto-repair no-opped |
| Deleted `cef_windows_x86_64/` (kept the `.tar.bz2`), rebuilt | Succeeded |
| `task dev` #2, minutes later, fresh `main` pull | Same CMake error, same file missing |
| Deleted `cef_windows_x86_64/` again, rebuilt | Succeeded |
| Found `vite.config.ts` doesn't exclude `target/` from the dev-server watcher | Applied the fix |
