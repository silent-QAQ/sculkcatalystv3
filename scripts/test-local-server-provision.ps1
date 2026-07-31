# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

param(
    [string]$BackendExecutable = 'D:\projects\sculkcatalystv3\backend\target\debug\backend.exe',
    [string]$PythonExecutable = 'C:\Users\Administrator\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe',
    [string]$RustCompiler = 'rustc'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$stamp = Get-Date -Format 'yyyyMMddHHmmssfff'
$runtimeRoot = [IO.Path]::GetFullPath((Join-Path $root '.runtime'))
$testRoot = [IO.Path]::GetFullPath((Join-Path $runtimeRoot "provision-e2e-$stamp"))
if (-not $testRoot.StartsWith($runtimeRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Provisioning E2E path escaped the runtime directory.'
}

$dataRoot = Join-Path $testRoot 'data'
$stateFile = Join-Path $testRoot 'state.json'
$fixtureRoot = Join-Path $testRoot 'fixture'
$fakeJava = Join-Path $testRoot 'fake-java.exe'
$backendOut = Join-Path $testRoot 'backend.out.log'
$backendErr = Join-Path $testRoot 'backend.err.log'
$fileServerOut = Join-Path $testRoot 'file-server.out.log'
$fileServerErr = Join-Path $testRoot 'file-server.err.log'
$adminToken = "provision-e2e-$stamp-token"
$backendProcess = $null
$fileServerProcess = $null
$serverId = ''
$testPassed = $false

function Get-FreeTcpPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    try {
        $listener.Start()
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

$backendPort = Get-FreeTcpPort
do { $fileServerPort = Get-FreeTcpPort } while ($fileServerPort -eq $backendPort)
do { $minecraftPort = Get-FreeTcpPort } while ($minecraftPort -in @($backendPort, $fileServerPort))
$origin = "http://127.0.0.1:$backendPort"

function Invoke-TestApi {
    param(
        [Parameter(Mandatory)][string]$Method,
        [Parameter(Mandatory)][string]$Path,
        $Body,
        [switch]$Administrator
    )
    $parameters = @{
        Uri = "$origin$Path"
        Method = $Method
        TimeoutSec = 30
    }
    if ($Administrator) {
        $parameters.Headers = @{ Authorization = "Bearer $adminToken" }
    }
    if ($null -ne $Body) {
        $parameters.ContentType = 'application/json'
        $parameters.Body = $Body | ConvertTo-Json -Depth 12 -Compress
    }
    Invoke-RestMethod @parameters
}

function Wait-Backend {
    param([int]$Seconds = 30)
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri "$origin/api/health" -TimeoutSec 2
            if ([int]$response.StatusCode -eq 200) { return }
        }
        catch {}
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "Isolated backend did not become healthy on $origin."
}

function Start-IsolatedBackend {
    param([string]$LogSuffix)
    $variables = @{
        SCULK_BIND_ADDRESS = "127.0.0.1:$backendPort"
        SCULK_STATE_FILE = $stateFile
        SCULK_DATA_DIR = $dataRoot
        SCULK_JAVA_BIN = $fakeJava
        SCULK_CATALOG_ADMIN_TOKEN = $adminToken
        SCULK_MSL_CORE_SYNC_ENABLED = 'false'
        DATABASE_URL = $null
        REDIS_URL = $null
    }
    $previous = @{}
    foreach ($entry in $variables.GetEnumerator()) {
        $previous[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, [EnvironmentVariableTarget]::Process)
        [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, [EnvironmentVariableTarget]::Process)
    }
    try {
        $process = Start-Process `
            -FilePath $BackendExecutable `
            -WorkingDirectory (Join-Path $root 'backend') `
            -RedirectStandardOutput "$backendOut.$LogSuffix" `
            -RedirectStandardError "$backendErr.$LogSuffix" `
            -WindowStyle Hidden `
            -PassThru
    }
    finally {
        foreach ($entry in $previous.GetEnumerator()) {
            [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, [EnvironmentVariableTarget]::Process)
        }
    }
    Wait-Backend
    return $process
}

function Stop-TestProcess {
    param($Process)
    if ($Process -and -not $Process.HasExited) {
        Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
        $Process.WaitForExit(10000) | Out-Null
    }
}

function Wait-ProvisionTask {
    param(
        [Parameter(Mandatory)][string]$TaskId,
        [Parameter(Mandatory)][string[]]$Statuses,
        [int]$Seconds = 90
    )
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $dashboard = Invoke-TestApi -Method GET -Path '/api/dashboard'
        $task = @($dashboard.tasks | Where-Object { $_.id -eq $TaskId } | Select-Object -First 1)
        if ($task.Count -and $Statuses -contains [string]$task[0].status) {
            return $task[0]
        }
        if ($task.Count -and @('completed', 'failed', 'cancelled', 'interrupted', 'rollback_failed') -contains [string]$task[0].status) {
            throw "Provision task reached unexpected status $($task[0].status): $($task[0].error)"
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "Provision task did not reach $($Statuses -join ', ')."
}

New-Item -ItemType Directory -Force -Path $dataRoot, $fixtureRoot | Out-Null
try {
    if (-not (Test-Path -LiteralPath $BackendExecutable)) {
        throw "Backend executable is missing: $BackendExecutable"
    }
    if (-not (Test-Path -LiteralPath $PythonExecutable)) {
        throw "Python executable is missing: $PythonExecutable"
    }

    & $RustCompiler --edition 2024 -C opt-level=s -o $fakeJava (Join-Path $PSScriptRoot 'fixtures\fake-java.rs')
    if ($LASTEXITCODE -ne 0) { throw 'Unable to compile the fake Java lifecycle fixture.' }

    $jarPath = Join-Path $fixtureRoot 'server.jar'
    $jarStream = [IO.File]::Open($jarPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $jarStream.SetLength(8 * 1024 * 1024)
        $marker = [Text.Encoding]::UTF8.GetBytes('sculk-provision-e2e')
        $jarStream.Position = 0
        $jarStream.Write($marker, 0, $marker.Length)
        $jarStream.Flush($true)
    }
    finally {
        $jarStream.Dispose()
    }
    $jarItem = Get-Item -LiteralPath $jarPath
    $jarHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $jarPath).Hash.ToLowerInvariant()

    $fileServerProcess = Start-Process `
        -FilePath $PythonExecutable `
        -ArgumentList @(
            (Join-Path $PSScriptRoot 'fixtures\slow_file_server.py'),
            '--root', $fixtureRoot,
            '--port', [string]$fileServerPort,
            '--delay-ms', '15'
        ) `
        -WorkingDirectory $root `
        -RedirectStandardOutput $fileServerOut `
        -RedirectStandardError $fileServerErr `
        -WindowStyle Hidden `
        -PassThru
    Start-Sleep -Milliseconds 500

    $backendProcess = Start-IsolatedBackend -LogSuffix 'first'
    $versions = @(Invoke-TestApi -Method GET -Path '/api/catalog/cores/paper/versions')
    $catalogVersion = $versions |
        Where-Object { @($_.minecraft_versions) -contains '1.21.4' } |
        Select-Object -First 1
    if (-not $catalogVersion) { throw 'Seed catalog does not contain a Paper 1.21.4 version.' }
    $versionPath = [Uri]::EscapeDataString([string]$catalogVersion.version)
    Invoke-TestApi -Method PUT -Path "/api/catalog/cores/paper/versions/$versionPath" -Administrator -Body @{
        version = [string]$catalogVersion.version
        channel = 'stable'
        minecraft_versions = @('1.21.4')
        loaders = @('paper')
        formats = @()
        java_version = 21
        filename = 'server.jar'
        size = [uint64]$jarItem.Length
        sha256 = $jarHash
        download_url = "http://127.0.0.1:$fileServerPort/server.jar"
        content = ''
        release_notes = 'Provisioning E2E local fixture'
        released_at = (Get-Date).ToUniversalTime().ToString('o')
        status = 'published'
    } | Out-Null

    $created = Invoke-TestApi -Method POST -Path '/api/servers' -Body @{
        name = 'Provisioning E2E'
        core = 'paper'
        version = '1.21.4'
        memory_gb = 2
        port = $minecraftPort
        eula_accepted = $true
        location = 'local'
    }
    $serverId = [string]$created.server.id
    $taskId = [string]$created.provision_task.id
    if (-not $serverId -or -not $taskId -or $created.provision_task.kind -ne 'server_provision') {
        throw 'Create server did not return a real server_provision task.'
    }

    $running = Wait-ProvisionTask -TaskId $taskId -Statuses @('running') -Seconds 20
    Stop-TestProcess -Process $backendProcess
    $backendProcess = Start-IsolatedBackend -LogSuffix 'second'
    $completed = Wait-ProvisionTask -TaskId $taskId -Statuses @('completed') -Seconds 90
    if (-not (@($completed.events).message -match '重启|恢复|重新|Interrupted provisioning|returned to queue')) {
        throw 'Recovered provision task did not retain a restart recovery event.'
    }

    $dashboard = Invoke-TestApi -Method GET -Path '/api/dashboard'
    $server = $dashboard.servers | Where-Object { $_.id -eq $serverId } | Select-Object -First 1
    $lastErrorProperty = $server.PSObject.Properties['last_error']
    $lastError = if ($lastErrorProperty) { $lastErrorProperty.Value } else { $null }
    if (-not $server.core_ready -or $server.operation_state -ne 'idle' -or $lastError) {
        throw 'Provisioned server did not reach an idle, core-ready state.'
    }
    $installedJar = Join-Path $dataRoot "servers\$serverId\server.jar"
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $installedJar).Hash.ToLowerInvariant() -ne $jarHash) {
        throw 'Installed server.jar does not match the catalog fixture hash.'
    }

    $idempotent = Invoke-TestApi -Method POST -Path "/api/servers/$serverId/provision"
    if ($idempotent.server.id -ne $serverId -or
        $idempotent.provision_task.id -ne $taskId -or
        $idempotent.provision_task.status -ne 'completed') {
        throw 'Completed provisioning retry was not idempotent.'
    }

    Invoke-TestApi -Method POST -Path "/api/servers/$serverId/action" -Body @{ action = 'start' } | Out-Null
    $onlineDeadline = (Get-Date).AddSeconds(20)
    do {
        $dashboard = Invoke-TestApi -Method GET -Path '/api/dashboard'
        $server = $dashboard.servers | Where-Object { $_.id -eq $serverId } | Select-Object -First 1
        if ($server.status -eq 'online' -and $server.operation_state -eq 'idle') { break }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $onlineDeadline)
    if ($server.status -ne 'online') { throw 'Provisioned server did not reach the online marker.' }

    $commandResult = Invoke-TestApi -Method POST -Path "/api/servers/$serverId/command" -Body @{ command = 'list' }
    if (-not (@($commandResult.lines) -match 'list')) { throw 'Server command did not reach the managed process.' }
    Invoke-TestApi -Method POST -Path "/api/servers/$serverId/action" -Body @{ action = 'stop' } | Out-Null
    $dashboard = Invoke-TestApi -Method GET -Path '/api/dashboard'
    $server = $dashboard.servers | Where-Object { $_.id -eq $serverId } | Select-Object -First 1
    if ($server.status -ne 'stopped' -or $server.operation_state -ne 'idle') {
        throw 'Managed server did not finish in the stopped idle state.'
    }

    [pscustomobject]@{
        ProvisionTask = $completed.status
        RestartRecovered = $true
        CoreReady = $server.core_ready
        InstalledSha256 = $jarHash
        LifecycleStartStop = 'online -> stopped'
        CommandForwarded = $true
    } | Format-List
    $testPassed = $true
}
finally {
    if ($serverId -and $backendProcess -and -not $backendProcess.HasExited) {
        try { Invoke-TestApi -Method POST -Path "/api/servers/$serverId/action" -Body @{ action = 'force_stop' } | Out-Null } catch {}
    }
    Stop-TestProcess -Process $backendProcess
    Stop-TestProcess -Process $fileServerProcess
    if (Test-Path -LiteralPath $fakeJava) {
        Get-CimInstance Win32_Process |
            Where-Object { $_.ExecutablePath -eq $fakeJava } |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
    if ($testPassed -and [IO.Directory]::Exists($testRoot)) {
        [IO.Directory]::Delete($testRoot, $true)
    }
    elseif (-not $testPassed) {
        Write-Error "Provisioning E2E files were retained at $testRoot" -ErrorAction Continue
    }
}
