<#
.SYNOPSIS
Frees disk space in a WindConnect working folder (a slot or a review copy)
without touching code (WN-95).

.DESCRIPTION
Removes only build output, from a fixed list under the repository root:
  - Cargo incremental caches: target/<profile>/incremental and
    target/<triple>/<profile>/incremental
  - MSI packages of earlier versions in target/release/bundle/msi
    (the current version, from Cargo.toml, stays)
  - Gradle builds: apps/android/build, app/build, wind-core/build, .gradle
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
    # MSI packages of earlier versions.
    $msi = Join-Path $target 'release/bundle/msi'
    if (Test-Path -LiteralPath $msi) {
        $manifest = Get-Content -LiteralPath (Join-Path $Root 'Cargo.toml') -Raw
        $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
        Get-ChildItem -LiteralPath $msi -File -Filter '*.msi' |
            Where-Object { -not $version -or $_.Name -notlike "*_${version}_*" } |
            ForEach-Object { Remove-Output $_.FullName }
    }
}

foreach ($path in @(
        'apps/android/build',
        'apps/android/app/build',
        'apps/android/wind-core/build',
        'apps/android/.gradle',
        'crates/wind-core/fuzz/target'
    )) {
    Remove-Output (Join-Path $Root $path)
}

Write-Host ("freed {0:N0} MB in {1}" -f ($script:freed / 1MB), $Root)
