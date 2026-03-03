Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinClickArea {
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
[WinClickArea]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinClickArea]::GetWindowText($hWnd, $sb, 256) | Out-Null
    if ($sb.ToString() -match "DevTools - tauri") {
        $script:dtHandle = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:dtHandle -ne [IntPtr]::Zero) {
    [WinClickArea]::ShowWindow($script:dtHandle, 9)
    [WinClickArea]::SetForegroundWindow($script:dtHandle)
    Start-Sleep -Milliseconds 800

    $rect = New-Object RECT
    [WinClickArea]::GetWindowRect($script:dtHandle, [ref]$rect) | Out-Null
    Write-Host "DevTools at: L=$($rect.Left) T=$($rect.Top) R=$($rect.Right) B=$($rect.Bottom)"

    # Click in the middle of the console output area
    $cx = $rect.Left + 320
    $cy = $rect.Top + 400
    Write-Host "Clicking console output area at: $cx, $cy"
    [WinClickArea]::Click($cx, $cy)
    Start-Sleep -Milliseconds 400

    # Now Ctrl+End to scroll to bottom of console output
    [System.Windows.Forms.SendKeys]::SendWait("^{END}")
    Start-Sleep -Milliseconds 400
    Write-Host "Scrolled to bottom of console"
} else {
    Write-Host "DevTools not found"
}
