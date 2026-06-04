# SPEC: macOS Accessibility Robustness — surviving external AX clients without crashing

**Status:** Draft · **Date:** 2026-06-03 · **Owner:** AgentMux host/macOS
**Related:** `SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md`, `REPORT_MACOS_TEAROFF_DRAG_CRASH_2026_05_29.md`, `docs/cef-patches/README.md`, `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md`

---

## 0. Motivation — handle the box *as it is*

A user on a normal macOS-26 machine clicked AgentMux's title-bar menu and the host
crashed (`EXC_BREAKPOINT`/`SIGTRAP`, main thread). The launcher brought it back, but it
recurred (6 host generations in one session). The crash is **not** caused by the menu's
position or CSS — it is a **Chromium/CEF macOS accessibility fault** reached through an
**incoming, cross-process accessibility query** from another app on the box.

Confirmed trigger on this machine: **Magnet** (window manager) and **Synergy** (KVM) — both
consume the macOS Accessibility API and poll other apps' AX trees. When the menu's portal
overlay opens, the app's AX tree mutates, one of these tools queries the new element, and
Chromium traps building the response.

This is a **robustness exercise, not a happy-path exercise.** The goal is that AgentMux
runs reliably on a real developer's machine — with window managers, KVMs, clipboard tools,
launchers, automation, and genuine screen readers present and poking at it — **without
crashing and without disabling accessibility to hide the problem.** When a real screen-reader
user runs AgentMux, accessibility should *work*, not crash.

Explicit non-goal / anti-pattern: blanket `--disable-renderer-accessibility`. It stops the
crash by making AgentMux permanently inaccessible to assistive technology. That is the happy
path we are rejecting.

---

## 1. Root-cause analysis

### 1.1 The crash
- Signal: `EXC_BREAKPOINT (SIGTRAP)`, faulting thread 0 (main/UI).
- Stack (bottom → top, the meaningful frames):
  `_AXXMIGCopyAttributeValue` → `-[NSApplication accessibilityParent]` →
  `NSAccessibilityGetObjectForAttributeUsingLegacyAPI` → `NSAccessibilityAttributeValue` →
  `NSAccessibilitySetUnsupportedAttributeError` → `+[NSString stringWithFormat:]` → … →
  Chromium Embedded Framework (web-content AX node code) → trap.
- `_AXXMIGCopyAttributeValue` is an **out-of-process** AX request (XMIG = Mach IPC). Something
  external asked the app for an attribute value; the legacy AX accessor descended into the
  CEF/Chromium web-content AX tree and trapped.

### 1.2 Why Chromium is in that code path at all
Chromium does **not** build its accessibility tree until it detects assistive technology, for
performance. On macOS the detection signal is the **`AXEnhancedUserInterface`** attribute being
set on the application — the same attribute VoiceOver sets. Once set, Chromium enables full
accessibility and builds the browser-process web-content AX tree (`BrowserAccessibilityCocoa` /
`AXPlatformNodeCocoa`). (Chromium accessibility overview, refs §8.)

The problem: **`AXEnhancedUserInterface` is overloaded.** It is set/triggered not only by
VoiceOver but by ordinary window managers and automation tools (Magnet, Synergy, etc.). This is
a known, long-standing defect — see Firefox bug 1664992, *"AXEnhancedUserInterface breaks window
managers"*, and Electron's move to a separate `AXManualAccessibility` attribute (refs §8). So a
window manager merely doing its job forces Chromium into the heavy, **crash-prone** full-AX mode
even though no real screen reader is present.

### 1.3 The actual fault, once AX is on
Two upstream defects, both reached by an external client iterating the tree:

- **CEF #3512 — "cefclient crashes when the menu button is pressed" (mac/views).** `SIGSEGV`
  in `AXPlatformNodeCocoa::AXChildren()` while the system queries accessibility on a menu /
  address-bar click. Affects CEF M114+ / macOS 13.3+. **Unresolved upstream.** This is our
  scenario almost exactly (menu → AX query → AX-node iteration → fault).
- **macOS-26 legacy-AX ObjC termination.** Our top frames go through
  `NSAccessibilitySetUnsupportedAttributeError`, which **raises an `NSException`** on the legacy
  AX path. An uncaught ObjC exception on macOS routes to `_objc_terminate()` →
  `EXC_BREAKPOINT` **before Rust panic machinery runs** — the *identical* mechanism AgentMux
  already shims for non-AX private selectors (see §3.2). macOS 26 (Tahoe) tightened/relocated AX
  internals, so selectors/attributes CEF expects are unsupported, taking this throw path.

### 1.4 Why this is *not* the title-bar menu change
- The flyout has **always** rendered through a `Portal` (left-side hamburger included); the AX
  tree is built from DOM **semantics**, not visual CSS. `transform`/`flex-direction`/position do
  not alter the AX tree. The old left menu and every other flyout/context menu fault the same way
  under the same external query.
- It reproduces independent of the mirrored-menu work; the menu was merely the interaction that
  mutated the AX tree and invited the query.

---

## 2. Design principles

1. **Robust, not evasive.** Do not globally disable accessibility. Keep AgentMux usable by real
   assistive technology.
2. **Correctness of activation.** A window manager / KVM polling the app should get *window-level*
   AX (all it needs) **without** forcing Chromium's heavy web-content AX mode. Full web-content AX
   should activate only for genuine assistive technology or explicit app intent. (This is exactly
   what Firefox bug 1664992 and Electron's `AXManualAccessibility` argue for.)
3. **Defense in depth (layered).** Prevent → fix-at-source → contain → recover → observe. No single
   layer is trusted to be perfect; each reduces blast radius.
4. **Reuse what exists.** AgentMux already (a) shims macOS-26 ObjC-selector termination in the host,
   (b) supervises the host from the launcher with a crash-forensics event log, (c) maintains a
   patched CEF fork, (d) persists pane/window/session state. Extend these rather than invent.
5. **Prove it with the box as-is.** Acceptance is measured with Magnet + Synergy (and VoiceOver)
   actually running, not in a sanitized environment.

---

## 3. The layered solution

### Layer 1 — Govern AX activation (primary fix, host-only, no CEF rebuild)

**Intent:** stop non-screen-reader tools from forcing Chromium into the crash-prone full-AX mode,
while still letting genuine assistive technology turn it on.

**Mechanism:** intercept the application's accessibility activation. Chromium enables AX when it
observes `AXEnhancedUserInterface=true` set on its `NSApplication` (via
`-[NSApplication accessibilitySetValue:forAttribute:]`). The host already owns ObjC-runtime
manipulation of `NSApplication` (`patch_nsapp_unrecognized_selector`, `main.rs:846`). Add a sibling
hook that **swizzles `-[NSApplication accessibilitySetValue:forAttribute:]`** (and the legacy
`accessibilitySetValue:forAttribute:` accessor) with a policy:

- `AXManualAccessibility` → honor it (explicit assistive-technology / app intent) → allow full AX.
- `AXEnhancedUserInterface` → **do not** auto-enable full web-content AX merely because it was set;
  gate on a real screen-reader signal (e.g. VoiceOver running — `defaults`/`AXIsProcessTrusted` /
  `voiceOverEnabled`), or an app setting. Window-level AX (windows, title, buttons) still answers.
- Expose an explicit app control: setting `accessibility:web_tree` (default `auto`) and/or a
  runtime command mirroring Electron's `app.setAccessibilitySupportEnabled(true)`, so the app (or a
  user who needs it) can force-enable.

**Result:** Magnet/Synergy polling the window AX tree no longer descend into the crashy
web-content nodes → no crash, and AgentMux is *not* globally inaccessible. Real screen-reader users
(VoiceOver, or `AXManualAccessibility`) still get full AX — guarded by Layer 2.

**Risks / care:**
- Must not break CEF's own `CefAppProtocol` `NSApplication`; wrap/swizzle, never replace.
- Swizzle once, immediately **after** `cef::initialize` — the CEF `NSApplication`
  subclass that owns the legacy AX setter only exists post-init (this is why the
  governor installs in the post-init macOS-setup block, not alongside the
  pre-init `patch_nsapp_unrecognized_selector` shim). See `main.rs` call site
  (post-`initialize`) and the function's own doc comment.
- Verify macOS-26 still activates via `AXEnhancedUserInterface` (refs §8) — confirm the exact
  selector/attribute names empirically (Accessibility Inspector + a probe build).

### Layer 2 — Make the AX path itself non-fatal (source-level guard)

**Intent:** when full AX legitimately activates (real screen reader, or `AXManualAccessibility`),
the tree-iteration / legacy-accessor path must degrade gracefully instead of trapping.

Two sub-options, prefer (a) first because it needs no CEF rebuild:

- **(a) Extend the host's ObjC firewall.** The crash is an **uncaught ObjC exception** on the
  legacy-AX accessor path (`NSAccessibilitySetUnsupportedAttributeError`) — the same class the
  existing `+resolveInstanceMethod:` shim was built for (`main.rs:816-925`, which already documents
  `_objc_terminate → EXC_BREAKPOINT`). Extend that shim to (i) provide **typed nil-returning stubs**
  for unsupported AX selectors so the unsupported-attribute throw is never reached, and/or (ii)
  install an **AppKit uncaught-exception firewall** scoped to the AX accessor so an unsupported
  attribute returns `nil`/`kAXErrorAttributeUnsupported` instead of terminating. Today's allowlist
  is `void`/`BOOL` stubs only; AX accessors return *objects* and need an `id`-returning `nil` stub
  (a `void` stub leaves `x0=self`, a garbage object — actively dangerous here).
- **(b) CEF fork patch.** If the fault is the `AXPlatformNodeCocoa::AXChildren()` SEGV (CEF #3512,
  invalid node lifecycle) rather than an ObjC throw, add a defensive null/lifecycle guard to the
  AgentMux CEF fork (we already carry `agentmux_disable_mach_rendezvous_validation` +
  `dcheck_always_on=false`; this is one more patch). Requires symbolized crash site (see §5).

**Dependency:** confirm whether the terminal fault is the ObjC throw (→ 2a) or the node SEGV
(→ 2b) by symbolizing against a **from-source DWARF** framework (the dev/prebuilt framework is
stripped; UUIDs won't match — see §5). Until symbolized, implement 2a (cheap, host-only) and
measure.

**Outcome:** a screen-reader user gets a possibly-slightly-degraded but **non-crashing** AX tree.

### Layer 3 — Crash containment & seamless recovery (general resilience)

This backstops **any** unforeseen crash (AX or otherwise) — the "box as-is" guarantee.

- **Supervised restart with state restore.** The launcher already runs a supervisor thread + a
  crash-surviving `event_log` (`agentmux-launcher/src/{splash_mac,event_log,state}.rs`). Define and
  verify the policy: on host crash, the launcher restarts the host and the **reducer/saga-persisted
  pane + window + session state** (`agentmux-cef/src/{saga_dispatch,state,events}.rs`,
  srv-side persistence) restores the full session — windows, panes, agent sessions, auth — with
  minimal visible disruption. Confirm current behavior (does the launcher respawn, or exit?) and
  close any gap so a single host crash never loses work.
- **Crash-loop breaker.** Bound restarts: if the host dies ≥ N times in T seconds (a deterministic
  crash, e.g. an AX client polling on a timer), stop thrashing, surface a clear diagnostic + a
  manual "relaunch" affordance, and record the signature. Prevents the 6-respawns-a-minute pattern.
- **Document the limit.** A browser-process AX crash is inherently app-wide in CEF's single
  browser-process model; per-window OS processes don't isolate it. Layers 1–2 prevent it; Layer 3
  ensures graceful recovery when prevention is incomplete.

### Layer 4 — Observability & telemetry

- **Crash-report ingestion.** On host start, scan `~/Library/Logs/DiagnosticReports/agentmux-cef-*.ips`
  for new reports, parse the signature (signal + top frames), classify (AX vs other), and log +
  optionally surface. (This spec's diagnosis was done by hand; automate it.)
- **Symbolication pipeline.** Keep a from-source DWARF framework (`symbol_level=1`) to resolve field
  crashes with `llvm-symbolizer` (per the dcheck retro: `atos` only reads the sparse export table).
- **Metrics.** host-crash rate, AX-crash rate, respawn count, crash-loop-breaker trips.

---

## 4. Testing & validation — the robustness exercise

**Reproduction harness (deterministic):** a small macOS AX client (Swift/ObjC using
`AXUIElementCreateApplication` + `AXUIElementCopyAttributeValue`, or a tiny CLI) that attaches to
the running `agentmux-cef`, enables AX (`AXManualAccessibility` / `AXEnhancedUserInterface`), and
**recursively enumerates** the AX tree while the UI is driven to open/close the title-bar menu and
context menus. This reproduces the external-query crash without needing a human to wiggle Magnet.

**Matrix (run with the box as-is):**
- Tools present: Magnet on; Synergy on; VoiceOver on; combinations; none (control).
- Interactions: open/close hamburger menu (mirrored, far-right), context menus, submenus, tab
  tear-off, multiple windows.
- Builds: dev (prebuilt CEF) and packaged (patched from-source CEF).

**Acceptance criteria:**
1. **L1:** with Magnet/Synergy running and VoiceOver off, the enumeration + menu loop runs ≥ 1000
   iterations with **zero** host crashes; Chromium web-content AX stays inactive (verify via AX
   probe: web nodes absent).
2. **L2:** with VoiceOver on (or `AXManualAccessibility` forced), the AX tree is exposed and the
   same loop runs ≥ 1000 iterations with **zero** crashes; menu items are reachable via AX.
3. **L3:** an injected/forced host crash restores the full session (windows, panes, agent sessions)
   within a defined budget; ≥ N crashes in T seconds trips the crash-loop breaker with a diagnostic.
4. **L4:** every crash in the run is captured, classified, and counted by the telemetry path.

**Regression gate:** the harness becomes a CI/local check (macOS runner) so AX robustness can't
silently regress.

---

## 5. Phasing & dependencies

- **Phase 1 (unblocks dev now; host + launcher only, no CEF rebuild):**
  - L1 activation governor (swizzle `accessibilitySetValue:forAttribute:`).
  - L2(a) host ObjC firewall: nil-returning AX selector stubs + scoped uncaught-exception guard,
    extending `patch_nsapp_unrecognized_selector`.
  - L3 crash-loop breaker in the launcher.
  - Repro harness + acceptance criteria 1.
- **Phase 2 (needs the from-source toolchain):**
  - Symbolize the crash site (from-source DWARF, `llvm-symbolizer`) to decide 2a vs 2b.
  - If needed, L2(b) CEF fork patch (`docs/cef-patches/`), rebuilt + notarized framework.
  - L3 state-restore verification/closure; L4 telemetry + symbolication pipeline.
- **Phase 3:** track CEF #3512 / Chromium upstream; bump Chromium when fixed and **remove** the
  patch; keep L1 (it is correct behavior regardless of the bug).

---

## 6. Open questions / risks

- **Exact terminal fault** (ObjC throw vs `AXChildren` SEGV) — resolve by symbolization (§5);
  determines whether Phase 1's host-only L2(a) suffices or a CEF patch (L2(b)) is required.
- **macOS-26 activation path** — confirm Chromium still keys off `AXEnhancedUserInterface` on Tahoe
  and capture the precise attribute/selector names empirically before swizzling.
- **Swizzle safety** — must not regress genuine VoiceOver support or CEF's `CefAppProtocol` app.
- **Current launcher restart semantics** — confirm respawn vs exit and the exact state-restore gap.
- **AgentMux self-dependence on AX** — confirm the test API / e2e harness is IPC-based and does not
  rely on the macOS AX tree (expected: yes, IPC — `SPEC_TEST_API_ACCESS.md`).

---

## 7. Decision: recommended path

Ship **Phase 1** immediately — it is host/launcher-only, needs no CEF rebuild, fixes the common case
(window managers / KVMs, i.e. *this* incident), keeps accessibility available, and adds a crash-loop
breaker so any residual crash can't thrash. Then symbolize and decide Phase 2. This is the robust,
non-evasive delivery: AgentMux survives the box as-is and remains accessible.

---

## 8. References

- CEF #3512 — *cefclient crashes when the menu button is pressed* (mac/views; `AXPlatformNodeCocoa::AXChildren` SIGSEGV): https://github.com/chromiumembedded/cef/issues/3512
- Firefox bug 1664992 — *AXEnhancedUserInterface breaks window managers; provide another attribute for non-VoiceOver apps*: https://bugzilla.mozilla.org/show_bug.cgi?id=1664992
- Electron accessibility (manual enable, `AXManualAccessibility`, `setAccessibilitySupportEnabled`): https://www.electronjs.org/docs/latest/tutorial/accessibility
- Electron #7206 — *Allow Mac accessibility for apps other than VoiceOver*: https://github.com/electron/electron/issues/7206
- Electron #37465 — *`AXManualAccessibility` can't be set (`kAXErrorAttributeUnsupported`)*: https://github.com/electron/electron/issues/37465
- Chromium accessibility overview (AT-gated activation, browser-process AX tree): https://chromium.googlesource.com/chromium/src/+/main/docs/accessibility/overview.md
- Chromium Mac accessibility (`AXEnhancedUserInterface` activation): https://www.chromium.org/developers/design-documents/accessibility/
- Internal: `SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md`, `REPORT_MACOS_TEAROFF_DRAG_CRASH_2026_05_29.md` (the existing macOS-26 `_objc_terminate`/`EXC_BREAKPOINT` selector shim), `docs/cef-patches/README.md`, `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md` (symbolization: `llvm-symbolizer`, not `atos`).
- Host hook to extend: `agentmux-cef/src/main.rs:846` (`patch_nsapp_unrecognized_selector`).
- Launcher supervision/forensics: `agentmux-launcher/src/{splash_mac,event_log,state}.rs`.
