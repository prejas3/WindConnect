<#
.SYNOPSIS
Frees disk space in a WindConnect working folder (a slot or a review copy)
without touching code (WN-95).

.DESCRIPTION
Removes only build output, from a fixed list under the repository root:
  - Cargo incremental caches: target/<profile>/incremental and
    target/<triple>/<profile>/incremental
  - MSI packages of versions older than [workspace.package] version in
    Cargo.toml, in target/release/bundle/msi; the current and newer ones
    stay, and without a readable version none is removed (a warning says so)
  - Gradle builds: apps/android/build, app/build, app/build-play (WN-124), wind-core/build, .gradle
  - the fuzzing target: crates/wind-core/fuzz/target and its corpus copies
    in fuzz/artifacts are left alone (they are inputs, not output)
With -Full it also removes the whole Cargo target folder.
Safe on a clean folder: missing paths are skipped. Nothing outside the
repository root is touched, and nothing Git tracks is removed.

.EXAMPLE
pwsh scripts/clean-build.ps1            # after a card is Done or a review
pwsh scripts/clean-build.ps1 -Full      # also drop target/ entirely
pwsh scripts/clean-build.ps1 -WhatIf    # list what would go
#>
[CmdletBinding(SupportsShouldProcess)]
param(
    # Repository root; defaults to the folder above this script.
    [string]$Root = (Split-Path -Parent $PSScriptRoot),
    # Also remove the whole Cargo target folder.
    [switch]$Full
)

$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path -LiteralPath $Root).Path
if (-not (Test-Path -LiteralPath (Join-Path $Root 'Cargo.toml'))) {
    throw "$Root is not the WindConnect repository root (no Cargo.toml)"
}

function Get-Size([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return 0 }
    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer) { return $item.Length }
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue |
        Measure-Object -Property Length -Sum).Sum
    if ($null -eq $sum) { 0 } else { $sum }
}

# `version = "x.y.z"` of the [workspace.package] table only (not the first
# `version =` in the file); $null when it is missing or not x.y.z.
function Get-WorkspaceVersion([string]$Manifest) {
    $inside = $false
    foreach ($line in $Manifest -split '\r?\n') {
        $trimmed = $line.Trim()
        if ($trimmed -match '^\[\s*([^\]]+?)\s*\]') {
            $inside = $Matches[1] -eq 'workspace.package'
            continue
        }
        if ($inside -and $trimmed -match '^version\s*=\s*"(\d+\.\d+\.\d+)"\s*(#.*)?$') {
            return [version]$Matches[1]
        }
    }
    return $null
}

$script:freed = 0
function Remove-Output([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $full = (Resolve-Path -LiteralPath $Path).Path
    # Only inside the repository, never the root itself.
    if (-not $full.StartsWith($Root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to remove $full (outside $Root)"
    }
    # Never anything Git tracks.
    $relative = $full.Substring($Root.Length + 1)
    $tracked = & git -C $Root ls-files -- $relative 2>$null
    if ($tracked) {
        throw "refusing to remove $relative (tracked by Git)"
    }
    $size = Get-Size $full
    if ($PSCmdlet.ShouldProcess($relative, "remove ($([math]::Round($size / 1MB)) MB)")) {
        Remove-Item -LiteralPath $full -Recurse -Force
        $script:freed += $size
        Write-Host ("removed {0,8:N0} MB  {1}" -f ($size / 1MB), $relative)
    }
}

$target = Join-Path $Root 'target'

if ($Full) {
    Remove-Output $target
} elseif (Test-Path -LiteralPath $target) {
    # target/<profile>/incremental and target/<triple>/<profile>/incremental
    Get-ChildItem -LiteralPath $target -Directory -Force | ForEach-Object {
        Remove-Output (Join-Path $_.FullName 'incremental')
        Get-ChildItem -LiteralPath $_.FullName -Directory -Force | ForEach-Object {
            Remove-Output (Join-Path $_.FullName 'incremental')
        }
    }
    # MSI packages of earlier versions (WindConnect_<x.y.z>_x64_en-US.msi).
    # CCS/1 RULE:no version read -> no MSI removed (WN-98). Only versions older
    # than the workspace's go; newer ones (another branch) and odd names stay.
    $msi = Join-Path $target 'release/bundle/msi'
    if (Test-Path -LiteralPath $msi) {
        $manifest = Get-Content -LiteralPath (Join-Path $Root 'Cargo.toml') -Raw
        $current = Get-WorkspaceVersion $manifest
        if ($null -eq $current) {
            Write-Warning "no version in [workspace.package] of Cargo.toml; no MSI removed from $msi"
        } else {
            Get-ChildItem -LiteralPath $msi -File -Filter '*.msi' |
                Where-Object {
                    $match = [regex]::Match($_.Name, '_(\d+\.\d+\.\d+)_')
                    $match.Success -and [version]$match.Groups[1].Value -lt $current
                } |
                ForEach-Object { Remove-Output $_.FullName }
        }
    }
}

foreach ($path in @(
        'apps/android/build',
        'apps/android/app/build',
        'apps/android/app/build-play',
        'apps/android/wind-core/build',
        'apps/android/.gradle',
        'crates/wind-core/fuzz/target'
    )) {
    Remove-Output (Join-Path $Root $path)
}

Write-Host ("freed {0:N0} MB in {1}" -f ($script:freed / 1MB), $Root)
