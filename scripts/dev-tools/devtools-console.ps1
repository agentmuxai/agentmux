Add-Type -AssemblyName System.Windows.Forms

# Find DevTools window - it may be a separate window or embedded
# Try Ctrl+` to switch to console in devtools, or just click the console tab
# First make sure the devtools window is focused

# Get all windows
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinHelper {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc enumProc, IntPtr lParam);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
}
"@

# Focus agentmux and send Ctrl+Shift+I to ensure devtools, then Ctrl+` for console
$app = Get-Process -Name 'agentmux' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($app) {
    [WinHelper]::SetForegroundWindow($app.MainWindowHandle)
    Start-Sleep -Milliseconds 500
    # Send Escape first to clear any popups
    [System.Windows.Forms.SendKeys]::SendWait("{ESC}")
    Start-Sleep -Milliseconds 200
    # Ctrl+Shift+J opens console directly in Chrome-based devtools
    [System.Windows.Forms.SendKeys]::SendWait("^+j")
    Start-Sleep -Milliseconds 1000
    Write-Host "Sent Ctrl+Shift+J (console shortcut)"
}
