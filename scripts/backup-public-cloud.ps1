# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'
$backupDir = Join-Path $runtime 'backups'
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$dump = Join-Path $backupDir "sculk-cloud-$timestamp.dump"
$stateSource = Join-Path $root 'backend\data\state-cloud.json'
$stateBackup = Join-Path $backupDir "state-cloud-$timestamp.json"
$pgDump = 'D:\PostgreSQL\18\bin\pg_dump.exe'

New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
$env:PGPASSWORD = Get-Content -Raw -LiteralPath `
    (Join-Path $runtime 'secrets\sculk-db.txt')

& $pgDump `
    -h 127.0.0.1 `
    -p 55432 `
    -U sculk `
    -d sculk_cloud `
    -F custom `
    -f $dump

if ($LASTEXITCODE -ne 0) {
    throw 'PostgreSQL backup failed.'
}

if (Test-Path -LiteralPath $stateSource) {
    Copy-Item -LiteralPath $stateSource -Destination $stateBackup
}

[pscustomobject]@{
    DatabaseBackup = $dump
    StateBackup = if (Test-Path -LiteralPath $stateBackup) {
        $stateBackup
    } else {
        $null
    }
} | Format-List
