# CEF vs Tauri: Value Proposition Analysis Post-PR #253

**Date:** 2026-03-30
**Author:** Agent1
**Status:** Strategic analysis — input for landing page + product positioning
**Related:** PR #253 (`agentx/cef-integration`), `docs/specs/cef-transparency-architecture.md`

---

## Executive Summary

PR #253 adds bundled Chromium (CEF 146) as an alternative host alongside Tauri. This is
a technically sound move with major product implications. The current landing page's
primary technical differentiator — **35-125 MB RAM vs 400 MB–3 GB for Electron-based
tools** — becomes partially false once CEF is bundled (~350 MB binary, Chromium runtime
overhead). This document analyzes what the value prop becomes, whether dual-host is
worth maintaining, and what the landing page needs to say.

---

## What PR #253 Actually Delivers

### Working in CEF (Phase 1-3)
- CEF host starts, spawns backend sidecar with Job Object cleanup
- IPC bridge (HTTP POST + CustomEvent dispatch)
- Terminal panes (shell I/O, xterm v6, WebGL renderer)
- Sysinfo panes and charts
- Context menus via pure-JS overlay (no native CEF API)
- Window transparency (DWM + WS_EX_LAYERED)
- Frameless window with custom title bar drag
- Min/max/close buttons
- In-window pane DnD (split, move) and tab DnD (reorder)
- DevTools via remote debugging (port 9222)

### Not Working Yet (Phase 4)
- Cross-window drag and tab tear-off (10 commands stubbed)
- Multi-window support
- File drop full paths (needs `CefDragHandler`)
- White resize border on frameless window
- macOS and Linux hosts (Windows-only so far)

### Architecture Impact
- An `ipc.ts` abstraction layer already exists (`detectHost()`, `invokeCommand()`,
  `listenEvent()`) — the right design for dual-host
- Tauri-specific imports (`@tauri-apps/*`) are being guarded or dynamically imported
- Several platform-split files (`*.tauri.ts`, `*.cef.ts`) are in place

---

## The Old Value Proposition (Pre-CEF)

The current landing page comparison table (`ComparePreview.tsx`) makes this argument:

| Tool | Memory | Price |
|------|--------|-------|
| **AgentMux** | **35-125 MB** | **Free** |
| Cursor | 400 MB–3 GB | From $20/mo |
| Warp | 150-400 MB | Free / $20/mo |
| Codex App | ~300 MB | From $20/mo |
| Claude Cowork | ~200 MB | From $20/mo |

The structural argument was: **Tauri + native webview = fraction of Electron's weight**.
Electron (used by Cursor, VS Code) bundles Chromium (~150 MB) + Node.js. Tauri delegates
to the OS webview (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) —
zero bundled browser.

This was a genuine technical differentiator AND a signal of architectural philosophy:
we're a terminal-class tool, not a bundled browser.

---

## What CEF Changes

### The Numbers

| Build | Install Size | Runtime RAM | Notes |
|-------|-------------|-------------|-------|
| Tauri | ~15-25 MB installer | 35-125 MB | OS webview (WebView2/WKWebView) |
| CEF | ~400 MB installer | 200-400 MB | libcef.dll alone is 262 MB debug |
| Electron (Cursor) | ~300 MB | 400 MB–3 GB | V8 + Node + Chromium |
| Electron (Warp) | ~200 MB | 150-400 MB | Bundled Chromium |

CEF debug builds are large; release binaries are somewhat smaller but CEF is a prebuilt
binary, so `libcef.dll` at 262 MB doesn't shrink much. Installer will be 350-400 MB.

**The Tauri RAM advantage disappears in CEF mode.** The installer size advantage disappears
entirely. CEF and Electron are in the same weight class.

### What CEF Gains

1. **Rendering consistency.** Chromium everywhere — no more WebKitGTK WebGL bug forcing
   Canvas renderer on Linux, no more WKWebView private API hacks for transparency on macOS.

2. **DevTools always available.** Remote debugging on port 9222 works out of the box.
   This is significant for developers extending AgentMux or debugging agents.

3. **Future web widget potential.** A full Chromium instance can embed OAuth flows,
   web-based agent UIs, and rich content that WebKitGTK can't reliably render.

4. **No OS webview version dependency.** WebView2 on older Windows or outdated WebKitGTK
   on Linux distros causes subtle rendering bugs. CEF pins to Chromium 146.

5. **Consistent clipboard, notifications, and Web APIs.** No more
   `@tauri-apps/plugin-clipboard-manager` shims — `navigator.clipboard` just works.

### What CEF Loses

1. **The memory/size talking point.** This was a headline differentiator on the landing page.
2. **Installer simplicity.** 400 MB downloads are a friction point for first-time users.
3. **"Terminal-class tool" positioning.** Bundling Chromium is the Electron trade.
4. **macOS and Linux parity.** CEF is Windows-only in PR #253. Tauri can't be dropped yet.

---

## The Dual-Host Question: Is It Worth Maintaining Both?

### Arguments FOR dual-host (recommended)

**1. Tauri remains necessary until CEF reaches macOS/Linux parity.**
PR #253 is Windows-only. macOS and Linux users need Tauri. There is no question here —
dropping Tauri before CEF ships on all platforms would be a regression.

**2. The IPC abstraction is already designed for this.**
`ipc.ts` with `detectHost()` was the right call. The cost of dual-host is already
encoded in the architecture. Removing Tauri wouldn't simplify much — the abstraction
layer stays regardless because it's also the async-safe IPC pattern.

**3. Tauri is a genuine "lite" tier with a different audience.**
Power-constrained users (laptops, shared machines), users on older hardware, or users
who simply value minimal footprint will prefer Tauri. Calling it "AgentMux Lite" or
offering it as the default download with CEF as "AgentMux Full" creates a real choice.

**4. CEF's Phase 4 is still unfinished.**
Cross-window drag, multi-window, file drop with full paths — these are core to the
AgentMux multi-agent workflow. Tauri handles all of this today. CEF needs more time.

**5. Maintenance cost is bounded, not unlimited.**
The `*.tauri.ts` / `*.cef.ts` platform-split pattern already handles per-host
differences cleanly. New features land in the shared `ipc.ts` surface first; host
divergence is the exception, not the rule. Looking at PR #253: 59 files changed,
but most are new CEF-specific files (`agentmux-cef/` crate), not splits of existing code.

### Arguments AGAINST dual-host

**1. Every new platform feature doubles the test surface.**
Window management, transparency, drag/drop, file system access — each needs two
implementations. This multiplies QA work.

**2. The JS context menu overlay is technical debt.**
CEF has no native context menu API, so PR #253 reimplements context menus in JS.
This diverges from the native feel of the Tauri version and will need independent
maintenance as context menu features grow.

**3. The "lightweight" claim is already eroding.**
The Tauri build's memory advantage was always partly contingent on WebView2 already
being installed on Windows. Once CEF becomes the headline download, the comparison
table numbers become confusing ("35 MB in Tauri mode, 300 MB in CEF mode").

### Verdict: Keep Both, but Clarify the Offering

The right move is **Tauri as the default, CEF as an opt-in "Developer" or "Full" build.**

- Tauri is the first-download experience: fast, lightweight, starts immediately
- CEF is for power users: DevTools, consistent rendering, future web widgets
- This maps to a real user split: most users want a fast tool, developers want DevTools
- It preserves the memory story on the landing page (Tauri numbers stay true)
- It lets CEF mature without blocking the core product

---

## Revised Value Proposition

### What Still Holds

| Claim | Status | Notes |
|-------|--------|-------|
| Run any agent (Claude Code, Codex, Gemini) | ✅ Unchanged | Core differentiator |
| Sub-agent visibility / tool call tracing | ✅ Unchanged | Core differentiator |
| Free, open source | ✅ Unchanged | Apache 2.0 |
| No lock-in to single provider | ✅ Unchanged | Core differentiator |
| Lightweight (Tauri build) | ✅ Still true for Tauri | Must qualify as "Tauri build" |

### What Changed

| Claim | Old | New |
|-------|-----|-----|
| Memory | "35-125 MB" (unconditional) | "35-125 MB (Tauri) / ~300 MB (CEF)" |
| Install size | Implied small | "~20 MB (Tauri) / ~400 MB (CEF)" |
| Rendering | "Native webview per platform" | "Choose: native OS webview or bundled Chromium" |

### New Claims Enabled by CEF

1. **Consistent DevTools.** Debug your agents, inspect tool calls, profile rendering —
   same Chromium DevTools on every platform. No other terminal offers this.

2. **Pinned browser engine.** CEF 146 doesn't change when Windows updates. No more
   surprise rendering regressions from OS-level WebView2 updates.

3. **Full Web API surface.** Web Notifications, Clipboard API, WebGL, WebGPU — no
   OS webview version dependency.

4. **Two install tiers, one codebase.** Tauri for speed, CEF for power. Pick your trade-off.

### The Real Moat (Unchanged by CEF)

CEF vs Tauri is an implementation detail. The actual competitive moat is:

- **Multi-agent orchestration**: run Claude Code + Codex CLI + Gemini CLI simultaneously
  in split panes, watching all of them at once
- **Sub-agent visibility**: see every tool call, every file write, every subagent spawn
  as it happens — not in a dashboard, in the terminal where it's happening
- **Open + local**: no cloud, no subscription, no data leaving the machine
- **Free**: the entire competitive set charges $20/mo

None of this is touched by CEF. The landing page over-indexed on memory numbers.
The real differentiator is the orchestration story.

---

## Landing Page Changes Needed

### ComparePreview.tsx

The RAM column currently reads `35-125 MB` for AgentMux. This needs an update post-CEF:

**Option A (recommended):** Qualify the column header as "Memory (Tauri build)" and
add a footnote that CEF build is ~300 MB. Keep the 35-125 MB as the primary comparison
because that's the default download experience.

**Option B:** Change the column to reflect the lightweight story more broadly —
"download size" instead of runtime RAM — where Tauri's ~20 MB installer still wins
vs Cursor's 300 MB even if CEF narrows the runtime gap.

**Option C:** Drop the RAM column and add a "Browser Engine" column:
- AgentMux: Native OS webview or Bundled Chromium (your choice)
- Cursor/Warp/Electron tools: Bundled Chromium (no choice)

This reframes CEF as a **feature** (flexibility) rather than a catch-up to Electron.

### Hero / Why Now sections

Add messaging around the dual-host angle: "The only AI terminal that lets you choose
between native webview performance and full Chromium dev tools."

### New landing section candidate: "Two Builds, One Tool"

```
Tauri build (default)          CEF build (developer)
─────────────────────          ─────────────────────
~20 MB installer               ~400 MB installer
35-125 MB RAM                  ~300 MB RAM
Native OS webview              Chromium 146
Ships today on all platforms   Windows (macOS/Linux coming)
                               Remote DevTools on :9222
                               Consistent rendering everywhere
                               Full Web API surface
```

---

## Recommended Actions

### Immediate (before PR #253 merges)

1. **Don't update the landing page memory numbers yet.** The CEF build is Windows-only
   and in Phase 3 of 4. Tauri remains the default download for all platforms.

2. **Update the ComparePreview footnote** to acknowledge that CEF builds will be ~300 MB
   once they ship, but frame it as an opt-in power mode.

3. **Add a "CEF" row or note to the comparison sources document** so the numbers stay
   defensible.

### After Phase 4 completes (multi-window + cross-window drag)

4. **Ship CEF as a separate download** — `AgentMux-vX.X.X-cef-x64-setup.exe` alongside
   the Tauri installer. Let users self-select.

5. **Rewrite the comparison table** to lead with orchestration features, not memory.
   Memory is still a good secondary point for the Tauri build but it shouldn't be the
   headline claim against tools whose primary value is code editing, not orchestration.

6. **Update landing page hero** to reflect dual-host as a choice, not a compromise.

### Long term

7. **CEF macOS/Linux port.** Until then, Tauri is not optional.

8. **Consider making CEF the default on Windows** once Phase 4 is complete. The DevTools
   story is genuinely compelling for the developer audience, and Windows users already
   have a 300 MB WebView2 runtime installed separately anyway.

9. **Add in-app host switcher** — let users toggle between Tauri and CEF mode in settings,
   with a restart. This makes the dual-host story a product feature, not just a download
   decision.

---

## Summary

| Question | Answer |
|----------|--------|
| Does CEF break the memory value prop? | Partially — for CEF builds only; Tauri numbers stay true |
| Is it worth maintaining both? | Yes — CEF is Windows-only and Phase 4 isn't done |
| What's the new headline differentiator? | Orchestration + visibility, not memory size |
| Should landing page numbers change now? | No — Tauri is still the default everywhere |
| What's the dual-host pitch? | "Only tool that gives you the choice. Tauri for speed, CEF for power." |
| When does this become urgent to update? | When CEF ships on macOS/Linux with Phase 4 complete |
