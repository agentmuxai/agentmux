Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinScroll {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc e, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int c);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
}
"@

Add-Type -AssemblyName System.Windows.Forms

$dtHandle = [IntPtr]::Zero
[WinScroll]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinScroll]::GetWindowText($hWnd, $sb, 256) | Out-Null
    if ($sb.ToString() -match "DevTools - tauri") {
        $script:dtHandle = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:dtHandle -ne [IntPtr]::Zero) {
    [WinScroll]::ShowWindow($script:dtHandle, 9)
    [WinScroll]::SetForegroundWindow($script:dtHandle)
    Start-Sleep -Milliseconds 600
    [System.Windows.Forms.SendKeys]::SendWait("{END}")
    Start-Sleep -Milliseconds 400
    Write-Host "Scrolled to end in DevTools"
} else {
    Write-Host "DevTools not found"
}
