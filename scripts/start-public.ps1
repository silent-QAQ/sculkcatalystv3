# SPDX-License-Identifier: Apache-2.0

<#
.SYNOPSIS
Starts a loopback-only backend behind an authenticated Caddy HTTPS proxy.

.DESCRIPTION
The backend continues to listen only on 127.0.0.1. Caddy is the sole public
listener and protects every route, including static files, APIs, WebSockets,
and objects, with HTTPS and HTTP Basic authentication.

.PARAMETER ConfirmPublicAdminConsole
Acknowledges that this opens a remotely reachable administrative console. The
distributed Start-Public-HTTPS.bat shortcut supplies this switch automatically.
#>
[CmdletBinding()]
param(
    [int]$BackendPort = 8787,
    [string]$Domain = '',
    [string]$Email = '',
    [string]$Username = 'sculk',
    [string]$CaddyCommand = '',
    [switch]$ConfirmPublicAdminConsole,
    [switch]$ResetCredentials,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($BackendPort -lt 1 -or $BackendPort -gt 65535) {
    throw 'BackendPort must be between 1 and 65535.'
}
if (-not $ConfirmPublicAdminConsole) {
    throw 'Pass -ConfirmPublicAdminConsole to acknowledge the remotely reachable administrative console.'
}

function Resolve-CaddyCommand([string]$Command) {
    if (-not [string]::IsNullOrWhiteSpace($Command)) {
        if (-not [System.IO.Path]::IsPathRooted($Command)) {
            throw '-CaddyCommand must be an absolute path when specified.'
        }
        $resolved = (Resolve-Path -LiteralPath $Command -ErrorAction Stop).ProviderPath
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            throw "Caddy executable does not exist: $resolved"
        }
        return $resolved
    }

    foreach ($name in @('caddy.exe', 'caddy')) {
        $candidate = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -ne $candidate) {
            return $candidate.Source
        }
    }
    throw 'Caddy was not found. Install Caddy first or pass -CaddyCommand with its absolute executable path.'
}

function Require-Domain([string]$Value) {
    $domain = $Value.Trim().ToLowerInvariant()
    if ($domain -notmatch '^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$') {
        throw 'A public HTTPS deployment requires a DNS hostname such as console.example.com.'
    }
    return $domain
}

function Require-Email([string]$Value) {
    $email = $Value.Trim()
    if ($email -notmatch '^[^\s@]+@[^\s@]+\.[^\s@]+$') {
        throw 'A valid ACME contact email is required for automatic HTTPS certificates.'
    }
    return $email
}

function Require-Username([string]$Value) {
    $username = $Value.Trim()
    if ($username -notmatch '^[A-Za-z0-9_-]{1,64}$') {
        throw 'Username must contain 1-64 ASCII letters, digits, underscores, or hyphens.'
    }
    return $username
}

function New-AdministratorPassword {
    $bytes = [byte[]]::new(32)
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    return -join ($bytes | ForEach-Object { $_.ToString('x2') })
}

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

function Process-UsesExecutable([System.Diagnostics.Process]$Process, [string]$ExpectedExecutable) {
    return -not [string]::IsNullOrWhiteSpace($Process.Path) -and
        $Process.Path.Equals($ExpectedExecutable, [System.StringComparison]::OrdinalIgnoreCase)
}

function Get-RecordedBackendProcess(
    [string]$PidPath,
    [string]$PublicPidPath,
    [string]$ExpectedExecutable
) {
    $backendProcessId = Read-RecordedPid $PidPath 'backend'
    if ($null -eq $backendProcessId) {
        Remove-Item -LiteralPath $PublicPidPath -Force -ErrorAction SilentlyContinue
        return $null
    }
    $process = Get-Process -Id $backendProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        Remove-Item -LiteralPath $PidPath, $PublicPidPath -Force -ErrorAction SilentlyContinue
        return $null
    }
    if (-not (Process-UsesExecutable $process $ExpectedExecutable)) {
        throw "Backend PID file points to a different process: $backendProcessId"
    }
    return $process
}

function Get-ManagedProxyProcess(
    [string]$PidPath,
    [string]$CommandPath,
    [string]$CurrentCommand
) {
    $proxyProcessId = Read-RecordedPid $PidPath 'Caddy'
    if ($null -eq $proxyProcessId) {
        return $null
    }
    $process = Get-Process -Id $proxyProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        Remove-Item -LiteralPath $PidPath, $CommandPath -Force -ErrorAction SilentlyContinue
        return $null
    }
    if (-not (Test-Path -LiteralPath $CommandPath -PathType Leaf)) {
        throw "Caddy command metadata is missing: $CommandPath"
    }
    $expectedCommand = (Get-Content -Raw -LiteralPath $CommandPath).Trim([char]0xfeff).Trim()
    if ([string]::IsNullOrWhiteSpace($expectedCommand)) {
        throw "Caddy command metadata is empty: $CommandPath"
    }
    if (-not [string]::Equals($expectedCommand, $CurrentCommand, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Caddy command changed while the public proxy is running. Stop it before using a different Caddy executable.'
    }
    if (-not (Process-UsesExecutable $process $expectedCommand)) {
        throw "Caddy PID file points to a different process: $proxyProcessId"
    }
    return $process
}

$root = Split-Path $PSScriptRoot -Parent
$runtimeDir = Join-Path $root '.runtime'
$dataDir = Join-Path $root 'backend\data'
$publicDir = Join-Path $dataDir 'public-proxy'
$caddyfile = Join-Path $publicDir 'Caddyfile'
$proxyPidFile = Join-Path $runtimeDir 'public-proxy.pid'
$proxyCommandFile = Join-Path $runtimeDir 'public-proxy.command'
$publicBackendPidFile = Join-Path $runtimeDir 'public-backend.pid'
$backendPidFile = Join-Path $runtimeDir 'backend.pid'
$backend = Join-Path $root 'backend\target-local\release\backend.exe'
$proxyLog = Join-Path $runtimeDir 'public-proxy.log'
$proxyErrorLog = Join-Path $runtimeDir 'public-proxy.err.log'
$startLocal = Join-Path $PSScriptRoot 'start-local.ps1'
$caddy = Resolve-CaddyCommand $CaddyCommand

New-Item -ItemType Directory -Force -Path $runtimeDir, $publicDir | Out-Null
$runningProxy = Get-ManagedProxyProcess $proxyPidFile $proxyCommandFile $caddy
$runningBackend = Get-RecordedBackendProcess $backendPidFile $publicBackendPidFile $backend
if ($null -ne $runningBackend) {
    $publicBackendPid = Read-RecordedPid $publicBackendPidFile 'public backend'
    if ($null -eq $publicBackendPid -or $publicBackendPid -ne $runningBackend.Id) {
        throw 'A local backend is already running outside public-proxy management. Stop it before starting the public HTTPS console.'
    }
}
if ($ResetCredentials -and $null -ne $runningProxy) {
    throw 'Stop the existing public proxy before resetting credentials.'
}

$createdCredentials = $false
$administratorPassword = ''
if ($ResetCredentials -or -not (Test-Path -LiteralPath $caddyfile -PathType Leaf)) {
    if ([string]::IsNullOrWhiteSpace($Domain)) {
        $Domain = Read-Host 'Public DNS hostname (for example console.example.com)'
    }
    if ([string]::IsNullOrWhiteSpace($Email)) {
        $Email = Read-Host 'ACME contact email'
    }
    if ([string]::IsNullOrWhiteSpace($Username)) {
        $Username = 'sculk'
    }
    $Domain = Require-Domain $Domain
    $Email = Require-Email $Email
    $Username = Require-Username $Username
    $administratorPassword = New-AdministratorPassword
    $passwordHash = (& $caddy 'hash-password' '--plaintext' $administratorPassword | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($passwordHash)) {
        throw 'Caddy could not create the administrator password hash.'
    }
    $config = @"
# sculk-public-domain: $Domain
# sculk-backend-port: $BackendPort
{
    email $Email
}

$Domain {
    basic_auth {
        $Username $passwordHash
    }
    reverse_proxy 127.0.0.1:$BackendPort
}
"@
    Set-Content -LiteralPath $caddyfile -Value $config -Encoding ASCII
    $createdCredentials = $true
} else {
    $configuredDomain = Select-String -LiteralPath $caddyfile -Pattern '^# sculk-public-domain:\s*(.+)$' |
        Select-Object -First 1
    if ($null -eq $configuredDomain) {
        throw "The managed Caddyfile does not contain a public DNS hostname: $caddyfile"
    }
    $Domain = Require-Domain $configuredDomain.Matches[0].Groups[1].Value
    $configuredPort = Select-String -LiteralPath $caddyfile -Pattern '^# sculk-backend-port:\s*(\d+)$' |
        Select-Object -First 1
    if ($null -eq $configuredPort) {
        $configuredPort = Select-String -LiteralPath $caddyfile -Pattern '^\s*reverse_proxy\s+127\.0\.0\.1:(\d+)\s*$' |
            Select-Object -First 1
    }
    if ($null -eq $configuredPort -or [int]$configuredPort.Matches[0].Groups[1].Value -ne $BackendPort) {
        throw 'The existing Caddyfile uses a different backend port. Stop the public proxy and reset credentials before changing it.'
    }
}

& $caddy 'validate' '--config' $caddyfile '--adapter' 'caddyfile'
if ($LASTEXITCODE -ne 0) {
    throw 'Caddyfile validation failed. Check the configured DNS hostname and Caddy installation.'
}

if ($null -eq $runningBackend) {
    $previousPublicProxyManaged = [Environment]::GetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', 'true', 'Process')
        & $startLocal -Port $BackendPort -Quiet
    } finally {
        [Environment]::SetEnvironmentVariable('SCULK_PUBLIC_PROXY_MANAGED', $previousPublicProxyManaged, 'Process')
    }
    $runningBackend = Get-RecordedBackendProcess $backendPidFile $publicBackendPidFile $backend
    if ($null -eq $runningBackend) {
        throw 'The loopback backend did not start.'
    }
    Set-Content -LiteralPath $publicBackendPidFile -Value $runningBackend.Id -Encoding ASCII
}

if ($null -eq $runningProxy) {
    $proxy = Start-Process -FilePath $caddy `
        -ArgumentList @('run', '--config', $caddyfile, '--adapter', 'caddyfile') `
        -WorkingDirectory $root `
        -RedirectStandardOutput $proxyLog `
        -RedirectStandardError $proxyErrorLog `
        -WindowStyle Hidden `
        -PassThru
    [System.IO.File]::WriteAllText(
        $proxyCommandFile,
        "$caddy`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    Set-Content -LiteralPath $proxyPidFile -Value $proxy.Id -Encoding ASCII
    Start-Sleep -Seconds 1
    if (-not (Get-Process -Id $proxy.Id -ErrorAction SilentlyContinue)) {
        Remove-Item -LiteralPath $proxyPidFile, $proxyCommandFile -Force -ErrorAction SilentlyContinue
        throw "Caddy exited during startup. Inspect $proxyErrorLog"
    }
}

if (-not $Quiet) {
    Write-Host "Public HTTPS console: https://$Domain"
    Write-Host 'The backend remains private on 127.0.0.1; do not expose port 8787 in the firewall or router.'
}
if ($createdCredentials) {
    Write-Host "Administrator username: $Username"
    Write-Host "Administrator password: $administratorPassword"
    Write-Host 'Save the password now. It is shown only once and is not stored in plaintext.'
}
