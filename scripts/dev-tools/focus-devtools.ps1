Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WinFinder {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc e, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int c);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    public static readonly IntPtr HWND_TOP = IntPtr.Zero;
    public const uint SWP_NOMOVE = 0x0002;
    public const uint SWP_NOSIZE = 0x0001;
    public const uint SWP_SHOWWINDOW = 0x0040;
}
"@

$found = $null
[WinFinder]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinFinder]::GetWindowText($hWnd, $sb, 256) | Out-Null
    $title = $sb.ToString()
    if ($title -match "DevTools|Console Errors|Inspector") {
        Write-Host "Found: '$title' handle=$hWnd"
        $script:found = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:found) {
    [WinFinder]::ShowWindow($script:found, 9)  # SW_RESTORE
    [WinFinder]::SetForegroundWindow($script:found)
    Write-Host "Focused DevTools window"
} else {
    Write-Host "DevTools window not found - listing all visible windows:"
    [WinFinder]::EnumWindows({
        param($hWnd, $lParam)
        if ([WinFinder]::IsWindowVisible($hWnd)) {
            $sb = New-Object System.Text.StringBuilder(256)
            [WinFinder]::GetWindowText($hWnd, $sb, 256) | Out-Null
            $t = $sb.ToString()
            if ($t.Length -gt 0) { Write-Host "  '$t'" }
        }
        return $true
    }, [IntPtr]::Zero) | Out-Null
}
