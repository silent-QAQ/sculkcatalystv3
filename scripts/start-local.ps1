# SPDX-License-Identifier: Apache-2.0

param(
    [int]$Port = 8787,
    [switch]$Quiet,
    [switch]$KeepAlive,
    [string]$NapCatApiUrl = '',
    [string]$NapCatConfigPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtime = Join-Path $root '.runtime'
$backend = Join-Path $root 'backend\target-local\release\backend.exe'
$staticDir = Join-Path $root 'frontend\dist'
$backendDir = Join-Path $root 'backend'
$pidFile = Join-Path $runtime 'backend.pid'
$backendPid = $null

if (-not (Test-Path -LiteralPath $backend)) {
    throw 'Rust release backend is not built.'
}
if (-not (Test-Path -LiteralPath (Join-Path $staticDir 'index.html'))) {
    throw 'Frontend production bundle is not built.'
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

    $backendProcess = Start-Process -FilePath $backend `
        -WorkingDirectory $backendDir `
        -RedirectStandardOutput (Join-Path $runtime 'backend.log') `
        -RedirectStandardError (Join-Path $runtime 'backend.err.log') `
        -WindowStyle Hidden `
        -PassThru
    $backendPid = $backendProcess.Id
    Set-Content -LiteralPath $pidFile -Value $backendPid -Encoding ASCII
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
