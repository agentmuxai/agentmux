param([string]$Path = "$PSScriptRoot\..\..\ss-out.png")
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$bmp = New-Object System.Drawing.Bitmap(1920, 1080)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
$bmp.Save($Path)
$g.Dispose()
$bmp.Dispose()
Write-Host "Saved: $Path"
