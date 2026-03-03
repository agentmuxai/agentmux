Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinReload {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc e, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int c);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
}
"@

Add-Type -AssemblyName System.Windows.Forms

# Find the AgentMux app window (not DevTools)
$appHandle = [IntPtr]::Zero
[WinReload]::EnumWindows({
    param($hWnd, $lParam)
    $sb = New-Object System.Text.StringBuilder(256)
    [WinReload]::GetWindowText($hWnd, $sb, 256) | Out-Null
    $title = $sb.ToString()
    if ($title -match "AgentMux" -and $title -notmatch "DevTools") {
        Write-Host "Found app: '$title'"
        $script:appHandle = $hWnd
    }
    return $true
}, [IntPtr]::Zero) | Out-Null

if ($script:appHandle -ne [IntPtr]::Zero) {
    [WinReload]::ShowWindow($script:appHandle, 9)
    [WinReload]::SetForegroundWindow($script:appHandle)
    Start-Sleep -Milliseconds 800
    # Ctrl+R to reload
    [System.Windows.Forms.SendKeys]::SendWait("^r")
    Start-Sleep -Milliseconds 500
    Write-Host "Sent Ctrl+R to reload app"
} else {
    Write-Host "AgentMux app window not found"
}
