# Retro: the macOS 26 crash saga was a CEF build-config bug (DCHECKs enabled)

**Date:** 2026-06-02
**Severity:** P0 — crash on every pane drag / window close on macOS 26
**Resolution:** rebuild from-source CEF with `dcheck_always_on=false` (production config)
**Time lost:** ~a full day of chasing symptoms before symbolizing the real stack

---

## TL;DR

Every crash in this saga was a **DCHECK** — a developer-only assertion that
**production CEF builds compile out**. Our from-source CEF was built with
`is_official_build=false`, which silently defaults `dcheck_always_on=true`, so
DCHECKs were live. macOS 26 changed enough of AppKit/NSApplication that several
macOS-specific DCHECKs now fail. The fix is one build flag: `dcheck_always_on=false`.
This is **not** an AgentMux architecture problem and needs **no** platform-specific
code rethink — just building CEF the way it ships.

---

## How we got lost

The crash reports all showed the same top frames:

```
abort
Chromium Embedded Framework +128753664
Chromium Embedded Framework +128752520    ← these never changed across rebuilds
Chromium Embedded Framework +128656136
Chromium Embedded Framework +128656200
Chromium Embedded Framework +129984784    ← the real crash site (kept changing)
```

**The mistake:** I treated `+128656136 / +128656200` (which are 64 bytes apart and
identical in every crash) as the crash site and tried to map them to source files.
They are actually the **shared CHECK/DCHECK failure machinery** —
`logging::CheckError::~CheckError`, `LogMessage::HandleFatal`, `ImmediateCrash` —
which is the same for *every* failed assertion. The actual crash site is the
**frame above** the logging machinery, which I wasn't symbolizing.

This led to three wrong fixes:
1. `ui/views/view.cc` `CHECK(!iterating_)` → guard (wrong file)
2. `base/callback_list.h` `CHECK(!iterating_)` → guard (wrong file, adds UAF risk)
3. Rust-side gating of `host.set_focus()` and deferring `window.show()` (symptoms)

All three "worked" intermittently only because they perturbed timing; the crash
kept coming back from other code paths.

## The breakthrough: real symbols

`atos` couldn't resolve internal frames (it reads the sparse symbol table, not
DWARF). The from-source framework is 567 MB with full inline DWARF, so
`llvm-symbolizer` (shipped in the Chromium toolchain) resolves it:

```
third_party/llvm-build/Release+Asserts/bin/llvm-symbolizer \
  --obj="…/Chromium Embedded Framework" 0x<imageOffset>
```

UUID check first confirmed the loaded framework == our built framework
(`4C4C444C-5555-3144-A121-FF6A27A620FC`), so symbolization is valid.

Symbolizing the **caller** frame (above the logging machinery) across many crash
reports revealed the real sites — all macOS-specific DCHECKs:

| Crash report | Real crash site | Assertion |
|---|---|---|
| drag (CrBrowserMain) | `base::mac::ScopedSendingEvent::ScopedSendingEvent()` | `DCHECK([app_ conformsToProtocol:@protocol(CrAppControlProtocol)])` |
| drag (CrBrowserMain) | `base::message_pump_apple::IsHandlingSendEvent()` | DCHECK on CrAppProtocol conformance |
| close/exit | `(anonymous)::CefShutdownChecker::~CefShutdownChecker()` (context.cc:46) | `DCHECK(!g_context) << "CefShutdown was not called"` |
| task-dev startup | `util_mac::BasicStartupComplete()` | `DCHECK(!framework_path.empty())` etc |

### Why these fire on macOS 26

`ScopedSendingEvent` is constructed for **every AppKit event** Chromium sends —
which is constant during a drag. Its DCHECK asserts that `NSApp` formally
conforms to `CrAppControlProtocol` (Chromium's `CrApplication` NSApplication
subclass implementing `isHandlingSendEvent` / `setHandlingSendEvent:`). On
macOS 26 our `NSApp` does not formally conform. Our existing
`class_addMethod` shim (main.rs, "macOS 26 compat: adding void stub") adds the
*methods* but `conformsToProtocol:` checks the class's declared protocol list,
which `class_addMethod` does not change — so the DCHECK fails.

`CefShutdownChecker` is a static-lifetime object whose destructor DCHECKs that
`CefShutdown()` was called. CEF apps that let the process exit (or whose
Chromium `ExitHandler::ExitWhenPossibleOnUIThread` fires) never call
`CefShutdown()` — so the DCHECK fires on every exit. This is the "close crash"
and the `__cxa_finalize_ranges`/`exit` stacks we saw.

**In production CEF (and the cef-dll-sys prebuilt), all of these are compiled
out.** That's why the official prebuilt never hit them — the only reason we
went from-source at all was the **-67030 renderer crash**, which is a *real*
functional code-sign failure (Mach-port peer `process_requirement`), not a
DCHECK, and needs the `GetPeerValidationPolicy → kNoValidation` source patch.

## The fix

Build the from-source CEF the way CEF actually ships:

```
# out/Release_GN_arm64/args.gn
is_official_build=false
is_debug=false
symbol_level=1
dcheck_always_on=false   # ← the fix: matches production; DCHECKs compiled out
```

`DCHECK_IS_ON()` = `is_debug || dcheck_always_on`. With both false, every DCHECK
becomes a no-op. The one real patch we keep is
`GetPeerValidationPolicy → kNoValidation` (for -67030).

Reverted (wrong guesses, kept the tree minimal):
- `docs/cef-patches/agentmux_views_reorder_guard.patch`
- `docs/cef-patches/agentmux_callback_list_iterating_guard.patch`

## Does this need a platform-specific rethink? (the strategic question)

**No.** The from-source CEF approach is correct and necessary for -67030. The
crashes were a build-config error, not a macOS architecture flaw. After
`dcheck_always_on=false`, the macOS code path behaves like the production
prebuilt, and the Rust-side workarounds added during the saga
(`set_focus` gating, deferred `window.show()`, `process::exit(0)` on shutdown)
should be reverted — they were symptom-chasing, not real fixes. Keep only the
genuinely platform-specific, genuinely necessary shims:
- `GetPeerValidationPolicy` CEF source patch (-67030)
- NSApplication selector injection / activation policy / drag-slideback swizzle

The bounded ongoing cost is macOS-26-tracking until CEF/Chromium ship official
macOS 26 support; revisit the from-source patches when bumping CEF.

## Lessons

1. **Symbolize before patching.** Use `llvm-symbolizer` (DWARF), not `atos`
   (symbol table), on an unstripped from-source framework. Verify UUID match
   first.
2. **The repeated frames in a CHECK crash are the abort machinery, not the bug.**
   The real site is the first non-`logging::` frame above them.
3. **A from-source Chromium with `is_official_build=false` has DCHECKs on by
   default.** That is a *debug* configuration; never ship it. Set
   `dcheck_always_on=false` for any from-source CEF intended to run like release.
