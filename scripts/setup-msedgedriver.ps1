# Script to download msedgedriver.exe for WebdriverIO tests
# Run this before running e2e tests: npm run test:e2e

$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "AgentMux msedgedriver Setup" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Get Edge version
Write-Host "Detecting Microsoft Edge version..." -ForegroundColor Yellow
$edgeVersion = $null

try {
    # Try to get version from registry
    $edgeVersion = (Get-ItemProperty -Path "HKCU:\Software\Microsoft\Edge\BLBeacon" -ErrorAction SilentlyContinue).version

    if (-not $edgeVersion) {
        # Try program files path
        $edgePath = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
        if (Test-Path $edgePath) {
            $edgeVersion = (Get-Item $edgePath).VersionInfo.ProductVersion
        }
    }
} catch {
    Write-Host "Could not detect Edge version automatically." -ForegroundColor Red
}

if ($edgeVersion) {
    Write-Host "Detected Edge version: $edgeVersion" -ForegroundColor Green
    $majorVersion = $edgeVersion.Split('.')[0]
} else {
    Write-Host "Could not auto-detect Edge version." -ForegroundColor Yellow
    Write-Host "Please enter your Edge version (e.g., 131):" -ForegroundColor Yellow
    $majorVersion = Read-Host
}

Write-Host ""
Write-Host "Downloading msedgedriver for Edge $majorVersion..." -ForegroundColor Yellow

# Construct download URL
$downloadUrl = "https://msedgedriver.azureedge.net/${majorVersion}.0.0.0/edgedriver_win64.zip"
$zipPath = ".\msedgedriver.zip"
$extractPath = "."

Write-Host "URL: $downloadUrl" -ForegroundColor Gray

try {
    # Download the zip
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
    Write-Host "✓ Downloaded" -ForegroundColor Green

    # Extract
    Write-Host "Extracting..." -ForegroundColor Yellow
    Expand-Archive -Path $zipPath -DestinationPath $extractPath -Force

    # Check if extracted successfully
    if (Test-Path ".\msedgedriver.exe") {
        Write-Host "✓ msedgedriver.exe extracted to project root" -ForegroundColor Green

        # Clean up
        Remove-Item $zipPath -Force
        Write-Host "✓ Cleaned up zip file" -ForegroundColor Green

        Write-Host ""
        Write-Host "========================================" -ForegroundColor Green
        Write-Host "✓ Setup complete!" -ForegroundColor Green
        Write-Host "========================================" -ForegroundColor Green
        Write-Host ""
        Write-Host "You can now run: npm run test:e2e" -ForegroundColor Cyan
    } else {
        Write-Host "✗ Failed to extract msedgedriver.exe" -ForegroundColor Red
        throw "Extraction failed"
    }
} catch {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Red
    Write-Host "✗ Setup failed" -ForegroundColor Red
    Write-Host "========================================" -ForegroundColor Red
    Write-Host ""
    Write-Host "Error: $_" -ForegroundColor Red
    Write-Host ""
    Write-Host "Manual download instructions:" -ForegroundColor Yellow
    Write-Host "1. Go to: https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/" -ForegroundColor Gray
    Write-Host "2. Download the driver for your Edge version ($majorVersion)" -ForegroundColor Gray
    Write-Host "3. Extract msedgedriver.exe to: $(Get-Location)" -ForegroundColor Gray
    exit 1
}
