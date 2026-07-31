# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

param(
    [string]$CloudOrigin = 'http://127.0.0.1:8788',
    [string]$AgentExecutable = 'D:\projects\sculkcatalystv3\agent\target\release\sculk-agent.exe',
    [string]$PostgresExecutable = 'D:\PostgreSQL\18\bin\psql.exe',
    [ValidateSet('windows', 'wsl')][string]$AgentPlatform = 'windows',
    [string]$WslDistribution = 'Ubuntu-24.04',
    [string]$LinuxAgentExecutable = '/mnt/d/projects/sculkcatalystv3/agent/target/x86_64-unknown-linux-musl/release/sculk-agent'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$stamp = Get-Date -Format 'yyyyMMddHHmmssfff'
$email = "codex-agent-e2e-$stamp@example.com"
$password = "Agent-E2E-$stamp!Aa"
$testRoot = Join-Path $root ".runtime\agent-e2e-$stamp"
$workspace = Join-Path $testRoot 'workspace'
$config = Join-Path $testRoot 'agent.json'
$stdout = Join-Path $testRoot 'agent.out.log'
$stderr = Join-Path $testRoot 'agent.err.log'
$linuxWorkspace = $workspace.Replace('D:\', '/mnt/d/').Replace('\', '/')
$linuxConfig = $config.Replace('D:\', '/mnt/d/').Replace('\', '/')
$script:session = $null
$agentProcess = $null
$testPassed = $false

function Invoke-CloudApi {
    param(
        [Parameter(Mandatory)][string]$Method,
        [Parameter(Mandatory)][string]$Path,
        $Body,
        [string]$Token
    )
    $params = @{
        Uri = "$CloudOrigin$Path"
        Method = $Method
        TimeoutSec = 30
    }
    if ($Token) {
        $params.Headers = @{ Authorization = "Bearer $Token" }
    }
    if ($null -ne $Body) {
        $params.ContentType = 'application/json'
        $params.Body = $Body | ConvertTo-Json -Depth 12 -Compress
    }
    Invoke-RestMethod @params
}

function Wait-AgentTask {
    param(
        [Parameter(Mandatory)][string]$TaskId,
        [Parameter(Mandatory)][string[]]$Statuses,
        [int]$Seconds = 90
    )
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $taskResponse = Invoke-CloudApi -Method GET -Path '/api/cloud/agent-tasks' -Token $script:session.access_token
        $tasks = @($taskResponse)
        $matches = @($tasks | Where-Object { $_.id -eq $TaskId })
        $found = if ($matches.Count -gt 0) { $matches[0] } else { $null }
        $foundStatus = if ($found) { [string]$found.status } else { '' }
        if ($found -and $Statuses -contains $foundStatus) {
            return $found
        }
        if ($found -and @('succeeded', 'failed', 'cancelled') -contains $foundStatus) {
            $eventSummary = @($found.events | ForEach-Object { "#$($_.seq) $($_.message)" }) -join '; '
            throw "Task $TaskId reached unexpected status ${foundStatus}: $($found.error). Events: $eventSummary"
        }
        Start-Sleep -Milliseconds 750
    } while ((Get-Date) -lt $deadline)
    throw "Task $TaskId remained ${foundStatus} instead of reaching $($Statuses -join ',')"
}

function Wait-WorkspacePath {
    param(
        [Parameter(Mandatory)][string]$Path,
        [int]$Seconds = 15
    )
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        if (Test-Path -LiteralPath $Path) {
            return
        }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "Workspace path did not appear: $Path"
}

function Wait-ChildProcessExit {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [int]$Seconds = 10
    )
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $alive = if ($AgentPlatform -eq 'windows') {
            $null -ne (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)
        }
        else {
            & wsl.exe -d $WslDistribution -- sh -lc "kill -0 $ProcessId 2>/dev/null"
            $LASTEXITCODE -eq 0
        }
        if (-not $alive) {
            return
        }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "Cancelled task child process $ProcessId is still alive."
}

function Remove-TestAccount {
    $envFile = Join-Path $root '.env'
    foreach ($line in Get-Content -LiteralPath $envFile) {
        if (-not $line -or $line.StartsWith('#')) {
            continue
        }
        $parts = $line -split '=', 2
        if ($parts.Count -eq 2) {
            [Environment]::SetEnvironmentVariable(
                $parts[0],
                $parts[1],
                [EnvironmentVariableTarget]::Process
            )
        }
    }
    $safeEmail = $email.Replace("'", "''")
    $cleanupSql = @"
BEGIN;
DELETE FROM cloud_agent_task_events
WHERE task_id IN (
    SELECT id FROM cloud_agent_tasks
    WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail')
);
DELETE FROM cloud_agent_task_checkpoints
WHERE task_id IN (
    SELECT id FROM cloud_agent_tasks
    WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail')
);
DELETE FROM cloud_agent_tasks
WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail');
DELETE FROM cloud_users WHERE email = '$safeEmail';
COMMIT;
"@
    & $PostgresExecutable $env:DATABASE_URL -v ON_ERROR_STOP=1 -q -c $cleanupSql | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Unable to remove the disposable Cloud test account.'
    }
}

New-Item -ItemType Directory -Path $workspace -Force | Out-Null
try {
    $script:session = Invoke-CloudApi -Method POST -Path '/api/cloud/auth/register' -Body @{
        email = $email
        password = $password
        nickname = 'Agent E2E'
        device_name = 'E2E'
        platform = $AgentPlatform
    }
    $pairing = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-pairings' -Token $script:session.access_token
    if ($AgentPlatform -eq 'windows') {
        & $AgentExecutable pair `
            --cloud $CloudOrigin `
            --code $pairing.pairing_code `
            --name 'e2e-host' `
            --workspace 'e2e' `
            --workspace-root $workspace `
            --permissions full `
            --capabilities 'heartbeat,tasks-v1,task-checkpoints-v1,shell-v1' `
            --config $config | Out-Null
    }
    else {
        & wsl.exe -d $WslDistribution -- $LinuxAgentExecutable pair `
            --cloud $CloudOrigin `
            --code $pairing.pairing_code `
            --name 'e2e-host' `
            --workspace 'e2e' `
            --workspace-root $linuxWorkspace `
            --permissions full `
            --capabilities 'heartbeat,tasks-v1,task-checkpoints-v1,shell-v1' `
            --config $linuxConfig | Out-Null
    }

    $agentResponse = Invoke-CloudApi -Method GET -Path '/api/cloud/agents' -Token $script:session.access_token
    $claimed = @($agentResponse) |
        Where-Object { $_.status -eq 'claimed' } |
        Select-Object -First 1
    if (-not $claimed) {
        throw 'Claimed Agent was not listed.'
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agents/$($claimed.id)/confirm" -Token $script:session.access_token | Out-Null
    if ($AgentPlatform -eq 'windows') {
        $agentProcess = Start-Process `
            -FilePath $AgentExecutable `
            -ArgumentList @('run', '--config', $config) `
            -RedirectStandardOutput $stdout `
            -RedirectStandardError $stderr `
            -WindowStyle Hidden `
            -PassThru
    }
    else {
        $agentProcess = Start-Process `
            -FilePath 'wsl.exe' `
            -ArgumentList @('-d', $WslDistribution, '--', $LinuxAgentExecutable, 'run', '--config', $linuxConfig) `
            -RedirectStandardOutput $stdout `
            -RedirectStandardError $stderr `
            -WindowStyle Hidden `
            -PassThru
    }

    $onlineDeadline = (Get-Date).AddSeconds(30)
    do {
        $agentResponse = Invoke-CloudApi -Method GET -Path '/api/cloud/agents' -Token $script:session.access_token
        $active = @($agentResponse) |
            Where-Object { $_.id -eq $claimed.id } |
            Select-Object -First 1
        if ($active.online) {
            break
        }
        Start-Sleep -Milliseconds 750
    } while ((Get-Date) -lt $onlineDeadline)
    if (-not $active.online) {
        throw 'Agent did not become online.'
    }

    $inspect = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-tasks' -Token $script:session.access_token -Body @{
        agent_id = $claimed.id
        operation = 'host.inspect'
        input = @{}
        idempotency_key = "inspect-$stamp"
    }
    $inspect = Wait-AgentTask -TaskId $inspect.id -Statuses @('succeeded')
    if ($inspect.output.workspace_label -ne 'e2e') {
        throw 'host.inspect output is incomplete.'
    }

    $shellCommand = if ($AgentPlatform -eq 'windows') { @'
New-Item -ItemType Directory -Force -Path 'server' | Out-Null; [IO.File]::WriteAllText((Join-Path (Get-Location) 'server\server.properties'), "motd=Before`nmax-players=20`n"); [IO.File]::AppendAllText((Join-Path (Get-Location) 'checkpoint-marker.txt'), "once`n"); Write-Output 'shell-ok'
'@
    } else { @'
mkdir -p server && printf 'motd=Before\nmax-players=20\n' > server/server.properties && printf 'once\n' >> checkpoint-marker.txt && printf 'shell-ok\n'
'@
    }
    $shell = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-tasks' -Token $script:session.access_token -Body @{
        agent_id = $claimed.id
        operation = 'shell.exec'
        input = @{ command = $shellCommand; timeout_seconds = 60 }
        idempotency_key = "shell-$stamp"
    }
    if ($shell.status -ne 'awaiting_approval') {
        throw 'Shell task did not require approval.'
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($shell.id)/approve" -Token $script:session.access_token | Out-Null
    $shell = Wait-AgentTask -TaskId $shell.id -Statuses @('succeeded')
    if ($shell.output.stdout -notmatch 'shell-ok') {
        throw 'Shell stdout was not captured.'
    }
    $propertiesPath = Join-Path $workspace 'server\server.properties'
    if (-not (Test-Path -LiteralPath $propertiesPath)) {
        throw 'Shell did not write the workspace file.'
    }
    if (-not $shell.can_resume -or -not $shell.latest_checkpoint.resumable) {
        throw 'Successful task did not expose a resumable result checkpoint.'
    }
    $markerPath = Join-Path $workspace 'checkpoint-marker.txt'
    $markerLines = @(Get-Content -LiteralPath $markerPath)
    if ($markerLines.Count -ne 1) {
        throw 'Original checkpoint test command did not execute exactly once.'
    }

    $resumed = Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($shell.id)/retry" `
        -Token $script:session.access_token -Body @{
            mode = 'resume'
            idempotency_key = "resume-$stamp"
        }
    if ($resumed.status -ne 'awaiting_approval' -or $resumed.execution_mode -ne 'resume') {
        throw 'Checkpoint recovery did not create a new approval-gated resume attempt.'
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($resumed.id)/approve" -Token $script:session.access_token | Out-Null
    $resumed = Wait-AgentTask -TaskId $resumed.id -Statuses @('succeeded')
    $markerLines = @(Get-Content -LiteralPath $markerPath)
    if ($markerLines.Count -ne 1) {
        throw 'Checkpoint recovery repeated an already completed side effect.'
    }
    $checkpointPreventedDuplicate = $markerLines.Count -eq 1
    if ($resumed.lineage_id -ne $shell.lineage_id -or $resumed.attempt_no -ne 2) {
        throw 'Checkpoint recovery did not retain task lineage and attempt numbering.'
    }

    $restarted = Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($resumed.id)/retry" `
        -Token $script:session.access_token -Body @{
            mode = 'restart'
            idempotency_key = "restart-$stamp"
        }
    if ($restarted.status -ne 'awaiting_approval' -or $restarted.execution_mode -ne 'restart') {
        throw 'Restart did not create a new approval-gated attempt.'
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($restarted.id)/approve" -Token $script:session.access_token | Out-Null
    $restarted = Wait-AgentTask -TaskId $restarted.id -Statuses @('succeeded')
    $markerLines = @(Get-Content -LiteralPath $markerPath)
    if ($markerLines.Count -ne 2) {
        throw 'Restart did not execute the operation exactly one additional time.'
    }
    if ($restarted.lineage_id -ne $shell.lineage_id -or $restarted.attempt_no -ne 3) {
        throw 'Restart did not advance the task attempt number.'
    }

    $cancelPidPath = Join-Path $workspace 'cancel-child.pid'
    $cancelTailPath = Join-Path $workspace 'cancel-should-not-exist.txt'
    $cancelCommand = if ($AgentPlatform -eq 'windows') { @'
Start-Sleep -Seconds 2; $child = Start-Process powershell.exe -WindowStyle Hidden -ArgumentList @('-NoLogo', '-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 60') -PassThru; [IO.File]::WriteAllText((Join-Path (Get-Location) 'cancel-child.pid'), [string]$child.Id); Wait-Process -Id $child.Id; [IO.File]::WriteAllText((Join-Path (Get-Location) 'cancel-should-not-exist.txt'), 'bad')
'@
    } else { @'
sleep 60 & child=$!; printf '%s' "$child" > cancel-child.pid; wait "$child"; printf bad > cancel-should-not-exist.txt
'@
    }
    $cancelled = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-tasks' -Token $script:session.access_token -Body @{
        agent_id = $claimed.id
        operation = 'shell.exec'
        input = @{ command = $cancelCommand; timeout_seconds = 90 }
        idempotency_key = "cancel-$stamp"
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($cancelled.id)/approve" -Token $script:session.access_token | Out-Null
    $cancelled = Wait-AgentTask -TaskId $cancelled.id -Statuses @('running')
    Wait-WorkspacePath -Path $cancelPidPath
    $cancelChildPid = [int]((Get-Content -LiteralPath $cancelPidPath -Raw).Trim())
    $cancelRequestedAt = Get-Date
    $cancelRequest = Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($cancelled.id)/cancel" -Token $script:session.access_token
    if ($cancelRequest.status -ne 'running' -or -not $cancelRequest.cancel_requested) {
        throw 'Running Shell cancellation was not recorded as a pending Agent request.'
    }
    $cancelled = Wait-AgentTask -TaskId $cancelled.id -Statuses @('cancelled') -Seconds 20
    $cancelLatencyMs = [int]((Get-Date) - $cancelRequestedAt).TotalMilliseconds
    if (-not $cancelled.cancel_requested_at -or -not $cancelled.cancel_acknowledged_at) {
        throw 'Cancelled task did not retain both request and Agent acknowledgement timestamps.'
    }
    if ($cancelled.rollback_available) {
        throw 'Cancelled Shell task must not advertise rollback support.'
    }
    Wait-ChildProcessExit -ProcessId $cancelChildPid
    Start-Sleep -Milliseconds 500
    if (Test-Path -LiteralPath $cancelTailPath) {
        throw 'Cancelled Shell continued executing after the process tree should have terminated.'
    }

    $update = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-tasks' -Token $script:session.access_token -Body @{
        agent_id = $claimed.id
        operation = 'server.properties.update'
        input = @{
            path = 'server/server.properties'
            changes = @{ motd = 'After'; 'max-players' = 30 }
        }
        idempotency_key = "update-$stamp"
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($update.id)/approve" -Token $script:session.access_token | Out-Null
    $update = Wait-AgentTask -TaskId $update.id -Statuses @('succeeded')
    if (-not $update.rollback_available) {
        throw 'Structured update did not expose rollback.'
    }
    $updatedText = Get-Content -LiteralPath $propertiesPath -Raw
    if ($updatedText -notmatch 'motd=After' -or $updatedText -notmatch 'max-players=30') {
        throw 'Structured update did not apply.'
    }

    $rollback = Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($update.id)/rollback" -Token $script:session.access_token
    if ($rollback.status -ne 'awaiting_approval') {
        throw 'Rollback task did not require approval.'
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($rollback.id)/approve" -Token $script:session.access_token | Out-Null
    $rollback = Wait-AgentTask -TaskId $rollback.id -Statuses @('succeeded')
    $restoredText = Get-Content -LiteralPath $propertiesPath -Raw
    if ($restoredText -notmatch 'motd=Before' -or $restoredText -notmatch 'max-players=20') {
        throw 'Rollback did not restore the original file.'
    }

    $failed = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-tasks' -Token $script:session.access_token -Body @{
        agent_id = $claimed.id
        operation = 'shell.exec'
        input = @{
            command = if ($AgentPlatform -eq 'windows') { "Write-Error 'expected-failure'; exit 7" } else { "printf 'expected-failure\n' >&2; exit 7" }
            timeout_seconds = 30
        }
        idempotency_key = "failed-$stamp"
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($failed.id)/approve" -Token $script:session.access_token | Out-Null
    $failed = Wait-AgentTask -TaskId $failed.id -Statuses @('failed')
    if ($failed.error -notmatch '7') {
        throw 'Failed Shell task did not preserve the exit status.'
    }

    [pscustomobject]@{
        AgentOnline = $active.online
        InspectStatus = $inspect.status
        ShellStatus = $shell.status
        ShellEvents = @($shell.events).Count
        CheckpointResumeStatus = $resumed.status
        RestartStatus = $restarted.status
        CheckpointPreventedDuplicate = $checkpointPreventedDuplicate
        AttemptCount = $restarted.attempt_no
        CancelledStatus = $cancelled.status
        CancelLatencyMs = $cancelLatencyMs
        CancelledChildPid = $cancelChildPid
        UpdateStatus = $update.status
        RollbackStatus = $rollback.status
        FailedStatus = $failed.status
        FailedExit = $failed.output.exit_code
    } | Format-List
    $testPassed = $true
}
finally {
    if ($agentProcess -and -not $agentProcess.HasExited) {
        Stop-Process -Id $agentProcess.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-TestAccount
    $resolvedRuntime = [IO.Path]::GetFullPath((Join-Path $root '.runtime'))
    $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
    if ($testPassed -and
        $resolvedTestRoot.StartsWith($resolvedRuntime + [IO.Path]::DirectorySeparatorChar) -and
        [IO.Path]::GetFileName($resolvedTestRoot).StartsWith('agent-e2e-') -and
        [IO.Directory]::Exists($resolvedTestRoot)) {
        [IO.Directory]::Delete($resolvedTestRoot, $true)
    }
    elseif (-not $testPassed) {
        Write-Error "Agent E2E logs were retained at $resolvedTestRoot" -ErrorAction Continue
    }
}
