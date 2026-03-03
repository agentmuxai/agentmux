Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinClearConsole {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc e, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int c);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(int flags, int x, int y, int data, int info);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    public const int LEFT_DOWN = 0x0002;
    public const int LEFT_UP   = 0x0004;
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(80);
        mouse_event(LEFT_DOWN, 0, 0, 0, 0);
        System.Threading.Thread.Sleep(50);
        mouse_event(LEFT_UP, 0, 0, 0, 0);
    }
}
[System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)]
public struct RECT { public int Left, Top, Right, Bottom; }
"@

Add-Type -AssemblyName System.Windows.Forms

$dtHandle = [IntPtr]::Zero
[WinClearConsole]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinClearConsole]::GetWindowText($hWnd, $sb, 256) | Out-Null
    if ($sb.ToString() -match "DevTools - tauri") {
        $script:dtHandle = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:dtHandle -ne [IntPtr]::Zero) {
    [WinClearConsole]::ShowWindow($script:dtHandle, 9)
    [WinClearConsole]::SetForegroundWindow($script:dtHandle)
    Start-Sleep -Milliseconds 600

    $rect = New-Object RECT
    [WinClearConsole]::GetWindowRect($script:dtHandle, [ref]$rect) | Out-Null

    # Click in the console input prompt area (bottom of DevTools ~50px from bottom)
    $cx = $rect.Left + 200
    $cy = $rect.Bottom - 30
    Write-Host "Clicking console input at: $cx, $cy"
    [WinClearConsole]::Click($cx, $cy)
    Start-Sleep -Milliseconds 400

    # Type console.clear() and press Enter
    [System.Windows.Forms.SendKeys]::SendWait("console.clear()")
    Start-Sleep -Milliseconds 200
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
    Start-Sleep -Milliseconds 400
    Write-Host "Cleared console"
} else {
    Write-Host "DevTools not found"
}
