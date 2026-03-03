Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinFocus {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc e, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int c);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
}
"@

Add-Type -AssemblyName System.Windows.Forms

$dtHandle = [IntPtr]::Zero
[WinFocus]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinFocus]::GetWindowText($hWnd, $sb, 256) | Out-Null
    if ($sb.ToString() -match "DevTools - tauri") {
        $script:dtHandle = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:dtHandle -ne [IntPtr]::Zero) {
    [WinFocus]::ShowWindow($script:dtHandle, 9)
    [WinFocus]::SetForegroundWindow($script:dtHandle)
    Start-Sleep -Milliseconds 600
    # Escape any welcome screen, then Ctrl+Shift+J for Console
    [System.Windows.Forms.SendKeys]::SendWait("{ESC}")
    Start-Sleep -Milliseconds 300
    [System.Windows.Forms.SendKeys]::SendWait("^+j")
    Start-Sleep -Milliseconds 800
    Write-Host "Sent Ctrl+Shift+J to DevTools"
} else {
    Write-Host "DevTools window not found"
}
