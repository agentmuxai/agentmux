param([string]$Path = "$PSScriptRoot\..\..\ss-crop.png", [int]$X=0, [int]$Y=0, [int]$W=700, [int]$H=800)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$full = New-Object System.Drawing.Bitmap(1920, 1080)
$g = [System.Drawing.Graphics]::FromImage($full)
$g.CopyFromScreen(0, 0, 0, 0, $full.Size)
$g.Dispose()
$W2 = $W * 2
$H2 = $H * 2
$scaled = New-Object System.Drawing.Bitmap($W2, $H2)
$gs = [System.Drawing.Graphics]::FromImage($scaled)
$gs.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$src = New-Object System.Drawing.Rectangle($X, $Y, $W, $H)
$dst = New-Object System.Drawing.Rectangle(0, 0, $W2, $H2)
$gs.DrawImage($full, $dst, $src, [System.Drawing.GraphicsUnit]::Pixel)
$gs.Dispose()
$full.Dispose()
$scaled.Save($Path)
$scaled.Dispose()
Write-Host "Saved: $Path"
