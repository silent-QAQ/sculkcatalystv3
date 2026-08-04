# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

<#
.SYNOPSIS
Starts the Windows-native Cloud development stack.

.PARAMETER EnableCodexFullAccess
Enables full host access only for the trusted native Codex CLI in this newly
started loopback backend. Codex still needs the application's "full" review mode.

.PARAMETER CodexCommand
Absolute path to the same Codex .exe, .cmd, or .bat command configured for the
native Codex agent. Required with EnableCodexFullAccess.

.EXAMPLE
$codexCommand = (Get-Command codex.cmd -CommandType Application).Path
.\scripts\start-cloud.ps1 -EnableCodexFullAccess -CodexCommand $codexCommand
#>
param(
    [switch]$Quiet,
    [switch]$EnableCodexFullAccess,
    [string]$CodexCommand = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'
$postgresData = Join-Path $runtime 'postgres\data'
$postgresLog = Join-Path $runtime 'postgres\postgres.log'
$postgresCtl = 'C:\Program Files\PostgreSQL\18\bin\pg_ctl.exe'
$redisServer = (Get-ChildItem -LiteralPath (Join-Path $runtime 'redis') -Recurse -Filter redis-server.exe | Select-Object -First 1).FullName
$redisRoot = Split-Path $redisServer -Parent
$redisData = Join-Path $runtime 'redis-data'
$backend = Join-Path $root 'backend\target-cloud\debug\backend.exe'

function Resolve-CodexFullAccessCommand([bool]$Enabled, [string]$Command) {
    if (-not $Enabled) {
        if (-not [string]::IsNullOrWhiteSpace($Command)) {
            throw '-CodexCommand can only be used with -EnableCodexFullAccess.'
        }
        return $null
    }

    if ([string]::IsNullOrWhiteSpace($Command)) {
        throw '-EnableCodexFullAccess requires -CodexCommand with an absolute native Codex CLI path.'
    }
    if ($Command -notmatch '^(?:[A-Za-z]:[\\/]|\\\\)') {
        throw '-CodexCommand must be an absolute path.'
    }

    $resolved = (Resolve-Path -LiteralPath $Command -ErrorAction Stop).ProviderPath
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "Codex command does not exist: $resolved"
    }
    $extension = [System.IO.Path]::GetExtension($resolved).ToLowerInvariant()
    if ($extension -notin @('.exe', '.cmd', '.bat')) {
        throw '-CodexCommand must point to a native Windows Codex .exe, .cmd, or .bat command.'
    }
    return $resolved
}

function Start-BackendWithCodexAccess(
    [string]$FilePath,
    [string]$WorkingDirectory,
    [string]$StandardOutput,
    [string]$StandardError,
    [bool]$EnableFullAccess,
    [string]$TrustedCodexCommand
) {
    $previousFullAccess = [Environment]::GetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', 'Process')
    $previousTrustedCommand = [Environment]::GetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', 'Process')
    try {
        # Do not inherit an accidental opt-in from the shell running this script.
        [Environment]::SetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', $null, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', $null, 'Process')
        if ($EnableFullAccess) {
            [Environment]::SetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', 'true', 'Process')
            [Environment]::SetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', $TrustedCodexCommand, 'Process')
        }
        return Start-Process -FilePath $FilePath `
            -WorkingDirectory $WorkingDirectory `
            -RedirectStandardOutput $StandardOutput `
            -RedirectStandardError $StandardError `
            -WindowStyle Hidden
    } finally {
        [Environment]::SetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', $previousFullAccess, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', $previousTrustedCommand, 'Process')
    }
}

$trustedCodexCommand = Resolve-CodexFullAccessCommand ([bool]$EnableCodexFullAccess) $CodexCommand

function Get-Listener([int]$Port) {
    Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
}

if (-not (Test-Path -LiteralPath (Join-Path $postgresData 'PG_VERSION'))) {
    throw 'PostgreSQL runtime is not initialized.'
}
if (-not (Test-Path -LiteralPath $redisServer)) {
    throw 'Redis runtime is not installed.'
}
if (-not (Test-Path -LiteralPath $backend)) {
    throw 'Rust backend is not built. Run cargo build with CARGO_TARGET_DIR=target-cloud.'
}

if (-not (Get-Listener 55432)) {
    & $postgresCtl start -D $postgresData -l $postgresLog -o '-p 55432 -h 127.0.0.1' -w
    if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL failed to start.' }
}

if (-not (Get-Listener 56379)) {
    Start-Process -FilePath $redisServer `
        -ArgumentList @('--bind','127.0.0.1','--port','56379','--dir',$redisData,'--dbfilename','dump.rdb','--appendonly','no') `
        -WorkingDirectory $redisRoot `
        -RedirectStandardOutput (Join-Path $runtime 'redis\redis.log') `
        -RedirectStandardError (Join-Path $runtime 'redis\redis.err.log') `
        -WindowStyle Hidden
}

$backendListener = Get-Listener 8788
if (-not $backendListener) {
    $env:SCULK_BIND_ADDRESS = '127.0.0.1:8788'
    $env:SCULK_STATE_FILE = 'data/state-cloud.json'
    # The bootstrap endpoint writes this trusted URL into one-time Agent JSON.
    # Local development intentionally uses loopback HTTP; production must set
    # SCULK_CLOUD_PUBLIC_URL to the public HTTPS origin in its service env.
    $env:SCULK_CLOUD_PUBLIC_URL = 'http://127.0.0.1:8788'
    Start-BackendWithCodexAccess `
        -FilePath $backend `
        -WorkingDirectory (Join-Path $root 'backend') `
        -StandardOutput (Join-Path $runtime 'backend.log') `
        -StandardError (Join-Path $runtime 'backend.err.log') `
        -EnableFullAccess ([bool]$EnableCodexFullAccess) `
        -TrustedCodexCommand $trustedCodexCommand
} elseif ($EnableCodexFullAccess) {
    throw 'Codex full access only applies when starting a new backend. Stop the existing Cloud backend and run this command again.'
}

$ready = $false
for ($attempt = 0; $attempt -lt 30; $attempt++) {
    Start-Sleep -Milliseconds 500
    try {
        $status = Invoke-RestMethod 'http://127.0.0.1:8788/api/cloud/status'
        if ($status.available) { $ready = $true; break }
    } catch {}
}
if (-not $ready) { throw 'Sculk Cloud did not become ready.' }

if (-not $Quiet) {
    [pscustomobject]@{
        Web = 'http://127.0.0.1:8788'
        PostgreSQL = '127.0.0.1:55432'
        Redis = '127.0.0.1:56379'
        CloudAvailable = $status.available
    } | Format-List
}
