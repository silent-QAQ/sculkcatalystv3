# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'

foreach ($port in 8788, 56379) {
    $listener = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $listener) { continue }
    $process = Get-CimInstance Win32_Process -Filter "ProcessId=$($listener.OwningProcess)"
    if ($process.ExecutablePath -notlike "$root\*") {
        throw "Port $port is owned by an unexpected process: $($process.ExecutablePath)"
    }
    Stop-Process -Id $listener.OwningProcess -Force
}

$postgresData = Join-Path $runtime 'postgres\data'
$postgresCtl = 'C:\Program Files\PostgreSQL\18\bin\pg_ctl.exe'
if (Get-NetTCPConnection -LocalPort 55432 -State Listen -ErrorAction SilentlyContinue) {
    & $postgresCtl stop -D $postgresData -m fast -w
}
