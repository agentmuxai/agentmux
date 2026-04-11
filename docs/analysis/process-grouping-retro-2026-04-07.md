# Process Grouping Retro — 2026-04-07

**Session:** AgentA  
**Trigger:** User reported "processes not grouped under AgentMux CEF in Task Manager"  
**Resolution:** False alarm — grouping was working correctly in v0.33.57

---

## What Happened

User reported that AgentMux processes were not grouped in Task Manager. The earlier analysis doc (`docs/analysis/process-tree-grouping.md`, written 2026-04-05) had already modelled this as a real bug caused by the launcher architecture.

I immediately moved to implement the recommended fix (Option A — DETACHED_PROCESS spawn in the launcher). Before the build completed, the user opened v0.33.57 and confirmed grouping was working fine.

Reverted the launcher change before it was committed or built.

---

## Root Cause of the False Alarm

The prior analysis doc (2026-04-05) described a real scenario where processes scatter, but got the Task Manager grouping mechanism wrong.

**What the doc assumed:**  
Task Manager groups sub-processes under their OS-level parent process (by PID parent-child relationship). Since the launcher has no visible window, it lands in "Background processes", and the CEF host (its child) would scatter too.

**What Task Manager actually does:**  
Task Manager groups sub-processes under the process that **owns visible top-level windows**, regardless of parent PID. Since `agentmux-cef.exe` creates the CEF windows, Task Manager treats it as the "App" owner. All of its child processes (renderer, GPU, utility, backend sidecar) nest under it correctly — even though the launcher is technically the OS-level parent and is sitting in "Background processes".

So the launcher + `.status()` (wait for exit) pattern is correct and does NOT break grouping. The launcher remains a background entry, the CEF host appears in "Apps", and everything groups properly.

---

## Why the Prior Analysis Was Wrong

The 2026-04-05 analysis was written during the launcher architecture investigation (PR #298). It was based on the theoretical behavior of Task Manager's parent-PID grouping. It was never validated empirically against a live build that showed the actual behavior. PR #302's test plan item "Process grouping under agentmux-cef.exe in Task Manager is preserved" was added based on the assumption that grouping was already broken and needed to be preserved — but the grouping was fine.

---

## Correct Mental Model

```
Task Manager (Processes tab)

[Apps]
  AgentMux CEF v0.33.57                    ← agentmux-cef-0.33.57.exe, owns CEF windows
    agentmux-cef-0.33.57.exe (renderer)    ← child of CEF host
    agentmux-cef-0.33.57.exe (gpu)         ← child of CEF host
    agentmux-cef-0.33.57.exe (utility)     ← child of CEF host
    agentmux-srv-0.33.57-windows.x64.exe   ← spawned by CEF host (Job Object tied)
      pwsh.exe                              ← shell pane 1
      pwsh.exe                              ← shell pane 2

[Background processes]
  agentmux.exe                             ← launcher, no window, waiting for CEF to exit
```

The launcher is an orphan entry in Background processes — that is expected and harmless.

---

## What Was NOT Reverted

The launcher source was edited and immediately reverted before any commit. No build was produced. The `docs/analysis/process-tree-grouping.md` file remains as historical context but its Option A recommendation is no longer needed and should not be acted on without fresh evidence that grouping is actually broken.

---

## Lesson

Before implementing a fix for a reported regression, verify the regression is actually present in the current build. A 30-second test run would have saved the investigation entirely.
