// AgentMux Launcher - Shows splash immediately and launches main executable
// Compile: cl /O2 /MT /EHsc launcher.cpp user32.lib gdi32.lib /Fe:AgentMux.exe
// Or with MinGW: g++ -O2 -static -mwindows launcher.cpp -o AgentMux.exe -lgdi32

#include <windows.h>
#include <string>

#define SPLASH_WIDTH 400
#define SPLASH_HEIGHT 400
#define MAIN_EXE_NAME L"agentmux-main.exe"
#define SPLASH_SIGNAL_MUTEX L"Global\\AgentMuxSplashReady"

HWND g_splashWindow = NULL;
HBITMAP g_splashBitmap = NULL;

// Load BMP from file next to launcher
HBITMAP LoadSplashBitmap() {
    wchar_t exePath[MAX_PATH];
    GetModuleFileNameW(NULL, exePath, MAX_PATH);

    // Get directory of launcher
    wchar_t* lastSlash = wcsrchr(exePath, L'\\');
    if (lastSlash) {
        *(lastSlash + 1) = L'\0';
    }

    // Construct path to splash.bmp
    wchar_t bmpPath[MAX_PATH];
    wcscpy_s(bmpPath, exePath);
    wcscat_s(bmpPath, L"splash.bmp");

    return (HBITMAP)LoadImageW(NULL, bmpPath, IMAGE_BITMAP, 0, 0, LR_LOADFROMFILE);
}

// Create and show splash window
bool ShowSplashWindow(HINSTANCE hInstance) {
    // Load splash bitmap
    g_splashBitmap = LoadSplashBitmap();
    if (!g_splashBitmap) {
        MessageBoxW(NULL, L"Failed to load splash.bmp", L"Error", MB_ICONERROR);
        return false;
    }

    // Get screen dimensions for centering
    int screenWidth = GetSystemMetrics(SM_CXSCREEN);
    int screenHeight = GetSystemMetrics(SM_CYSCREEN);
    int x = (screenWidth - SPLASH_WIDTH) / 2;
    int y = (screenHeight - SPLASH_HEIGHT) / 2;

    // Create splash window (STATIC control with bitmap)
    g_splashWindow = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        L"STATIC",
        L"AgentMux Loading",
        WS_POPUP | SS_BITMAP,
        x, y,
        SPLASH_WIDTH, SPLASH_HEIGHT,
        NULL, NULL, hInstance, NULL
    );

    if (!g_splashWindow) {
        return false;
    }

    // Set bitmap on static control
    SendMessageW(g_splashWindow, STM_SETIMAGE, IMAGE_BITMAP, (LPARAM)g_splashBitmap);

    // Show window
    ShowWindow(g_splashWindow, SW_SHOW);
    UpdateWindow(g_splashWindow);

    return true;
}

// Launch main executable
PROCESS_INFORMATION LaunchMainExecutable() {
    wchar_t exePath[MAX_PATH];
    GetModuleFileNameW(NULL, exePath, MAX_PATH);

    // Get directory of launcher
    wchar_t* lastSlash = wcsrchr(exePath, L'\\');
    if (lastSlash) {
        *(lastSlash + 1) = L'\0';
    }

    // Construct path to main executable
    wchar_t mainExePath[MAX_PATH];
    wcscpy_s(mainExePath, exePath);
    wcscat_s(mainExePath, MAIN_EXE_NAME);

    // Launch main executable
    STARTUPINFOW si = { sizeof(si) };
    PROCESS_INFORMATION pi = { 0 };

    if (!CreateProcessW(
        mainExePath,
        NULL,
        NULL, NULL,
        FALSE,
        0,
        NULL, NULL,
        &si, &pi
    )) {
        MessageBoxW(NULL, L"Failed to launch agentmux-main.exe", L"Error", MB_ICONERROR);
    }

    return pi;
}

// Check if main executable is ready (via named mutex)
bool CheckMainReady() {
    HANDLE hMutex = OpenMutexW(SYNCHRONIZE, FALSE, SPLASH_SIGNAL_MUTEX);
    if (hMutex) {
        CloseHandle(hMutex);
        return true;
    }
    return false;
}

int WINAPI WinMain(HINSTANCE hInstance, HINSTANCE, LPSTR, int) {
    // Show splash immediately
    if (!ShowSplashWindow(hInstance)) {
        return 1;
    }

    // Launch main executable
    PROCESS_INFORMATION pi = LaunchMainExecutable();
    if (!pi.hProcess) {
        DestroyWindow(g_splashWindow);
        return 1;
    }

    // Wait for main executable to signal ready or timeout after 10 seconds
    DWORD startTime = GetTickCount();
    bool mainReady = false;

    while (!mainReady) {
        // Process Windows messages to keep splash responsive
        MSG msg;
        while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Check if main is ready
        if (CheckMainReady()) {
            mainReady = true;
            break;
        }

        // Check timeout (10 seconds)
        if (GetTickCount() - startTime > 10000) {
            break;
        }

        // Check if main process crashed
        DWORD exitCode;
        if (GetExitCodeProcess(pi.hProcess, &exitCode) && exitCode != STILL_ACTIVE) {
            MessageBoxW(NULL, L"AgentMux crashed during startup", L"Error", MB_ICONERROR);
            break;
        }

        Sleep(50);
    }

    // Close splash window
    if (g_splashWindow) {
        DestroyWindow(g_splashWindow);
    }
    if (g_splashBitmap) {
        DeleteObject(g_splashBitmap);
    }

    // Cleanup process handles
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);

    return 0;
}
