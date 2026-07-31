# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$postgres = Get-Service -Name 'sculk-postgresql-18'
$taskStates = @{}
foreach ($taskName in @('Sculk-Garnet', 'Sculk-Cloud-Backend', 'Sculk-Caddy')) {
    $taskStates[$taskName] = (Get-ScheduledTask -TaskName $taskName).State
}

$cloudAvailable = $false
try {
    $status = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:8788/api/cloud/status' `
        -TimeoutSec 3
    $cloudAvailable = [bool]$status.available
} catch {}

$publicReachable = $false
$publicStatusCode = $null
try {
    $publicResponse = Invoke-WebRequest `
        -Uri 'https://sculk.mcmy.love/api/cloud/status' `
        -UseBasicParsing `
        -TimeoutSec 8
    $publicStatusCode = [int]$publicResponse.StatusCode
    $publicReachable = $publicStatusCode -eq 200
} catch {
    if ($_.Exception.Response) {
        $publicStatusCode = [int]$_.Exception.Response.StatusCode
    }
}

$certificate = Get-ChildItem `
    -LiteralPath 'D:\projects\sculkcatalystv3\.runtime\caddy-data' `
    -Recurse `
    -File `
    -Filter '*.crt' `
    -ErrorAction SilentlyContinue |
    Select-Object -First 1

[pscustomobject]@{
    PostgreSQL = $postgres.Status
    Garnet = $taskStates['Sculk-Garnet']
    Backend = $taskStates['Sculk-Cloud-Backend']
    Caddy = $taskStates['Sculk-Caddy']
    CloudAvailable = $cloudAvailable
    LocalBackend = 'http://127.0.0.1:8788'
    PublicUrl = 'https://sculk.mcmy.love'
    OriginCertificateReady = [bool]$certificate
    PublicEndpointReachable = $publicReachable
    PublicStatusCode = $publicStatusCode
} | Format-List
