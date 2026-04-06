#Requires -Version 5.1
# Install script for spacebot-homelab-mcp on Windows
# Usage: irm https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "Joshf225/spacebot-homelab-mcp"
$Binary = "spacebot-homelab-mcp"
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { "$env:LOCALAPPDATA\Programs\$Binary" }

function Get-LatestVersion {
    $url = "https://api.github.com/repos/$Repo/releases/latest"
    $release = Invoke-RestMethod -Uri $url -UseBasicParsing
    return $release.tag_name.TrimStart("v")
}

function Get-Platform {
    if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64" -or $env:PROCESSOR_ARCHITEW6432 -eq "ARM64") {
        throw "ARM64 Windows builds are not yet available."
    }

    $arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else {
        throw "32-bit Windows is not supported."
    }

    return "$arch-pc-windows-msvc"
}

$Version = if ($env:VERSION) { $env:VERSION } else {
    Write-Host "==> Fetching latest release..." -ForegroundColor Cyan
    Get-LatestVersion
}

if (-not $Version) {
    throw "Could not determine latest version. Set `$env:VERSION = 'x.y.z'` manually."
}

$Platform = Get-Platform
Write-Host "==> Installing $Binary v$Version for $Platform" -ForegroundColor Cyan

$Archive = "$Binary-$Version-$Platform.zip"
$Url = "https://github.com/$Repo/releases/download/v$Version/$Archive"
$TempDir = Join-Path $env:TEMP "spacebot-install-$(Get-Random)"

try {
    New-Item -ItemType Directory -Force $TempDir | Out-Null
    $ArchivePath = Join-Path $TempDir $Archive

    Write-Host "==> Downloading $Url..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $Url -OutFile $ArchivePath -UseBasicParsing

    Write-Host "==> Extracting..." -ForegroundColor Cyan
    Expand-Archive -Path $ArchivePath -DestinationPath $TempDir -Force

    $BinaryPath = Join-Path $TempDir "$Binary.exe"
    if (-not (Test-Path $BinaryPath)) {
        throw "Binary not found in archive"
    }

    # Create install directory
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Force $InstallDir | Out-Null
    }

    Copy-Item $BinaryPath (Join-Path $InstallDir "$Binary.exe") -Force
    Write-Host "==> Installed $Binary to $InstallDir\$Binary.exe" -ForegroundColor Cyan

    # Add to PATH if not already there
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$UserPath;$InstallDir", "User")
        Write-Host "==> Added $InstallDir to user PATH (restart terminal to take effect)" -ForegroundColor Cyan
    }

    Write-Host ""
    Write-Host "==> Next steps:" -ForegroundColor Cyan
    Write-Host "  1. Create config directory: mkdir ~\.spacebot-homelab"
    Write-Host "  2. Copy example config:     copy example.config.toml ~\.spacebot-homelab\config.toml"
    Write-Host "  3. Edit config with your Docker/SSH hosts"
    Write-Host "  4. Validate: $Binary doctor --config ~\.spacebot-homelab\config.toml"
}
finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
