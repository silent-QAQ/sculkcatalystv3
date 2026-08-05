# SPDX-License-Identifier: Apache-2.0

[CmdletBinding()]
param([int]$BackendPort = 8787)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Read-RecordedPid([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    $raw = (Get-Content -Raw -LiteralPath $Path).Trim()
    $recordedPid = 0
    if (-not [int]::TryParse($raw, [ref]$recordedPid) -or $recordedPid -le 0) {
        throw "Invalid $Label PID file: $Path"
    }
    return $recordedPid
}

$root = Split-Path $PSScriptRoot -Parent
$runtimeDir = Join-Path $root '.runtime'
$proxyPidFile = Join-Path $runtimeDir 'public-proxy.pid'
$proxyCommandFile = Join-Path $runtimeDir 'public-proxy.command'
$publicBackendPidFile = Join-Path $runtimeDir 'public-backend.pid'
$backendPidFile = Join-Path $runtimeDir 'backend.pid'
$backend = Join-Path $root 'backend\target-local\release\backend.exe'
$stopLocal = Join-Path $PSScriptRoot 'stop-local.ps1'

$proxyPid = Read-RecordedPid $proxyPidFile 'Caddy'
if ($null -ne $proxyPid) {
    $proxy = Get-Process -Id $proxyPid -ErrorAction SilentlyContinue
    if ($null -ne $proxy) {
        if (-not (Test-Path -LiteralPath $proxyCommandFile -PathType Leaf)) {
            throw "Caddy command metadata is missing: $proxyCommandFile"
        }
        $expectedCommand = (Get-Content -Raw -LiteralPath $proxyCommandFile).Trim([char]0xfeff).Trim()
        if ([string]::IsNullOrWhiteSpace($expectedCommand) -or
            [string]::IsNullOrWhiteSpace($proxy.Path) -or
            -not $proxy.Path.Equals($expectedCommand, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Public proxy PID file points to a different process: $proxyPid"
        }
        try {
            Stop-Process -Id $proxyPid -ErrorAction Stop
            $proxy.WaitForExit(10000) | Out-Null
        } catch {
            Stop-Process -Id $proxyPid -Force -ErrorAction SilentlyContinue
        }
    }
    Remove-Item -LiteralPath $proxyPidFile, $proxyCommandFile -Force -ErrorAction SilentlyContinue
}

$stopManagedBackend = $false
$publicBackendPid = Read-RecordedPid $publicBackendPidFile 'public backend'
$backendPid = Read-RecordedPid $backendPidFile 'backend'
if ($null -ne $publicBackendPid -and $publicBackendPid -eq $backendPid) {
    $backendProcess = Get-Process -Id $backendPid -ErrorAction SilentlyContinue
    $stopManagedBackend = $null -ne $backendProcess -and
        -not [string]::IsNullOrWhiteSpace($backendProcess.Path) -and
        $backendProcess.Path.Equals($backend, [System.StringComparison]::OrdinalIgnoreCase)
}
if ($stopManagedBackend) {
    & $stopLocal -Port $BackendPort
}
Remove-Item -LiteralPath $publicBackendPidFile -Force -ErrorAction SilentlyContinue
