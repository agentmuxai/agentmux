Add-Type -AssemblyName System.Windows.Forms

$app = Get-Process -Name 'agentmux' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $app) { Write-Host "agentmux not found"; exit 1 }

Write-Host "Found agentmux PID: $($app.Id)"

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@

[Win32]::ShowWindow($app.MainWindowHandle, 9)
[Win32]::SetForegroundWindow($app.MainWindowHandle)
Start-Sleep -Milliseconds 800

# Open DevTools with F12
[System.Windows.Forms.SendKeys]::SendWait("{F12}")
Write-Host "Sent F12"
Start-Sleep -Milliseconds 2000
Write-Host "Done"
