# Crash Dump Analysis — agentmuxsrv-rs.exe — 2026-03-26

**Crash code:** `0xC0000409` (`STATUS_STACK_BUFFER_OVERRUN` / `__fastfail`)
**Binary:** `agentmuxsrv-rs.exe` (Rust sidecar, 8.1 MB, built 2026-03-26 13:40)
**OS:** Windows 10 Pro 10.0.19045 (22H2)

---

## 1. TL;DR — Why Dumps Are Not Being Captured

The WER `LocalDumps` registry key for `agentmuxsrv-rs.exe` **is correctly configured** (`DumpType=2`, `DumpCount=10`, no `DumpFolder` override so it targets `%LOCALAPPDATA%\CrashDumps`). However, dumps are still not appearing because `0xC0000409` (`STATUS_STACK_BUFFER_OVERRUN`) is a **`__fastfail` intrinsic**, which invokes a kernel fast-path that **terminates the process before the WER exception handler ever runs**. On Windows 8.1+, `__fastfail` bypasses the entire SEH/VEH chain and calls `NtRaiseHardError` directly; WER's `WerFault.exe` is never launched, and no post-mortem debugger attachment occurs. Additionally, the WER service (`WerSvc`) is **stopped with `StartType: Manual`** — even for exception codes that do reach WER normally, the service may not be running when needed. The combination of a bypass-by-design exception code and a dormant WER service explains the complete absence of dumps.

---

## 2. WER Registry State

### What is configured

```
HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\agentmuxsrv-rs.exe
  DumpType  = 2   (full heap dump)
  DumpCount = 10
  DumpFolder = [NOT SET → defaults to %LOCALAPPDATA%\CrashDumps]
```

### What is missing / wrong

| Item | State | Required |
|------|-------|----------|
| `DumpFolder` value | Not set (uses default) | Default is fine if WER runs at all |
| `%LOCALAPPDATA%\CrashDumps` folder | **Does not exist** | Created on first dump write (OK if WER fires) |
| `HKLM WER Disabled` | Not set (WER enabled at policy level) | OK |
| `HKCU LocalDumps` | Not present | Not needed (HKLM subkey takes precedence) |
| `WerSvc` service | **Stopped, StartType=Manual** | Should be Running or at least demand-start |
| Root `LocalDumps` key values | Empty (no default DumpType) | OK — per-app subkey overrides |

**Verdict:** The per-app registry key itself is syntactically correct. The absence of a `DumpFolder` is intentional and valid. The WER service being stopped is a contributing factor but not the root cause (WER can still auto-start on demand for normal exceptions).

---

## 3. `__fastfail` / `0xC0000409` Mechanics — Why This Specific Code Bypasses WER

### What `__fastfail` is

`__fastfail` is a **security-hardened fast termination path** introduced with the Windows 8 / MSVC 2012 era as part of Control Flow Guard and stack cookie (`/GS`) failure handling. Rust's compiler backend (LLVM) emits `__fastfail` calls in several situations:

- Stack buffer overflow detected by LLVM's `__chkstk` or GS cookie
- Rust's `panic = "abort"` on unrecoverable panics in some configurations
- Security-sensitive contract violations in the runtime

The NTSTATUS code `0xC0000409` maps to `STATUS_STACK_BUFFER_OVERRUN` in the Windows header, but this name is **misleading** — Microsoft reused this code for `__fastfail` broadly. The actual cause may be any `__fastfail` call, not necessarily a stack overrun. Common `__fastfail` codes (the second parameter) include:

| Code | Meaning |
|------|---------|
| 0 | `FAST_FAIL_LEGACY_GS_VIOLATION` |
| 7 | `FAST_FAIL_FATAL_APP_EXIT` (Rust `process::abort()`) |
| 5 | `FAST_FAIL_INVALID_ARG` |

### Why WER does not fire

On Windows 8.1 and later, `__fastfail` translates to a single `int 0x29` instruction (x86-64) which triggers a **kernel-mode exception filter**. The sequence is:

1. `int 0x29` is executed in user mode
2. CPU traps to kernel `KiRaiseSecurityCheckFailure`
3. Kernel calls `NtRaiseHardError` with `STATUS_STACK_BUFFER_OVERRUN` and the `OptionAbortRetryIgnore` response option set to **abort immediately**
4. Process is terminated **before returning to user mode**
5. WER's `WerFault.exe` is **never launched** — the kernel does not give WER a chance to capture the dump

This is by design: the kernel refuses to trust user-mode code for anything after a security-critical failure. Contrast with a normal unhandled exception (e.g., `0xC0000005` access violation), where the kernel calls back into user mode at the `KiUserExceptionDispatcher` which eventually invokes `WerFault.exe`.

### Rust-specific context

Looking at `Cargo.toml` (`C:\Systems\agentmux\Cargo.toml`):

```toml
[profile.release]
strip = true
lto = true
codegen-units = 1
opt-level = "s"
```

There is **no `panic = "abort"`** configured. This means the release profile uses the default `panic = "unwind"`. However, Rust's standard library still calls `__fastfail` (via `process::abort()`) in certain unrecoverable situations even without `panic = "abort"` — notably on stack overflow detection (`SIGSEGV` on alt stack, translated to abort on Windows), double-panics, and FFI boundary violations. The crash **is not triggered by a normal Rust panic**, but by one of these abort paths.

### Binary PE header analysis

```
DLL Characteristics: 0x8160
Flags: DYNAMIC_BASE (ASLR), NX_COMPAT (DEP)
```

The binary has ASLR and DEP enabled. Notably **no `GUARD_CF` (Control Flow Guard)** flag — CFG is disabled, which is typical for Rust binaries built without MSVC's `/guard:cf`. This is not the cause of the `__fastfail` but is a secondary observation.

---

## 4. Evidence from Event Logs

### agentmuxsrv-rs.exe crashes

**Zero event log entries** for `agentmuxsrv-rs.exe` in the Application event log (IDs 1000/1001) for the past 7 days. This is **consistent with `__fastfail`**: because the process is terminated by the kernel without passing through WER, no `APPCRASH` event is written to the Application log either.

### Other crash evidence found

| Process | Time | Exception | Note |
|---------|------|-----------|------|
| `msedgewebview2.exe` | 2026-03-25 14:41 | `0xe0000008` | WebView crash, has dump in WER Temp |
| `chrome.exe` (×3) | 2026-03-24 16:35 | `0xc00000fd` (stack overflow) | Normal SEH path, dump created |
| `SearchApp.exe` | 2026-03-24 16:23 | `0xc00001ad` | Normal SEH path, dump created |
| `Traktor.exe` (hang) | 2026-03-24 10:04 | AppHangB1 | Full minidump + heapdump in ReportQueue |

**Key observation:** WER is fully functional for normal exception codes — `chrome.exe` stack overflows (`0xc00000fd`) produced dumps correctly. The problem is **specific to `__fastfail`**, not to WER being broken.

### agentmux.exe WER archive (historical)

Two reports from 2026-03-06 for `agentmux.exe` (the Tauri frontend, not the sidecar):
- Exception `0xc000041d` — `STATUS_CALLBACK_RETURNED_WHILE_IMPERSONATING` (abnormal termination in WebView2/Tauri layer)
- Exception `0xc0000005` — access violation

These are older crashes of the Tauri host process, unrelated to the current `agentmuxsrv-rs.exe` issue. **Neither of these is `0xC0000409`**, and neither produced a dump file in the archive (only `.wer` metadata, no `.mdmp`).

---

## 5. What Was Found in WER Report Queue/Archive

### ReportQueue (pending upload)

Empty — no pending reports.

### ReportArchive (processed)

No entries for `agentmuxsrv-rs.exe`. The only `agentmux`-related entries are:

```
AppCrash_agentmux.exe_2c302cab...  2026-03-06  (exception 0xc000041d)
AppCrash_agentmux.exe_fcaad454...  2026-03-06  (exception 0xc0000005)
```

Both contain only a `Report.wer` metadata file, no `.mdmp` dump file. This is because the `LocalDumps` registry key for `agentmuxsrv-rs.exe` was not present at the time of those crashes (it was added later per the project memory notes), and even if it had been, neither of those exceptions was `__fastfail`.

### Crashpad dumps (EBWebView)

The following Crashpad `.dmp` files exist for the WebView2 component (not the Rust sidecar):

```
C:\Users\area54\AppData\Local\ai.agentmux.app.dev\EBWebView\Crashpad\reports\  (4 files)
C:\Users\area54\AppData\Local\ai.agentmux.app.v0-32-75\EBWebView\Crashpad\reports\  (1 file)
C:\Users\area54\AppData\Local\ai.agentmux.app.v0-32-82\EBWebView\Crashpad\reports\  (1 file)
```

These are WebView2/Chromium renderer crashes captured by Chromium's own Crashpad handler, independent of WER. They do not contain sidecar stack traces.

---

## 6. Solutions Ranked by Effort

### Option A — Fix WER to also trigger for `__fastfail` (Registry tweak)

**Will it work?** Partially. There is a registry value `HKLM\SYSTEM\CurrentControlSet\Control\CrashControl\ForceDumpsEnabled` (Windows 10 version 1809+) that instructs the kernel to write a user-mode minidump even for `__fastfail` / `STATUS_STACK_BUFFER_OVERRUN`. This was added specifically for this problem.

**Steps:**

```reg
Windows Registry Editor Version 5.00

[HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\agentmuxsrv-rs.exe]
"DumpType"=dword:00000002
"DumpCount"=dword:0000000a
"DumpFolder"="C:\\CrashDumps\\agentmuxsrv"

[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\CrashControl]
"ForceDumpsEnabled"=dword:00000001
```

**Note:** `ForceDumpsEnabled` tells the kernel to invoke WER even for `__fastfail`. This is the "right" fix and requires no code changes. **Effectiveness: moderate** — it works for `__fastfail` codes that originate from the main thread but may still miss some low-level kernel-initiated terminations. Also ensure the dump folder exists: `mkdir C:\CrashDumps\agentmuxsrv`.

**Effort:** Very low. One registry file import.

---

### Option B — ProcDump with `-e 1 -x` (Temporary measure)

**Will it work?** Partially. ProcDump registers itself as the **AeDebug** post-mortem debugger via `-i`, which fires for unhandled exceptions that reach the debugger notification point. However, `__fastfail` **bypasses the AeDebug path** on Windows 10 just as it bypasses WER. ProcDump's `-e 1` flag ("write dump on first chance exception") does not help here either.

The **only ProcDump approach that can work** is to attach ProcDump to the running sidecar process in monitoring mode with a filter on the specific exception code:

```cmd
procdump64 -accepteula -e -f C0000409 -ma -x C:\CrashDumps agentmuxsrv-rs.exe
```

This uses ProcDump's `-f` (filter by exception code) and `-e` (on unhandled exception) flags. However, even this may not fire for `__fastfail` since the kernel terminates the process before raising a debugger-catchable exception. ProcDump's `-ma` (full dump) with `-e` is most likely to work if ProcDump is already attached and using a kernel-level breakpoint.

**Alternative that definitely works with ProcDump:** Use `-t` (write dump on process termination):
```cmd
procdump64 -accepteula -t -ma agentmuxsrv-rs.exe C:\CrashDumps\
```
This captures a snapshot of memory at the moment the process exits, regardless of how it exits. The dump will exist but may not have a meaningful exception record in the dump header (since the process was killed externally). Still useful for heap analysis.

**Effort:** Medium — requires downloading Sysinternals ProcDump and keeping it attached.

---

### Option C — Crashpad / minidump-writer Embedded in Rust Code

**Will it work?** Yes, but only if the handler is installed before the `__fastfail`. The Rust crate `crashpad` (or `minidump-writer`) can register a custom Windows exception handler via `SetUnhandledExceptionFilter`. However, for `__fastfail`, this filter is **not called** by the OS. The only way crashpad-style handlers can capture `__fastfail` is if they also register a **Vectored Exception Handler (VEH)** with the highest priority, which runs before the SEH chain — but even VEH is bypassed by `__fastfail` on Windows 8.1+.

Chromium's Crashpad works around this by using an **out-of-process crash handler** that monitors the target process via a shared memory pipe. When the process terminates unexpectedly, the handler detects the exit and tries to read the thread contexts. This is complex to set up from Rust.

**Recommended crates:**
- `crash-handler` crate (provides a Rust-safe wrapper for this pattern)
- `minidump-writer` for the actual dump format

**Effort:** High. Requires adding dependencies, a signal/crash handler infrastructure, and testing that it fires for `__fastfail`.

---

### Option D — VEH (Vectored Exception Handler) in a C Shim

**Will it work?** No for `__fastfail`. A VEH is a user-mode construct installed via `AddVectoredExceptionHandler`. On Windows 8.1+, `__fastfail` (`int 0x29`) is handled **entirely in kernel mode** and never dispatches back to user mode for any handler (SEH, VEH, or AeDebug). A C shim wrapping the binary adds significant complexity with no gain for this specific exception code.

**Effort:** High, and ineffective for `__fastfail`.

---

### Option E — `SetUnhandledExceptionFilter` + `MiniDumpWriteDump` in Rust Startup Shim

**Will it work?** No for `__fastfail`, same reason as Option D. `SetUnhandledExceptionFilter` is a user-mode callback that runs when the process has an unhandled exception that would normally cause termination. `__fastfail` does not go through this path.

However, this approach is **excellent for catching normal Rust panics** (with `panic = "unwind"`) and other unhandled exceptions like access violations. Consider it complementary: add it to catch the panics that WER might miss (e.g., panics in threads other than the main thread), while using Option A for `__fastfail`.

**Effort:** Medium. Can be implemented as a small Rust `fn main()` wrapper using the `windows` crate.

Example Rust code skeleton:
```rust
use windows::Win32::System::Diagnostics::Debug::{
    SetUnhandledExceptionFilter, MiniDumpWriteDump, MiniDumpWithFullMemory,
    EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
};

unsafe extern "system" fn crash_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // write dump to file, then return EXCEPTION_CONTINUE_SEARCH (0)
    0
}

fn install_crash_handler() {
    unsafe { SetUnhandledExceptionFilter(Some(crash_handler)); }
}
```

---

## 7. Recommended Path

**Primary fix (do immediately): Option A — `ForceDumpsEnabled` registry key + ensure `DumpFolder` exists.**

This is the correct, zero-code-change fix specifically designed for the `__fastfail` problem. Microsoft added `ForceDumpsEnabled` precisely because `__fastfail` was silently terminating processes without WER dumps. It works at the kernel level and is the documented solution for this scenario.

**Secondary fix (complementary): Option E — `SetUnhandledExceptionFilter` in Rust.**

This covers all the other ways the sidecar can die (panics, access violations, FFI violations) that `ForceDumpsEnabled` does not help with. It is also useful for capturing a dump before `panic = "abort"` terminates the process. Add this to `main.rs` in the next sprint.

**Temporary immediate measure (no ProcDump currently installed):** Download Sysinternals ProcDump, use `-t -ma` attach to catch termination dumps as a backup while Option A is being validated.

**Do not pursue:** Options B (VEH shim), C (crashpad), D (VEH) as primary approaches — they are either ineffective for `__fastfail` or have high implementation cost.

---

## 8. Exact Registry File to Apply

Save as `C:\Systems\agentmux\tools\enable-crash-dumps.reg` and double-click to import (requires Administrator):

```reg
Windows Registry Editor Version 5.00

; Enable WER dumps for agentmuxsrv-rs.exe
; DumpType 2 = full heap dump
; DumpFolder must exist; create it first:
;   mkdir C:\CrashDumps\agentmuxsrv

[HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\agentmuxsrv-rs.exe]
"DumpType"=dword:00000002
"DumpCount"=dword:0000000a
"DumpFolder"="C:\\CrashDumps\\agentmuxsrv"

; ForceDumpsEnabled: kernel-level hook so __fastfail / 0xC0000409 also triggers WER
; This was added in Windows 10 1809 (OS build 17763). This machine is 19045 - supported.
[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\CrashControl]
"ForceDumpsEnabled"=dword:00000001

; Ensure WerSvc starts automatically so WER is ready when crash occurs
[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\WerSvc]
"Start"=dword:00000002
```

**To apply from command line (as Administrator):**
```cmd
mkdir C:\CrashDumps\agentmuxsrv
reg import C:\Systems\agentmux\tools\enable-crash-dumps.reg
net start WerSvc
```

**To verify after next crash:**
```cmd
dir C:\CrashDumps\agentmuxsrv\*.dmp
```

---

## 9. Exact ProcDump Command (Temporary Immediate Measure)

First, download ProcDump (no ProcDump found on this machine):
```powershell
# Download Sysinternals ProcDump to C:\Tools
New-Item -ItemType Directory -Force C:\Tools | Out-Null
Invoke-WebRequest -Uri "https://download.sysinternals.com/files/Procdump.zip" -OutFile "C:\Tools\Procdump.zip"
Expand-Archive "C:\Tools\Procdump.zip" -DestinationPath "C:\Tools" -Force
```

Then, to attach to a running `agentmuxsrv-rs.exe` and capture a dump on termination:
```cmd
C:\Tools\procdump64.exe -accepteula -t -ma agentmuxsrv-rs.exe C:\CrashDumps\agentmuxsrv\
```

This writes a full minidump when the process exits for any reason.

To register as a crash-on-exception watcher (best effort — may not fire for `__fastfail`):
```cmd
C:\Tools\procdump64.exe -accepteula -e -f c0000409 -ma agentmuxsrv-rs.exe C:\CrashDumps\agentmuxsrv\
```

The `-f c0000409` flag filters for the specific exception code. Whether this fires depends on whether ProcDump's kernel callback precedes the `__fastfail` termination — it may not, but it costs nothing to try alongside `-t`.

---

## Appendix: Raw Data Summary

### WER Registry

```
HKLM\...\LocalDumps\agentmuxsrv-rs.exe
  DumpType  = 2
  DumpCount = 10
  DumpFolder = [not set, default = %LOCALAPPDATA%\CrashDumps]

HKLM\...\Windows Error Reporting
  EnableZip = 1
  Disabled  = [not set]

HKLM\SOFTWARE\Policies\...\Windows Error Reporting
  Disabled  = [not set]
```

### WER Service

```
Name:      WerSvc
Status:    Stopped
StartType: Manual
Registry Start value: 3 (demand start)
```

### CrashDumps Folder

`C:\Users\area54\AppData\Local\CrashDumps` — **does not exist**

### Binary PE Flags

```
DLL Characteristics: 0x8160
  DYNAMIC_BASE (ASLR): yes
  NX_COMPAT (DEP): yes
  GUARD_CF (CFG): no
  NO_SEH: no
```

### Cargo.toml panic strategy

```toml
[profile.release]
strip = true
lto = true
# NO panic = "abort" configured
# Default: panic = "unwind"
```

### Event Log Findings (last 7 days)

- Zero `agentmuxsrv-rs.exe` entries in Application log (IDs 1000/1001)
- WER fully functional for chrome.exe, msedgewebview2.exe, SearchApp.exe crashes
- Historical agentmux.exe crashes (2026-03-06): no dumps captured (WER key not yet set at that time)
