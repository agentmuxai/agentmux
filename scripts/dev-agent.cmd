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

:: Call task.exe by explicit extension (Gap A fix — no bash lookup needed here).
:: task is installed as task.exe (via go install / WinGet), not task.cmd.
task.exe dev %*
