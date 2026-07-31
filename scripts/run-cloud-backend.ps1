# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'
$backendDir = Join-Path $root 'backend'
$backend = Join-Path $backendDir 'target-cloud\release\backend.exe'

if (-not (Test-Path -LiteralPath $backend)) {
    throw 'Rust release backend is not built.'
}
if (-not (Test-Path -LiteralPath (Join-Path $root '.env'))) {
    throw 'Production .env is not configured.'
}

New-Item -ItemType Directory -Force -Path $runtime | Out-Null
Set-Location -LiteralPath $backendDir

$envFile = Join-Path $root '.env'
foreach ($line in Get-Content -LiteralPath $envFile) {
    if (-not $line -or $line.StartsWith('#')) {
        continue
    }
    $parts = $line -split '=', 2
    if ($parts.Count -eq 2) {
        [Environment]::SetEnvironmentVariable(
            $parts[0],
            $parts[1],
            [EnvironmentVariableTarget]::Process
        )
    }
}

$stdout = Join-Path $runtime 'cloud-backend.log'
$stderr = Join-Path $runtime 'cloud-backend.err.log'
$command = "`"$backend`" 1>>`"$stdout`" 2>>`"$stderr`""
& $env:ComSpec /d /s /c $command

exit $LASTEXITCODE
