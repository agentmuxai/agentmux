@echo off
REM Build launcher with MSVC (preferred)
where cl >nul 2>nul
if %ERRORLEVEL% EQU 0 (
    echo Building with MSVC...
    cl /O2 /MT /EHsc launcher.cpp user32.lib gdi32.lib /Fe:AgentMux.exe
    goto :done
)

REM Try MinGW as fallback
where g++ >nul 2>nul
if %ERRORLEVEL% EQU 0 (
    echo Building with MinGW...
    g++ -O2 -static -mwindows launcher.cpp -o AgentMux.exe -lgdi32 -luser32
    goto :done
)

echo ERROR: No C++ compiler found. Install MSVC or MinGW.
exit /b 1

:done
echo.
echo Launcher built successfully: AgentMux.exe
echo Copy this launcher and splash.bmp alongside agentmux.exe (renamed to agentmux-main.exe)
