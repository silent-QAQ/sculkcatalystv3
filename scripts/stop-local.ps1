# SPDX-License-Identifier: Apache-2.0

param([int]$Port = 8787)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$backend = Join-Path $root 'backend\target-local\release\backend.exe'
$pidFile = Join-Path $root '.runtime\backend.pid'

if (-not (Test-Path -LiteralPath $pidFile)) {
    return
}

$backendPid = [int](Get-Content -Raw -LiteralPath $pidFile)
$process = Get-Process -Id $backendPid -ErrorAction SilentlyContinue
if (-not $process) {
    Remove-Item -LiteralPath $pidFile
    return
}
if ($process.Path -ne $backend) {
    throw "Recorded process is not the Sculk backend: $($process.Path)"
}

try {
    $baseUrl = "http://127.0.0.1:$Port"
    $dashboard = Invoke-RestMethod -Uri "$baseUrl/api/dashboard" -TimeoutSec 3
    foreach ($server in @($dashboard.servers)) {
        if ($server.status -notin @('online', 'warning')) {
            continue
        }
        try {
            Invoke-RestMethod `
                -Uri "$baseUrl/api/servers/$($server.id)/action" `
                -Method Post `
                -ContentType 'application/json' `
                -Body '{"action":"stop"}' `
                -TimeoutSec 45 | Out-Null
        } catch {
            Write-Warning "Server $($server.id) did not stop cleanly: $($_.Exception.Message)"
        }
    }
} catch {
    Write-Warning "Backend did not accept graceful server shutdown requests: $($_.Exception.Message)"
}

Stop-Process -Id $backendPid -Force
Remove-Item -LiteralPath $pidFile
