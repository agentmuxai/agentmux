Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinClick {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc e, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int c);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(int flags, int x, int y, int data, int info);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
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
[StructLayout(LayoutKind.Sequential)]
public struct RECT { public int Left, Top, Right, Bottom; }
"@

# Find DevTools window
$dtHandle = [IntPtr]::Zero
[WinClick]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinClick]::GetWindowText($hWnd, $sb, 256) | Out-Null
    if ($sb.ToString() -match "DevTools - tauri") {
        $script:dtHandle = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:dtHandle -eq [IntPtr]::Zero) {
    Write-Host "DevTools not found"; exit 1
}

# Get DevTools window position
$rect = New-Object RECT
[WinClick]::GetWindowRect($script:dtHandle, [ref]$rect) | Out-Null
Write-Host "DevTools at: L=$($rect.Left) T=$($rect.Top) R=$($rect.Right) B=$($rect.Bottom)"

# Focus it
[WinClick]::ShowWindow($script:dtHandle, 9)
[WinClick]::SetForegroundWindow($script:dtHandle)
Start-Sleep -Milliseconds 500

# The DevTools tab bar is roughly 50px from the top of the window content area
# (after title bar ~30px + devtools header ~22px)
# Console tab is roughly the 2nd tab, about 75px from left of window
$tabY = $rect.Top + 52
$consoleX = $rect.Left + 75
Write-Host "Clicking Console tab at: $consoleX, $tabY"
[WinClick]::Click($consoleX, $tabY)
Start-Sleep -Milliseconds 800
Write-Host "Done"
