#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Generate the MSIX Store logo asset set from the app icon.
.DESCRIPTION
  Downscales assets/linux/icons/hicolor/512x512/apps/agentmux.png into the
  Square/Wide/Store logos referenced by packaging/msix/AppxManifest.xml.template,
  writing them to packaging/msix/assets/. Committed PNGs keep `task package:msix`
  reproducible without an image toolchain on the build host. Re-run this only when
  the source icon changes. Uses System.Drawing (ships with Windows) — no ImageMagick.
#>
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$root   = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)  # repo root
$src    = Join-Path $root "assets\linux\icons\hicolor\512x512\apps\agentmux.png"
$outDir = Join-Path $PSScriptRoot "assets"
if (-not (Test-Path $src)) { throw "Source icon not found: $src" }
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

function New-Canvas([int]$w, [int]$h) {
    $bmp = New-Object System.Drawing.Bitmap($w, $h, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode     = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode   = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    @{ bmp = $bmp; g = $g }
}

function Save-Logo([string]$name, [int]$w, [int]$h) {
    $img = [System.Drawing.Image]::FromFile($src)
    $c = New-Canvas $w $h
    # Center a square render of the (square) icon; for square tiles this fills, for wide it letterboxes.
    $side = [Math]::Min($w, $h)
    $x = [int](($w - $side) / 2)
    $y = [int](($h - $side) / 2)
    $c.g.DrawImage($img, $x, $y, $side, $side)
    $c.g.Dispose()
    $dst = Join-Path $outDir $name
    $c.bmp.Save($dst, [System.Drawing.Imaging.ImageFormat]::Png)
    $c.bmp.Dispose(); $img.Dispose()
    Write-Host ("  + {0,-22} {1}x{2}" -f $name, $w, $h)
}

Write-Host "Generating MSIX assets from $src ->"
Save-Logo "StoreLogo.png"        50  50
Save-Logo "Square44x44Logo.png"  44  44
Save-Logo "Square71x71Logo.png"  71  71
Save-Logo "Square150x150Logo.png" 150 150
Save-Logo "Square310x310Logo.png" 310 310
Save-Logo "Wide310x150Logo.png"  310 150
Write-Host "Done -> $outDir"
