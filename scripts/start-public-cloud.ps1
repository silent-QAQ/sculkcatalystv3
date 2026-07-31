# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$postgresService = 'sculk-postgresql-18'
$tasks = @('Sculk-Garnet', 'Sculk-Cloud-Backend', 'Sculk-Caddy')

$postgres = Get-Service -Name $postgresService
if ($postgres.Status -ne 'Running') {
    Start-Service -Name $postgresService
    $postgres.WaitForStatus('Running', (New-TimeSpan -Seconds 30))
}

foreach ($taskName in $tasks) {
    $task = Get-ScheduledTask -TaskName $taskName
    if ($task.State -ne 'Running') {
        Start-ScheduledTask -TaskName $taskName
    }
}

$ready = $false
for ($attempt = 0; $attempt -lt 60; $attempt++) {
    Start-Sleep -Milliseconds 500
    try {
        $status = Invoke-RestMethod `
            -Uri 'http://127.0.0.1:8788/api/cloud/status' `
            -TimeoutSec 2
        if ($status.available) {
            $ready = $true
            break
        }
    } catch {}
}

if (-not $ready) {
    throw 'Sculk Cloud backend did not become ready.'
}

$certificate = Get-ChildItem `
    -LiteralPath (Join-Path $PSScriptRoot '..\.runtime\caddy-data') `
    -Recurse `
    -File `
    -Filter '*.crt' `
    -ErrorAction SilentlyContinue |
    Select-Object -First 1

[pscustomobject]@{
    LocalBackend = 'http://127.0.0.1:8788'
    PublicUrl = 'https://sculk.mcmy.love'
    CloudAvailable = $status.available
    OriginTlsReady = [bool]$certificate
} | Format-List
