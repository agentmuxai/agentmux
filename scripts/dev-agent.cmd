@echo off
:: Bridge script for agent / MCP-Shell invocation of `task dev` on Windows.
::
:: Problem (two gaps — see docs/retro/retro-task-dev-agent-shell-path-2026-06-27.md):
::
::   Gap A: MSYS2 bash does not resolve .cmd files from bare command names,
::           so `bash -c "task dev"` exits with "command not found".
::
::   Gap B: go-task's Taskfile calls `bash -c '...'` for build steps
::           (build:host:windows, inject-exe-icon, repair-cef-extract).
::           When cmd.exe spawns these, bash.exe must be on the Windows PATH.
::           The registry PATH has Git\cmd (shims) but NOT Git\bin (bash.exe),
::           so go-task exits 200 / "The system cannot find the file specified".
::
:: This wrapper runs as a plain .cmd file (cmd.exe — no bash needed to start it)
:: and prepends Git\bin so bash.exe is findable for all Taskfile subprocesses.
:: Agents call it by full Windows path from mcp__agentmux__Shell; no go-task
:: indirection required for the outer invocation.
::
:: Usage (from mcp__agentmux__Shell):
::   cmd:  C:\<repo>\scripts\dev-agent.cmd
::   or:   C:\<repo>\scripts\dev-agent.cmd TITLE="zoom-fix: PR #1234"
::
:: If TITLE= isn't passed, it defaults to $AGENTMUX_AGENT_ID (the calling
:: agent's own identity, injected at spawn) so the OS taskbar shows whose
:: dev window is whose — see specs/SPEC_DEV_WINDOW_TITLE_ARG_2026_06_25.md.
::
:: On macOS / Linux `task dev` works directly — this script is Windows-only.

:: Change to the repo root (parent of scripts\) so go-task can find Taskfile.yml.
:: Required when the MCP Shell cwd is not the repo root (e.g. called by full path).
:: %~dp0 expands to the directory of this script (scripts\); .. is the repo root.
cd /d "%~dp0.."

:: Prepend Git\bin so bash.exe is on PATH for Taskfile subprocesses (Gap B fix).
:: Standard Git for Windows install location. If your Git is elsewhere, update:
set "GIT_BIN=C:\Program Files\Git\bin"
set "GIT_USR=C:\Program Files\Git\usr\bin"
set "PATH=%GIT_BIN%;%GIT_USR%;%PATH%"

:: Default the OS window title to this agent's own identity (AGENTMUX_AGENT_ID,
:: injected into every agent's shell at spawn — see CLAUDE.md) when the caller
:: didn't pass an explicit TITLE= override. Without this, every agent-launched
:: dev window shows as plain "AgentMux" in the taskbar, indistinguishable from
:: any other parallel dev session — this is the common case: an agent runs its
:: own `task dev` to let the human test a fix, and the human needs to tell
:: whose window is whose. An explicit TITLE= from the caller always wins.
set "ARGS=%*"
set "TITLE_ARG="
echo %ARGS% | findstr /C:"TITLE=" >nul
if errorlevel 1 (
    if not "%AGENTMUX_AGENT_ID%"=="" set "TITLE_ARG=TITLE=%AGENTMUX_AGENT_ID%"
)

:: Call bare `task` (no extension) — this .cmd wrapper itself runs under cmd.exe
:: (Gap A only applies to bash resolving .cmd files, not to us), so PATHEXT
:: resolution finds task.exe (go install / WinGet) OR task.cmd (npm global
:: install) transparently. Do NOT hardcode an extension — which one exists
:: depends on how `task` was installed on the machine.
:: Merge stderr into stdout so shell viewers don't color build progress red
:: (cargo writes all output to stderr, not stdout).
task dev %ARGS% %TITLE_ARG% 2>&1
