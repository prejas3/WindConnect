<#
.SYNOPSIS
Checks which MSI packages scripts/clean-build.ps1 removes (WN-98).

.DESCRIPTION
Builds throwaway repository layouts in %TEMP% (Cargo.toml, a Git repository
and target/release/bundle/msi with empty .msi files), runs clean-build.ps1 on
each and compares what is left. Touches nothing outside its temp folders.
Exit code 0 when every case passes.

.EXAMPLE
pwsh scripts/test-clean-build.ps1
#>
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'clean-build.ps1'
$failures = 0

function Test-Case([string]$Name, [string]$Manifest, [string[]]$Msi, [string[]]$Left, [bool]$Warns) {
    $root = Join-Path ([IO.Path]::GetTempPath()) ("windconnect-wn98-" + [guid]::NewGuid())
    try {
        $msiDir = Join-Path $root 'target/release/bundle/msi'
        New-Item -ItemType Directory -Force $msiDir | Out-Null
        Set-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Value $Manifest
        & git -C $root init -q
        foreach ($file in $Msi) { New-Item -ItemType File (Join-Path $msiDir $file) | Out-Null }

        $output = & $script -Root $root 3>&1 6>&1
        $warnings = @($output | Where-Object { $_ -is [System.Management.Automation.WarningRecord] })
        $remaining = @(Get-ChildItem -LiteralPath $msiDir -File | ForEach-Object Name | Sort-Object)
        $expected = @($Left | Sort-Object)

        $problems = @()
        if (($remaining -join '|') -ne ($expected -join '|')) {
            $problems += "left [$($remaining -join ', ')], expected [$($expected -join ', ')]"
        }
        if ($Warns -and $warnings.Count -eq 0) { $problems += 'no warning' }
        if (-not $Warns -and $warnings.Count -gt 0) { $problems += "unexpected warning: $($warnings[0])" }
        if ($problems) {
            $script:failures++
            Write-Host "FAIL $Name - $($problems -join '; ')"
        } else {
            Write-Host "ok   $Name$(if ($warnings) { " (warning: $($warnings[0]))" })"
        }
    } finally {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$all = 'WindConnect_0.36.0_x64_en-US.msi', 'WindConnect_0.37.2_x64_en-US.msi',
'WindConnect_0.38.0_x64_en-US.msi', 'WindConnect_0.39.0_x64_en-US.msi', 'WindConnect_dev.msi'

Test-Case 'no [workspace.package] version: nothing removed, warning' `
    "[workspace]`nmembers = []`n`n[workspace.package]`nedition = `"2021`"`n" `
    $all $all $true

Test-Case 'version only in another table: nothing removed, warning' `
    "[package]`nname = `"x`"`nversion = `"9.9.9`"`n`n[workspace.package]`nedition = `"2021`"`n" `
    $all $all $true

Test-Case 'no Cargo version format (x.y.z): nothing removed, warning' `
    "[workspace.package]`nversion = `"next`"`n" `
    $all $all $true

Test-Case 'current version: only older ones go' `
    "[workspace]`nmembers = []`n`n[workspace.package]`nversion = `"0.38.0`"`nedition = `"2021`"`n" `
    $all ('WindConnect_0.38.0_x64_en-US.msi', 'WindConnect_0.39.0_x64_en-US.msi', 'WindConnect_dev.msi') $false

Test-Case 'first version = in another table is not the current one' `
    "[package]`nversion = `"0.1.0`"`n`n[workspace.package]`nversion = `"0.38.0`"`n" `
    $all ('WindConnect_0.38.0_x64_en-US.msi', 'WindConnect_0.39.0_x64_en-US.msi', 'WindConnect_dev.msi') $false

Test-Case 'numeric comparison: 0.9.0 is older than 0.10.0' `
    "[workspace.package]`nversion = `"0.10.0`" # current`n" `
    ('WindConnect_0.9.0_x64_en-US.msi', 'WindConnect_0.10.0_x64_en-US.msi') ('WindConnect_0.10.0_x64_en-US.msi') $false

if ($failures) { Write-Host "$failures case(s) failed"; exit 1 }
Write-Host 'all cases passed'
