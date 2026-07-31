# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

foreach ($taskName in @('Sculk-Caddy', 'Sculk-Cloud-Backend', 'Sculk-Garnet')) {
    $task = Get-ScheduledTask -TaskName $taskName
    if ($task.State -eq 'Running') {
        Stop-ScheduledTask -TaskName $taskName
    }
}

$postgres = Get-Service -Name 'sculk-postgresql-18'
if ($postgres.Status -ne 'Stopped') {
    Stop-Service -Name 'sculk-postgresql-18'
    $postgres.WaitForStatus('Stopped', (New-TimeSpan -Seconds 30))
}
