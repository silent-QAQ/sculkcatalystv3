# SPDX-License-Identifier: Apache-2.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'
$caddyDir = Join-Path $runtime 'caddy'
$caddy = Join-Path $caddyDir 'caddy.exe'
$config = Join-Path $root 'deploy\Caddyfile'
$dataDir = Join-Path $runtime 'caddy-data'
$configDir = Join-Path $runtime 'caddy-config'

if (-not (Test-Path -LiteralPath $caddy)) {
    throw 'Caddy is not installed in .runtime.'
}

New-Item -ItemType Directory -Force -Path $dataDir, $configDir | Out-Null
$env:XDG_DATA_HOME = $dataDir
$env:XDG_CONFIG_HOME = $configDir

$stdout = Join-Path $caddyDir 'caddy.log'
$stderr = Join-Path $caddyDir 'caddy.err.log'
$command = "`"$caddy`" run --config `"$config`" --adapter caddyfile 1>>`"$stdout`" 2>>`"$stderr`""
& $env:ComSpec /d /s /c $command

exit $LASTEXITCODE
