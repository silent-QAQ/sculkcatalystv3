# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

param(
    [string]$CloudOrigin = 'http://127.0.0.1:8788',
    [string]$AgentExecutable = 'D:\projects\sculkcatalystv3\agent\target\release\sculk-agent.exe',
    [string]$PostgresExecutable = 'D:\PostgreSQL\18\bin\psql.exe',
    [ValidateSet('windows', 'wsl')][string]$AgentPlatform = 'windows',
    [string]$WslDistribution = 'Ubuntu-24.04',
    [string]$LinuxAgentExecutable = '/mnt/d/projects/sculkcatalystv3/agent/target/x86_64-unknown-linux-musl/release/sculk-agent',
    [string]$ApprovalTeamId,
    [string]$ApproverToken
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$stamp = Get-Date -Format 'yyyyMMddHHmmssfff'
$email = "codex-terminal-e2e-$stamp@example.com"
$password = "Terminal-E2E-$stamp!Aa"
$testRoot = Join-Path $root ".runtime\terminal-e2e-$stamp"
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

function ConvertTo-TerminalBase64 {
    param([Parameter(Mandatory)][string]$Text)
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Text))
}

function Get-TerminalOutput {
    param($Events)
    $builder = [Text.StringBuilder]::new()
    foreach ($event in @($Events)) {
        if ($event.kind -eq 'output' -and $event.data_base64) {
            [void]$builder.Append([Text.Encoding]::UTF8.GetString(
                [Convert]::FromBase64String([string]$event.data_base64)
            ))
        }
    }
    $builder.ToString()
}

function Wait-AgentOnline {
    param([Parameter(Mandatory)][string]$AgentId, [int]$Seconds = 30)
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $agents = @(Invoke-CloudApi -Method GET -Path '/api/cloud/agents' -Token $script:session.access_token)
        $agent = $agents | Where-Object { $_.id -eq $AgentId } | Select-Object -First 1
        if ($agent -and $agent.online) { return $agent }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    throw 'Agent did not become online.'
}

function Wait-AgentTask {
    param(
        [Parameter(Mandatory)][string]$TaskId,
        [Parameter(Mandatory)][string[]]$Statuses,
        [int]$Seconds = 90
    )
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $task = Invoke-CloudApi -Method GET -Path "/api/cloud/agent-tasks/$TaskId" -Token $script:session.access_token
        if ($Statuses -contains [string]$task.status) { return $task }
        if (@('succeeded', 'failed', 'cancelled') -contains [string]$task.status) {
            throw "Task $TaskId reached unexpected status $($task.status): $($task.error)"
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    throw "Task $TaskId did not reach $($Statuses -join ',')."
}

function Wait-TerminalSession {
    param(
        [Parameter(Mandatory)][string]$SessionId,
        [Parameter(Mandatory)][string[]]$Statuses,
        [string]$OutputPattern = '',
        [int]$Seconds = 45
    )
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        $response = Invoke-CloudApi -Method GET -Path "/api/cloud/terminal-sessions/$SessionId/events?after_seq=0&limit=500" -Token $script:session.access_token
        $output = Get-TerminalOutput -Events $response.events
        if (($Statuses -contains [string]$response.session.status) -and
            (-not $OutputPattern -or $output -match $OutputPattern)) {
            return [pscustomobject]@{ Session = $response.session; Events = @($response.events); Output = $output }
        }
        if (@('exited', 'failed', 'cancelled') -contains [string]$response.session.status -and
            -not ($Statuses -contains [string]$response.session.status)) {
            throw "Terminal reached unexpected status $($response.session.status): $($response.session.error)"
        }
        Start-Sleep -Milliseconds 300
    } while ((Get-Date) -lt $deadline)
    throw "Terminal $SessionId did not reach $($Statuses -join ',') with output '$OutputPattern'."
}

function Approve-LinkedResource {
    param(
        [Parameter(Mandatory)][string]$ApprovalId,
        [Parameter(Mandatory)][string]$ResourceId,
        [Parameter(Mandatory)][string]$ResourceField
    )
    if ([string]::IsNullOrWhiteSpace($ApprovalTeamId) -or [string]::IsNullOrWhiteSpace($ApproverToken)) {
        throw 'High-risk conversation and terminal tests require -ApprovalTeamId and an independent -ApproverToken.'
    }
    $approval = Invoke-CloudApi -Method POST -Path "/api/cloud/approvals/$ApprovalId/decision" `
        -Token $ApproverToken -Body @{ decision = 'approved'; comment = 'Terminal E2E independent approval' }
    $linkedId = [string]$approval.PSObject.Properties[$ResourceField].Value
    if ($approval.status -ne 'approved' -or $linkedId -ne $ResourceId) {
        throw "Approval $ApprovalId did not approve $ResourceField $ResourceId."
    }
}

function Remove-TestAccount {
    $envFile = Join-Path $root '.env'
    foreach ($line in Get-Content -LiteralPath $envFile) {
        if (-not $line -or $line.StartsWith('#')) { continue }
        $parts = $line -split '=', 2
        if ($parts.Count -eq 2) {
            [Environment]::SetEnvironmentVariable(
                $parts[0], $parts[1], [EnvironmentVariableTarget]::Process
            )
        }
    }
    $safeEmail = $email.Replace("'", "''")
    $cleanupSql = @"
BEGIN;
CREATE TEMP TABLE cleanup_terminal_session_ids ON COMMIT DROP AS
SELECT id FROM cloud_terminal_sessions
WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail');
DELETE FROM cloud_terminal_events WHERE session_id IN (
    SELECT id FROM cleanup_terminal_session_ids
);
DELETE FROM cloud_terminal_commands WHERE session_id IN (
    SELECT id FROM cleanup_terminal_session_ids
);
DELETE FROM cloud_terminal_sessions
WHERE id IN (SELECT id FROM cleanup_terminal_session_ids);
DELETE FROM cloud_conversation_messages WHERE conversation_id IN (
    SELECT id FROM cloud_conversations
    WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail')
);
DELETE FROM cloud_conversations
WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail');
CREATE TEMP TABLE cleanup_agent_task_ids ON COMMIT DROP AS
SELECT id FROM cloud_agent_tasks
WHERE user_id IN (SELECT id FROM cloud_users WHERE email = '$safeEmail');
DELETE FROM cloud_agent_task_events WHERE task_id IN (
    SELECT id FROM cleanup_agent_task_ids
);
DELETE FROM cloud_agent_task_checkpoints WHERE task_id IN (
    SELECT id FROM cleanup_agent_task_ids
);
DO `$cleanup$`
DECLARE
    deleted_count INTEGER;
BEGIN
    LOOP
        DELETE FROM cloud_agent_tasks AS task
        WHERE task.id IN (SELECT id FROM cleanup_agent_task_ids)
          AND NOT EXISTS (
              SELECT 1
              FROM cloud_agent_tasks AS child
              WHERE child.id <> task.id
                AND (
                    child.source_task_id = task.id
                    OR child.lineage_id = task.id
                    OR child.retry_of_task_id = task.id
                    OR child.rollback_source_task_id = task.id
                )
          );
        GET DIAGNOSTICS deleted_count = ROW_COUNT;
        EXIT WHEN deleted_count = 0;
    END LOOP;
END
`$cleanup$`;
DELETE FROM cloud_approvals AS approval
WHERE approval.requested_by IN (SELECT id FROM cloud_users WHERE email = '$safeEmail')
  AND (
      approval.agent_task_id IN (SELECT id FROM cleanup_agent_task_ids)
      OR approval.terminal_session_id IN (SELECT id FROM cleanup_terminal_session_ids)
  )
  AND NOT EXISTS (
      SELECT 1 FROM cloud_agent_tasks task WHERE task.id = approval.agent_task_id
  )
  AND NOT EXISTS (
      SELECT 1 FROM cloud_terminal_sessions session WHERE session.id = approval.terminal_session_id
  );
DELETE FROM cloud_users WHERE email = '$safeEmail';
COMMIT;
"@
    & $PostgresExecutable $env:DATABASE_URL -v ON_ERROR_STOP=1 -q -c $cleanupSql | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Unable to remove the disposable Cloud test account.' }
}

New-Item -ItemType Directory -Path $workspace -Force | Out-Null
try {
    $script:session = Invoke-CloudApi -Method POST -Path '/api/cloud/auth/register' -Body @{
        email = $email
        password = $password
        nickname = 'Terminal E2E'
        device_name = 'Terminal E2E'
        platform = $AgentPlatform
    }
    $pairing = Invoke-CloudApi -Method POST -Path '/api/cloud/agent-pairings' -Token $script:session.access_token
    if ($AgentPlatform -eq 'windows') {
        & $AgentExecutable pair --cloud $CloudOrigin --code $pairing.pairing_code `
            --name 'terminal-e2e' --workspace 'terminal-e2e' --workspace-root $workspace `
            --permissions full --capabilities 'heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1' `
            --config $config | Out-Null
    }
    else {
        & wsl.exe -d $WslDistribution -- $LinuxAgentExecutable pair --cloud $CloudOrigin `
            --code $pairing.pairing_code --name 'terminal-e2e' --workspace 'terminal-e2e' `
            --workspace-root $linuxWorkspace --permissions full `
            --capabilities 'heartbeat,tasks-v1,task-checkpoints-v1,shell-v1,terminal-v1' --config $linuxConfig | Out-Null
    }

    $agents = @(Invoke-CloudApi -Method GET -Path '/api/cloud/agents' -Token $script:session.access_token)
    $claimed = $agents | Where-Object { $_.status -eq 'claimed' } | Select-Object -First 1
    if (-not $claimed) { throw 'Claimed Agent was not listed.' }
    Invoke-CloudApi -Method POST -Path "/api/cloud/agents/$($claimed.id)/confirm" -Token $script:session.access_token | Out-Null
    if ($AgentPlatform -eq 'windows') {
        $agentProcess = Start-Process -FilePath $AgentExecutable `
            -ArgumentList @('run', '--config', $config) -RedirectStandardOutput $stdout `
            -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
    }
    else {
        $agentProcess = Start-Process -FilePath 'wsl.exe' `
            -ArgumentList @('-d', $WslDistribution, '--', $LinuxAgentExecutable, 'run', '--config', $linuxConfig) `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
    }
    $active = Wait-AgentOnline -AgentId $claimed.id
    if ([string]::IsNullOrWhiteSpace($ApprovalTeamId) -or [string]::IsNullOrWhiteSpace($ApproverToken)) {
        throw 'High-risk conversation and terminal tests require -ApprovalTeamId and an independent -ApproverToken.'
    }
    $idleWorkingSet = $null
    $idlePrivateBytes = $null
    if ($AgentPlatform -eq 'windows') {
        Start-Sleep -Seconds 1
        $agentProcess.Refresh()
        $idleWorkingSet = $agentProcess.WorkingSet64
        $idlePrivateBytes = $agentProcess.PrivateMemorySize64
    }

    $conversationResponse = Invoke-CloudApi -Method POST -Path '/api/cloud/conversations' `
        -Token $script:session.access_token -Body @{ title = 'E2E plan'; agent_id = $claimed.id }
    $conversation = $conversationResponse
    if (-not $conversation.id) { throw 'Conversation creation did not return its detail.' }
    $planCommand = if ($AgentPlatform -eq 'windows') { "Write-Output 'conversation-task-ok'" } else { "printf 'conversation-task-ok\n'" }
    $plan = Invoke-CloudApi -Method POST -Path "/api/cloud/conversations/$($conversation.id)/plans" `
        -Token $script:session.access_token -Body @{
            content = '执行一条经过批准的测试命令'
            agent_id = $claimed.id
            team_id = $ApprovalTeamId
            operation = 'shell.exec'
            input = @{ command = $planCommand; timeout_seconds = 30 }
            idempotency_key = "conversation-$stamp"
        }
    $planMessage = @($plan.messages) | Where-Object { $_.kind -eq 'plan' } | Select-Object -Last 1
    if (-not $planMessage.linked_task_id) { throw 'Plan message was not directly linked to a task.' }
    $linkedTask = Invoke-CloudApi -Method GET -Path "/api/cloud/agent-tasks/$($planMessage.linked_task_id)" -Token $script:session.access_token
    if ($linkedTask.status -ne 'awaiting_approval') { throw 'Conversation task did not wait for approval.' }
    Approve-LinkedResource -ApprovalId $linkedTask.approval_id -ResourceId $linkedTask.id -ResourceField 'agent_task_id'
    $linkedTask = Wait-AgentTask -TaskId $linkedTask.id -Statuses @('succeeded')
    if ($linkedTask.output.stdout -notmatch 'conversation-task-ok') { throw 'Conversation task output is incomplete.' }
    if (-not $linkedTask.can_resume) { throw 'Conversation task did not publish a resumable checkpoint.' }
    $conversationRetry = Invoke-CloudApi -Method POST -Path "/api/cloud/agent-tasks/$($linkedTask.id)/retry" `
        -Token $script:session.access_token -Body @{
            mode = 'resume'
            idempotency_key = "conversation-resume-$stamp"
        }
    if ($conversationRetry.status -ne 'awaiting_approval') { throw 'Conversation recovery did not require approval.' }
    Approve-LinkedResource -ApprovalId $conversationRetry.approval_id -ResourceId $conversationRetry.id -ResourceField 'agent_task_id'
    $conversationRetry = Wait-AgentTask -TaskId $conversationRetry.id -Statuses @('succeeded')
    $conversationDetail = Invoke-CloudApi -Method GET -Path "/api/cloud/conversations/$($conversation.id)" -Token $script:session.access_token
    $retryMessage = @($conversationDetail.messages) | Where-Object { $_.linked_task_id -eq $conversationRetry.id } | Select-Object -First 1
    if (-not $retryMessage) { throw 'Conversation retry was not directly bound to a new plan message.' }

    $terminal = Invoke-CloudApi -Method POST -Path '/api/cloud/terminal-sessions' `
        -Token $script:session.access_token -Body @{
            agent_id = $claimed.id
            team_id = $ApprovalTeamId
            title = 'E2E terminal'
            cwd = if ($AgentPlatform -eq 'windows') { $workspace } else { $linuxWorkspace }
            cols = 80
            rows = 24
    }
    if ($terminal.status -ne 'awaiting_approval') { throw 'Terminal did not wait for approval.' }
    Approve-LinkedResource -ApprovalId $terminal.approval_id -ResourceId $terminal.id -ResourceField 'terminal_session_id'
    $running = Wait-TerminalSession -SessionId $terminal.id -Statuses @('running')

    $terminalInput = if ($AgentPlatform -eq 'windows') {
        "$([char]27)[1;1Recho once>>terminal-input.txt`rtype terminal-input.txt`recho SCULK_TERMINAL_READY`r"
    } else {
        "printf 'once\n' >> terminal-input.txt`ncat terminal-input.txt`nprintf 'SCULK_TERMINAL_READY\n'`n"
    }
    $inputBody = @{
        data_base64 = ConvertTo-TerminalBase64 -Text $terminalInput
        idempotency_key = "terminal-input-$stamp"
    }
    Invoke-CloudApi -Method POST -Path "/api/cloud/terminal-sessions/$($terminal.id)/input" -Token $script:session.access_token -Body $inputBody | Out-Null
    Invoke-CloudApi -Method POST -Path "/api/cloud/terminal-sessions/$($terminal.id)/input" -Token $script:session.access_token -Body $inputBody | Out-Null
    Invoke-CloudApi -Method POST -Path "/api/cloud/terminal-sessions/$($terminal.id)/resize" -Token $script:session.access_token -Body @{ cols = 100; rows = 32 } | Out-Null
    $running = Wait-TerminalSession -SessionId $terminal.id -Statuses @('running') -OutputPattern 'SCULK_TERMINAL_READY'
    Start-Sleep -Seconds 2
    $replayed = Wait-TerminalSession -SessionId $terminal.id -Statuses @('running') -OutputPattern 'SCULK_TERMINAL_READY'
    if ($replayed.Session.cols -ne 100 -or $replayed.Session.rows -ne 32) { throw 'Terminal resize was not persisted.' }
    $inputFile = Join-Path $workspace 'terminal-input.txt'
    if (-not (Test-Path -LiteralPath $inputFile)) {
        $visibleOutput = $replayed.Output.Replace([string][char]27, '<ESC>').Replace("`r", '<CR>').Replace("`n", '<LF>')
        throw "Terminal command did not create its output file. Output: $visibleOutput"
    }
    $lines = @(Get-Content -LiteralPath $inputFile)
    if ($lines.Count -ne 1 -or $lines[0] -ne 'once') { throw 'Terminal input idempotency did not prevent duplicate input.' }

    $exitInput = if ($AgentPlatform -eq 'windows') { "exit`r" } else { "exit`n" }
    Invoke-CloudApi -Method POST -Path "/api/cloud/terminal-sessions/$($terminal.id)/input" `
        -Token $script:session.access_token -Body @{
            data_base64 = ConvertTo-TerminalBase64 -Text $exitInput
            idempotency_key = "terminal-exit-$stamp"
        } | Out-Null
    $exited = Wait-TerminalSession -SessionId $terminal.id -Statuses @('exited')
    $kinds = @($exited.Events | ForEach-Object { $_.kind })
    if (-not ($kinds -contains 'started') -or -not ($kinds -contains 'output') -or -not ($kinds -contains 'exit')) {
        throw 'Terminal event history is incomplete.'
    }

    $cancelled = Invoke-CloudApi -Method POST -Path '/api/cloud/terminal-sessions' `
        -Token $script:session.access_token -Body @{
            agent_id = $claimed.id; team_id = $ApprovalTeamId; title = 'Cancel before approval'; cols = 80; rows = 24
        }
    $cancelled = Invoke-CloudApi -Method POST -Path "/api/cloud/terminal-sessions/$($cancelled.id)/terminate" -Token $script:session.access_token
    if ($cancelled.status -ne 'cancelled') { throw 'Unapproved terminal was not cancelled.' }

    [pscustomobject]@{
        AgentPlatform = $AgentPlatform
        AgentOnline = $active.online
        ConversationLinkedTask = $linkedTask.status
        ConversationCheckpointResume = $conversationRetry.status
        ConversationRetryBound = [bool]$retryMessage
        TerminalStatus = $exited.Session.status
        TerminalEvents = $exited.Events.Count
        ReplayRecovered = $replayed.Output -match 'SCULK_TERMINAL_READY'
        Resize = "$($replayed.Session.cols)x$($replayed.Session.rows)"
        InputIdempotent = $lines.Count -eq 1
        PreApprovalCancel = $cancelled.status
        IdleWorkingSetBytes = $idleWorkingSet
        IdlePrivateBytes = $idlePrivateBytes
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
        [IO.Path]::GetFileName($resolvedTestRoot).StartsWith('terminal-e2e-') -and
        [IO.Directory]::Exists($resolvedTestRoot)) {
        [IO.Directory]::Delete($resolvedTestRoot, $true)
    }
    elseif (-not $testPassed) {
        Write-Error "Terminal E2E logs were retained at $resolvedTestRoot" -ErrorAction Continue
    }
}
