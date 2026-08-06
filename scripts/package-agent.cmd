@echo off
:: Bridge script for agent / MCP-Shell invocation of `task package` on Windows.
:: Same Gap A + Gap B fixes as dev-agent.cmd — see that file for full explanation.
:: Documented usage: CLAUDE.md's "Launching `task package` from an agent /
:: MCP Shell (Windows)" section. Full investigation of why this script is
:: required (not optional) for this specific task:
:: docs/retro/retro-task-package-mcp-timeout-and-shell-output-gap-2026-08-06.md

cd /d "%~dp0.."

set "GIT_BIN=C:\Program Files\Git\bin"
set "GIT_USR=C:\Program Files\Git\usr\bin"
set "PATH=%GIT_BIN%;%GIT_USR%;%PATH%"

:: Call bare `task` — PATHEXT resolves task.exe or task.cmd depending on how
:: it was installed on the machine (see dev-agent.cmd for the full rationale).
:: Merge stderr into stdout so shell viewers don't color build progress red
:: (cargo writes all output to stderr, not stdout).
task package %* 2>&1
