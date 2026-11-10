param([Parameter(Mandatory = $true)][string]$MsiPath)
$ErrorActionPreference = 'Stop'

# CCS/1 RULE:read-only MSI database inspection; never install or execute custom actions.
function Read-MsiTable($Database, [string]$Sql) {
    $view = $Database.OpenView($Sql)
    try {
        [void]$view.Execute()
        $columns = $view.ColumnInfo(0)
        try {
            while ($record = $view.Fetch()) {
                try {
                    $count = $record.GetType().InvokeMember('FieldCount', [Reflection.BindingFlags]::GetProperty, $null, $record, $null)
                    $row = [ordered]@{}
                    for ($index = 1; $index -le $count; $index++) { $row[$columns.StringData($index)] = $record.StringData($index) }
                    [pscustomobject]$row
                } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) }
            }
        } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($columns) }
    } finally {
        [void]$view.Close()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
    }
}

$inspector = New-Object -ComObject WindowsInstaller.Installer
$database = $null
try {
    $database = $inspector.OpenDatabase((Resolve-Path -LiteralPath $MsiPath).Path, 0)
    $rules = @(Read-MsiTable $database 'SELECT * FROM WixFirewallException')
    if ($rules.Count -ne 1) { throw "Expected exactly one firewall exception, got $($rules.Count): $($rules | ConvertTo-Json -Compress)" }
    $rule = $rules[0]
    if ($rule.WixFirewallException -ne 'WindConnectLanUdp' -or $rule.Program -ne '[#Path]' -or
        $rule.RemoteAddresses -ne 'LocalSubnet' -or $rule.Protocol -ne '17' -or $rule.Profile -ne '2' -or
        $rule.Attributes -ne '0' -or $rule.Port -ne '' -or $rule.Component_ -ne 'WindConnectLanFirewall') {
        throw "Unexpected firewall scope: $($rule | ConvertTo-Json -Compress)"
    }
    $features = @(Read-MsiTable $database 'SELECT * FROM FeatureComponents')
    if (-not ($features | Where-Object { $_.Component_ -eq $rule.Component_ })) { throw 'Firewall component is not installed by a feature' }
    $files = @(Read-MsiTable $database 'SELECT * FROM File')
    if (-not ($files | Where-Object { $_.File -eq 'Path' -and $_.FileName -like '*windconnect-windows.exe' })) { throw 'Firewall program does not reference the installed executable' }
    $actions = @(Read-MsiTable $database 'SELECT * FROM CustomAction')
    foreach ($name in @('WixSchedFirewallExceptionsInstall', 'WixSchedFirewallExceptionsUninstall',
        'WixRollbackFirewallExceptionsInstall', 'WixExecFirewallExceptionsInstall',
        'WixRollbackFirewallExceptionsUninstall', 'WixExecFirewallExceptionsUninstall')) {
        if (-not ($actions | Where-Object { $_.Action -eq $name -and $_.Source -eq 'WixFirewallCA' })) { throw "Missing firewall action: $name" }
    }
    $sequence = @(Read-MsiTable $database 'SELECT * FROM InstallExecuteSequence')
    foreach ($name in @('WixSchedFirewallExceptionsInstall', 'WixSchedFirewallExceptionsUninstall')) {
        if (-not ($sequence | Where-Object { $_.Action -eq $name })) { throw "Firewall action is not scheduled: $name" }
    }
    $registry = @(Read-MsiTable $database 'SELECT * FROM Registry')
    $startup = @($registry | Where-Object { $_.Key -eq 'Software\Microsoft\Windows\CurrentVersion\Run' -and $_.Name -eq 'WindConnect' })
    if ($startup.Count -ne 0) { throw 'Installer must not write an autostart value' }
    $cleanup = @($actions | Where-Object { $_.Action -eq 'WindConnectRemoveAutostart' -and $_.Source -eq 'Path' -and $_.Target -eq '--remove-autostart' -and $_.Type -eq '82' })
    if ($cleanup.Count -ne 1) { throw 'Missing best-effort installed-EXE autostart cleanup action (Type 82, ignore failures)' }
    $schedule = @($sequence | Where-Object { $_.Action -eq 'WindConnectRemoveAutostart' -and $_.Condition -eq 'REMOVE="ALL" AND NOT UPGRADINGPRODUCTCODE' })
    $removeFiles = @($sequence | Where-Object { $_.Action -eq 'RemoveFiles' })
    if ($schedule.Count -ne 1 -or [int]$schedule[0].Sequence -ge [int]$removeFiles[0].Sequence) { throw 'Autostart cleanup must run on full uninstall, before removing its EXE, and not on upgrade' }
    Write-Output 'PASS: EXE/UDP/Private/LocalSubnet firewall lifecycle; current-user autostart uninstall cleanup, no startup value written on install.'
} finally {
    if ($database) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) }
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($inspector)
}
