# SPDX-License-Identifier: Apache-2.0

param([int]$Port = 8799)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$runtimeRoot = Join-Path $root '.runtime'
$smokeRoot = Join-Path $runtimeRoot ("smoke-bootstrap-" + [guid]::NewGuid().ToString('N'))
$backend = Join-Path $root 'backend\target-local\release\backend.exe'
$staticDir = Join-Path $root 'frontend\dist'
$backendDir = Join-Path $root 'backend'
$process = $null

New-Item -ItemType Directory -Path $smokeRoot | Out-Null
try {
    $env:SCULK_BIND_ADDRESS = "127.0.0.1:$Port"
    $env:SCULK_STATIC_DIR = $staticDir
    $env:SCULK_DATA_DIR = $smokeRoot
    $env:SCULK_STATE_FILE = Join-Path $smokeRoot 'state.json'

    $process = Start-Process -FilePath $backend `
        -WorkingDirectory $backendDir `
        -RedirectStandardOutput (Join-Path $smokeRoot 'stdout.log') `
        -RedirectStandardError (Join-Path $smokeRoot 'stderr.log') `
        -WindowStyle Hidden `
        -PassThru

    $ready = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 300
        try {
            Invoke-RestMethod "http://127.0.0.1:$Port/api/health" -TimeoutSec 2 | Out-Null
            $ready = $true
            break
        } catch {}
    }
    if (-not $ready) {
        throw 'Isolated backend did not become ready.'
    }

    $plan = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$Port/api/servers/plan" `
        -Method Post `
        -ContentType 'application/json' `
        -Body '{"name":"bootstrap-smoke","location":"local"}'

    $planningBody = @{
        server_id = $plan.server.id
        conversation_id = $plan.conversation.id
        message = 'Paper 26.2 plugin survival server, about 10 players'
        history = @()
        agent_override = 'default'
    } | ConvertTo-Json -Depth 6
    Invoke-WebRequest `
        -Uri "http://127.0.0.1:$Port/api/chat/stream" `
        -Method Post `
        -ContentType 'application/json' `
        -Body ([Text.Encoding]::UTF8.GetBytes($planningBody)) `
        -TimeoutSec 45 | Out-Null

    $bootstrapBody = @{
        server_id = $plan.server.id
        conversation_id = $plan.conversation.id
        message = "$([char]0x7EE7)$([char]0x7EED)"
        history = @()
        agent_override = 'default'
    } | ConvertTo-Json -Depth 6
    $bootstrapResponse = Invoke-WebRequest `
        -Uri "http://127.0.0.1:$Port/api/chat/stream" `
        -Method Post `
        -ContentType 'application/json' `
        -Body ([Text.Encoding]::UTF8.GetBytes($bootstrapBody)) `
        -TimeoutSec 45

    if ($bootstrapResponse.Content -notmatch '"executor":true') {
        throw 'Plan confirmation was sent back to the model instead of the task executor.'
    }

    $dashboard = Invoke-RestMethod "http://127.0.0.1:$Port/api/dashboard" -TimeoutSec 5
    $server = @($dashboard.servers | Where-Object id -eq $plan.server.id)[0]
    $task = @($dashboard.tasks | Where-Object server_id -eq $plan.server.id)[0]
    $coreResourceId = if ($server.PSObject.Properties.Name -contains 'core_resource_id') {
        [string]$server.core_resource_id
    } else {
        ''
    }
    if ($server.core -ne 'Paper' -or $coreResourceId -or $server.version -ne '26.2' -or $server.port -ne 25565) {
        throw "Bare continuation did not reuse the planning conversation: '$($server.core) [$coreResourceId] $($server.version)' on port $($server.port), expected Paper [no catalog id] 26.2 on 25565."
    }
    if ($server.memory_gb -ne 4) {
        throw 'Ten-player plugin server did not receive the conservative 4 GB default.'
    }
    if ($task.kind -ne 'server_bootstrap' -or $task.status -ne 'awaiting_approval') {
        throw 'Chat did not create an approval-gated server_bootstrap task.'
    }
    if (Test-Path -LiteralPath (Join-Path $smokeRoot "servers\$($server.id)")) {
        throw 'Approval-gated task changed the server filesystem before approval.'
    }

    [pscustomobject]@{
        Health = 'ok'
        Core = $server.core
        CoreResourceId = $coreResourceId
        Version = $server.version
        MemoryGB = $server.memory_gb
        Port = $server.port
        TaskKind = $task.kind
        TaskStatus = $task.status
        FilesCreated = $false
    }
} finally {
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
    $resolvedRuntime = [IO.Path]::GetFullPath($runtimeRoot)
    $resolvedSmoke = [IO.Path]::GetFullPath($smokeRoot)
    if ($resolvedSmoke.StartsWith($resolvedRuntime, [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolvedSmoke -Recurse -Force
    }
}
