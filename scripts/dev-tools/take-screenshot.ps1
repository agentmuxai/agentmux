Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap(1600, 900)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
$bmp.Save("$PSScriptRoot\..\..\screenshot-rustbackend.png")
$g.Dispose()
$bmp.Dispose()
Write-Host "Screenshot saved"
