@echo off
rem agentmux-build-slot-shim v1: written by AgentMux; edits here are overwritten.
rem cmd.exe and PowerShell reach the same build-slot shim as Git Bash: the sibling
rem file "cargo" does the work. Without Git for Windows, or with
rem AGENTMUX_BUILD_SLOT_OFF set, this runs the real cargo.exe directly.
set "SLOT_BASH=%ProgramFiles%\Git\bin\bash.exe"
if defined AGENTMUX_BUILD_SLOT_OFF goto direct
if not exist "%SLOT_BASH%" goto direct
"%SLOT_BASH%" "%~dp0cargo" %*
exit /b %errorlevel%
:direct
set "SLOT_REAL="
for /f "usebackq delims=" %%i in (`where cargo.exe 2^>nul`) do if not defined SLOT_REAL set "SLOT_REAL=%%i"
if not defined SLOT_REAL (
  echo cargo: cargo.exe not found on PATH 1>&2
  exit /b 127
)
"%SLOT_REAL%" %*
exit /b %errorlevel%
