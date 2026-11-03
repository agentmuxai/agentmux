#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Superseded: the MSIX logos are drawn from the icon SVGs by `node scripts/gen-app-icons.mjs`.
.DESCRIPTION
  This used to downscale the 512 px PNG with System.Drawing. The icon is now flat rectangles on a pixel
  grid, drawn at every size from assets/icon/agentmux-dark.svg and agentmux-light.svg (which also gives the
  taskbar sets, `*_altform-unplated` and `*_altform-lightunplated`). See assets/icon/README.md.
#>
Write-Host "Superseded. Run: node scripts/gen-app-icons.mjs"
node (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "scripts/gen-app-icons.mjs")
