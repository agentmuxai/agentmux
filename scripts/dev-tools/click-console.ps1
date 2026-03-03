Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinMouse {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(int flags, int x, int y, int data, int info);
    public const int LEFT_DOWN = 0x0002;
    public const int LEFT_UP   = 0x0004;
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(100);
        mouse_event(LEFT_DOWN, x, y, 0, 0);
        mouse_event(LEFT_UP, x, y, 0, 0);
    }
}
"@

$app = Get-Process -Name 'agentmux' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($app) {
    [WinMouse]::SetForegroundWindow($app.MainWindowHandle)
    Start-Sleep -Milliseconds 600
}

# Click the Console tab in DevTools (approximately x=75, y=232 based on screenshot)
[WinMouse]::Click(75, 232)
Start-Sleep -Milliseconds 500
Write-Host "Clicked Console tab"
