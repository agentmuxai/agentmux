#!/usr/bin/env pwsh
# AgentMux Performance Benchmarking Script
# Measures startup time, memory usage, and bundle size
# Usage: ./measure-performance.ps1 [-Runs 10] [-OutputJson]

param(
    [int]$Runs = 5,
    [switch]$OutputJson,
    [string]$AppPath = "src-tauri\target\release\AgentMux.exe"
)

$ErrorActionPreference = "Stop"

function Write-Header {
    param([string]$Text)
    Write-Host "`n========================================" -ForegroundColor Cyan
    Write-Host "  $Text" -ForegroundColor Cyan
    Write-Host "========================================`n" -ForegroundColor Cyan
}

function Measure-StartupTime {
    param([string]$ExePath, [int]$Iterations)

    Write-Header "Startup Time Measurement"
    Write-Host "Running $Iterations iterations..." -ForegroundColor Yellow

    $times = @()

    for ($i = 1; $i -le $Iterations; $i++) {
        Write-Host "Run $i/$Iterations..." -NoNewline

        $sw = [System.Diagnostics.Stopwatch]::StartNew()

        # Start process and measure until main window appears
        $proc = Start-Process -FilePath $ExePath -PassThru

        # Wait for window to be ready (look for any window with a title)
        $timeout = 10
        $elapsed = 0
        $windowReady = $false

        while ($elapsed -lt $timeout -and -not $windowReady) {
            Start-Sleep -Milliseconds 100
            $elapsed += 0.1

            # Check if process still exists
            if (-not (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)) {
                Write-Host " Process exited!" -ForegroundColor Red
                break
            }

            # Check if main window exists (any non-empty title)
            $process = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
            if ($process -and $process.MainWindowHandle -ne 0) {
                $windowReady = $true
            }
        }

        $sw.Stop()
        $startupTime = $sw.Elapsed.TotalMilliseconds

        # Kill the process and any child processes
        if (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue) {
            # Kill child processes (backend)
            Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $proc.Id } | ForEach-Object {
                Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
            }
            # Kill main process
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        }
        Start-Sleep -Milliseconds 500

        $times += $startupTime
        Write-Host " $([math]::Round($startupTime, 2))ms" -ForegroundColor Green
    }

    $avg = ($times | Measure-Object -Average).Average
    $min = ($times | Measure-Object -Minimum).Minimum
    $max = ($times | Measure-Object -Maximum).Maximum
    $median = ($times | Sort-Object)[[math]::Floor($times.Count / 2)]

    Write-Host "`nResults:" -ForegroundColor Cyan
    Write-Host "  Average: $([math]::Round($avg, 2))ms" -ForegroundColor White
    Write-Host "  Median:  $([math]::Round($median, 2))ms" -ForegroundColor White
    Write-Host "  Min:     $([math]::Round($min, 2))ms" -ForegroundColor Green
    Write-Host "  Max:     $([math]::Round($max, 2))ms" -ForegroundColor Yellow

    return @{
        Average = $avg
        Median = $median
        Min = $min
        Max = $max
        Runs = $times
    }
}

function Measure-MemoryUsage {
    param([string]$ExePath)

    Write-Header "Memory Usage Measurement"

    Write-Host "Starting application..." -ForegroundColor Yellow
    $proc = Start-Process -FilePath $ExePath -PassThru

    # Wait for startup
    Start-Sleep -Seconds 5

    # Idle memory
    $idleMemory = (Get-Process -Id $proc.Id).WorkingSet64 / 1MB
    Write-Host "Idle Memory: $([math]::Round($idleMemory, 2)) MB" -ForegroundColor Green

    # Wait a bit more for full initialization
    Start-Sleep -Seconds 5

    # Measure again after full init
    $initMemory = (Get-Process -Id $proc.Id).WorkingSet64 / 1MB
    Write-Host "After Init:  $([math]::Round($initMemory, 2)) MB" -ForegroundColor Green

    # Clean up
    Stop-Process -Id $proc.Id -Force

    return @{
        IdleMB = $idleMemory
        AfterInitMB = $initMemory
    }
}

function Measure-BundleSize {
    Write-Header "Bundle Size Measurement"

    # Tauri release bundle
    $tauriExe = "src-tauri\target\release\AgentMux.exe"
    $tauriBundle = "src-tauri\target\release\bundle"

    if (Test-Path $tauriExe) {
        $exeSize = (Get-Item $tauriExe).Length / 1MB
        Write-Host "AgentMux.exe:      $([math]::Round($exeSize, 2)) MB" -ForegroundColor Green
    }

    # Check for installer
    $msiFiles = Get-ChildItem -Path $tauriBundle -Filter "*.msi" -Recurse -ErrorAction SilentlyContinue
    if ($msiFiles) {
        $installerSize = ($msiFiles | Select-Object -First 1).Length / 1MB
        Write-Host "Installer (.msi): $([math]::Round($installerSize, 2)) MB" -ForegroundColor Green
    }

    # Check for NSIS installer
    $nsisFiles = Get-ChildItem -Path $tauriBundle -Filter "*.exe" -Recurse -ErrorAction SilentlyContinue |
                 Where-Object { $_.Name -match "setup" }
    if ($nsisFiles) {
        $nsisSize = ($nsisFiles | Select-Object -First 1).Length / 1MB
        Write-Host "Installer (.exe): $([math]::Round($nsisSize, 2)) MB" -ForegroundColor Green
    }

    # Compare with dist/ if it exists (Electron artifacts)
    $electronZip = Get-ChildItem -Path "dist" -Filter "Wave-*.zip" -ErrorAction SilentlyContinue |
                   Select-Object -First 1
    if ($electronZip) {
        $electronSize = $electronZip.Length / 1MB
        Write-Host "`nElectron Package: $([math]::Round($electronSize, 2)) MB" -ForegroundColor Yellow

        if ($msiFiles -or $nsisFiles) {
            $tauriSize = if ($msiFiles) { $installerSize } else { $nsisSize }
            $reduction = (($electronSize - $tauriSize) / $electronSize) * 100
            Write-Host "Size Reduction:   $([math]::Round($reduction, 1))%" -ForegroundColor Cyan
        }
    }

    return @{
        ExeMB = if (Test-Path $tauriExe) { $exeSize } else { 0 }
        InstallerMB = if ($msiFiles) { $installerSize } elseif ($nsisFiles) { $nsisSize } else { 0 }
    }
}

# Main execution
Write-Host @"

 ██╗    ██╗ █████╗ ██╗   ██╗███████╗███╗   ███╗██╗   ██╗██╗  ██╗
 ██║    ██║██╔══██╗██║   ██║██╔════╝████╗ ████║██║   ██║╚██╗██╔╝
 ██║ █╗ ██║███████║██║   ██║█████╗  ██╔████╔██║██║   ██║ ╚███╔╝
 ██║███╗██║██╔══██║╚██╗ ██╔╝██╔══╝  ██║╚██╔╝██║██║   ██║ ██╔██╗
 ╚███╔███╔╝██║  ██║ ╚████╔╝ ███████╗██║ ╚═╝ ██║╚██████╔╝██╔╝ ██╗
  ╚══╝╚══╝ ╚═╝  ╚═╝  ╚═══╝  ╚══════╝╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝

           Performance Benchmarking Tool v1.0
"@

$results = @{}

# Check if app exists
if (-not (Test-Path $AppPath)) {
    Write-Host "`nERROR: Application not found at: $AppPath" -ForegroundColor Red
    Write-Host "Build the release version first: task package" -ForegroundColor Yellow
    exit 1
}

# Run benchmarks
try {
    $results.Startup = Measure-StartupTime -ExePath $AppPath -Iterations $Runs
    $results.Memory = Measure-MemoryUsage -ExePath $AppPath
    $results.BundleSize = Measure-BundleSize

    Write-Header "Summary"
    Write-Host "Startup Time (avg):  $([math]::Round($results.Startup.Average, 2))ms" -ForegroundColor Cyan
    Write-Host "Memory Usage (idle): $([math]::Round($results.Memory.IdleMB, 2)) MB" -ForegroundColor Cyan
    Write-Host "Executable Size:     $([math]::Round($results.BundleSize.ExeMB, 2)) MB" -ForegroundColor Cyan

    # Save JSON if requested
    if ($OutputJson) {
        $jsonPath = "benchmark-results.json"
        $results | ConvertTo-Json -Depth 10 | Out-File -FilePath $jsonPath -Encoding UTF8
        Write-Host "`nResults saved to: $jsonPath" -ForegroundColor Green
    }

} catch {
    Write-Host "`nERROR: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

Write-Host ""
