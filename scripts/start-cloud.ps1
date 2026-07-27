# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

param([switch]$Quiet)

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
    $env:SCULK_STATE_FILE = 'data/state-cloud.json'
    Start-Process -FilePath $backend `
        -WorkingDirectory (Join-Path $root 'backend') `
        -RedirectStandardOutput (Join-Path $runtime 'backend.log') `
        -RedirectStandardError (Join-Path $runtime 'backend.err.log') `
        -WindowStyle Hidden
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
