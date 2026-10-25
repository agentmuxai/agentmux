@echo off
rem agentmux-build-slot-shim v3: written by AgentMux; edits here are overwritten.
rem cmd.exe and PowerShell reach the same build-slot shim as Git Bash: the sibling file with
rem this file's name and no extension (cargo, task, npm, npx, docker) does the work. Without
rem Git for Windows, or with AGENTMUX_BUILD_SLOT_OFF set, this runs the real tool directly.
set "SLOT_BASH=%ProgramFiles%\Git\bin\bash.exe"
if defined AGENTMUX_BUILD_SLOT_OFF goto direct
if not exist "%SLOT_BASH%" goto direct
"%SLOT_BASH%" "%~dp0%~n0" %*
exit /b %errorlevel%
:direct
set "SLOT_REAL="
rem Skip this directory's own copy: the first match that is not %~dp0.
for /f "usebackq delims=" %%i in (`where %~n0.exe %~n0.cmd %~n0.bat 2^>nul`) do call :pick "%%i"
if not defined SLOT_REAL (
  echo %~n0: not found on PATH 1>&2
  exit /b 127
)
"%SLOT_REAL%" %*
exit /b %errorlevel%
:pick
if defined SLOT_REAL exit /b 0
if /i "%~dp1"=="%~dp0" exit /b 0
set "SLOT_REAL=%~1"
exit /b 0
