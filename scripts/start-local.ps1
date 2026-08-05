# SPDX-License-Identifier: Apache-2.0

<#
.SYNOPSIS
Starts the local production backend and frontend bundle.

.PARAMETER EnableCodexFullAccess
Enables full host access only for the trusted native Codex CLI in this newly
started loopback backend. Codex still needs the application's "full" review mode.

.PARAMETER CodexCommand
Absolute path to the same Codex .exe, .cmd, or .bat command configured for the
native Codex agent. Required with EnableCodexFullAccess.

.EXAMPLE
$codexCommand = (Get-Command codex.cmd -CommandType Application).Path
.\scripts\start-local.ps1 -EnableCodexFullAccess -CodexCommand $codexCommand
#>
param(
    [int]$Port = 8787,
    [switch]$Quiet,
    [switch]$KeepAlive,
    [string]$NapCatApiUrl = '',
    [string]$NapCatConfigPath = '',
    [switch]$EnableCodexFullAccess,
    [string]$CodexCommand = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'
$backend = Join-Path $root 'backend\target-local\release\backend.exe'
$staticDir = Join-Path $root 'frontend\dist'
$backendDir = Join-Path $root 'backend'
$pidFile = Join-Path $runtime 'backend.pid'
$publicBackendPidFile = Join-Path $runtime 'public-backend.pid'
$backendPid = $null

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
    $previousCloudDisabled = [Environment]::GetEnvironmentVariable('SCULK_DISABLE_CLOUD', 'Process')
    $previousPublicProxyManaged = [Environment]::GetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', 'Process')
    $cloudEnvironment = @{}
    foreach ($entry in Get-ChildItem Env:) {
        if ($entry.Name -match '^(?:DATABASE_URL|REDIS_URL|SCULK_MASTER_KEY|SCULK_ALLOWED_ORIGINS|SCULK_CLOUD_|SCULK_POSTGRES_|SCULK_REDIS_)') {
            $cloudEnvironment[$entry.Name] = $entry.Value
            [Environment]::SetEnvironmentVariable($entry.Name, $null, 'Process')
        }
    }
    try {
        # Do not inherit an accidental opt-in from the shell running this script.
        [Environment]::SetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', $null, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', $null, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_DISABLE_CLOUD', 'true', 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', $null, 'Process')
        if ($EnableFullAccess) {
            [Environment]::SetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', 'true', 'Process')
            [Environment]::SetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', $TrustedCodexCommand, 'Process')
        }
        return Start-Process -FilePath $FilePath `
            -WorkingDirectory $WorkingDirectory `
            -RedirectStandardOutput $StandardOutput `
            -RedirectStandardError $StandardError `
            -WindowStyle Hidden `
            -PassThru
    } finally {
        [Environment]::SetEnvironmentVariable('SCULK_ALLOW_CODEX_FULL', $previousFullAccess, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_CODEX_TRUSTED_COMMAND', $previousTrustedCommand, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_DISABLE_CLOUD', $previousCloudDisabled, 'Process')
        [Environment]::SetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', $previousPublicProxyManaged, 'Process')
        foreach ($name in $cloudEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $cloudEnvironment[$name], 'Process')
        }
    }
}

$trustedCodexCommand = Resolve-CodexFullAccessCommand ([bool]$EnableCodexFullAccess) $CodexCommand

if (-not (Test-Path -LiteralPath $backend)) {
    throw 'Rust release backend is not built.'
}
if (-not (Test-Path -LiteralPath (Join-Path $staticDir 'index.html'))) {
    throw 'Frontend production bundle is not built.'
}

# A normal local launch invalidates the marker used by start-public.ps1. The
# marker is retained only for the temporary handoff from that script.
if (-not [string]::Equals(
    [Environment]::GetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', 'Process'),
    'true',
    [System.StringComparison]::OrdinalIgnoreCase
)) {
    Remove-Item -LiteralPath $publicBackendPidFile -Force -ErrorAction SilentlyContinue
}

if (Test-Path -LiteralPath $pidFile) {
    $recordedPid = [int](Get-Content -Raw -LiteralPath $pidFile)
    $recordedProcess = Get-Process -Id $recordedPid -ErrorAction SilentlyContinue
    if ($recordedProcess -and $recordedProcess.Path -eq $backend) {
        $backendPid = $recordedPid
    } else {
        Remove-Item -LiteralPath $pidFile
    }
}

if (-not $backendPid) {
    $connection = [Net.Sockets.TcpClient]::new()
    try {
        $connectTask = $connection.ConnectAsync('127.0.0.1', $Port)
        $portInUse = $connectTask.Wait(500) -and $connection.Connected
    } catch {
        $portInUse = $false
    } finally {
        $connection.Dispose()
    }
    if ($portInUse) {
        throw "Port $Port is already in use by an untracked process."
    }

    New-Item -ItemType Directory -Force -Path $runtime | Out-Null
    $env:SCULK_BIND_ADDRESS = "127.0.0.1:$Port"
    $env:SCULK_STATIC_DIR = $staticDir
    $env:SCULK_DATA_DIR = Join-Path $backendDir 'data'
    $env:SCULK_STATE_FILE = Join-Path $env:SCULK_DATA_DIR 'state.json'

    if (-not [string]::IsNullOrWhiteSpace($NapCatApiUrl)) {
        $env:SCULK_NAPCAT_API_URL = $NapCatApiUrl.TrimEnd('/')
    }
    if (-not [string]::IsNullOrWhiteSpace($NapCatConfigPath)) {
        if (-not (Test-Path -LiteralPath $NapCatConfigPath)) {
            throw "NapCat config file does not exist: $NapCatConfigPath"
        }
        $napCatConfig = Get-Content -Raw -LiteralPath $NapCatConfigPath | ConvertFrom-Json
        $napCatToken = [string]$napCatConfig.bots[0].connect.httpServers[0].token
        if ([string]::IsNullOrWhiteSpace($napCatToken)) {
            throw 'NapCat HTTP server token is missing.'
        }
        $env:SCULK_NAPCAT_ACCESS_TOKEN = $napCatToken
    }

    # Some managed shells inject both Path and PATH. Windows PowerShell 5.1
    # cannot pass that duplicate environment block through Start-Process.
    $cleanPath = [Environment]::GetEnvironmentVariable(
        'Path',
        [EnvironmentVariableTarget]::Process
    )
    [Environment]::SetEnvironmentVariable(
        'PATH',
        $null,
        [EnvironmentVariableTarget]::Process
    )
    [Environment]::SetEnvironmentVariable(
        'Path',
        $cleanPath,
        [EnvironmentVariableTarget]::Process
    )

    $backendProcess = Start-BackendWithCodexAccess `
        -FilePath $backend `
        -WorkingDirectory $backendDir `
        -StandardOutput (Join-Path $runtime 'backend.log') `
        -StandardError (Join-Path $runtime 'backend.err.log') `
        -EnableFullAccess ([bool]$EnableCodexFullAccess) `
        -TrustedCodexCommand $trustedCodexCommand
    $backendPid = $backendProcess.Id
    Set-Content -LiteralPath $pidFile -Value $backendPid -Encoding ASCII
} elseif ($EnableCodexFullAccess) {
    throw 'Codex full access only applies when starting a new backend. Stop the existing local backend and run this command again.'
}

$ready = $false
for ($attempt = 0; $attempt -lt 40; $attempt++) {
    Start-Sleep -Milliseconds 500
    try {
        $dashboard = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/dashboard" -TimeoutSec 3
        $ready = $true
        break
    } catch {}
}
if (-not $ready) {
    throw "Sculk Catalyst did not become ready on port $Port."
}

if (-not $Quiet) {
    [pscustomobject]@{
        Web = "http://127.0.0.1:$Port"
        ProcessId = $backendPid
        Servers = @($dashboard.servers).Count
        Tasks = @($dashboard.tasks).Count
    } | Format-List
}

if ($KeepAlive -and $backendPid) {
    Wait-Process -Id $backendPid
}
