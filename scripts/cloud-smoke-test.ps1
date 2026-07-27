param(
    [string]$BaseUrl = 'http://127.0.0.1:8788',
    [string]$RelayBaseUrl = 'http://127.0.0.1:9944'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-Cloud {
    param(
        [string]$Method,
        [string]$Path,
        [object]$Body,
        [string]$Token
    )
    $arguments = @{
        Method = $Method
        Uri = "$BaseUrl$Path"
        ContentType = 'application/json'
    }
    if ($Token) {
        $arguments.Headers = @{ Authorization = "Bearer $Token" }
    }
    if ($null -ne $Body) {
        $arguments.Body = $Body | ConvertTo-Json -Depth 20
    }
    Invoke-RestMethod @arguments
}

function Assert-Cloud {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw "Assertion failed: $Message" }
}

$suffix = [Guid]::NewGuid().ToString('N').Substring(0, 10)
$password = "Cloud-$suffix-test"
$adminEmail = "admin-$suffix@example.com"
$memberEmail = "member-$suffix@example.com"

$status = Invoke-Cloud GET '/api/cloud/status' $null ''
Assert-Cloud $status.available 'cloud service should be available'

$adminAuth = Invoke-Cloud POST '/api/cloud/auth/register' @{
    email = $adminEmail
    password = $password
    nickname = 'Integration Admin'
    device_name = 'Primary Test Device'
    platform = 'Windows'
} ''
$adminToken = $adminAuth.access_token
Assert-Cloud ($adminAuth.profile.role -eq 'admin') 'first account should become admin'

$memberAuth = Invoke-Cloud POST '/api/cloud/auth/register' @{
    email = $memberEmail
    password = $password
    nickname = 'Integration Member'
    device_name = 'Member Test Device'
    platform = 'Linux'
} ''
$memberToken = $memberAuth.access_token

$profile = Invoke-Cloud PATCH '/api/cloud/me' @{
    nickname = 'Sculk Test Admin'
    locale = 'zh-CN'
} $adminToken
Assert-Cloud ($profile.nickname -eq 'Sculk Test Admin') 'profile update should persist'

$initialSync = Invoke-Cloud GET '/api/cloud/sync/settings' $null $adminToken
$updatedSync = Invoke-Cloud PUT '/api/cloud/sync/settings' @{
    base_version = $initialSync.version
    payload = @{ ui = @{ language = 'zh-CN'; test_marker = $suffix } }
} $adminToken
Assert-Cloud ($updatedSync.version -eq ($initialSync.version + 1)) 'sync version should increment'
$syncConflict = $false
try {
    Invoke-Cloud PUT '/api/cloud/sync/settings' @{
        base_version = $initialSync.version
        payload = @{ ui = @{ language = 'en-US' } }
    } $adminToken | Out-Null
} catch {
    $syncConflict = $_.Exception.Response.StatusCode.value__ -eq 409
}
Assert-Cloud $syncConflict 'stale sync should return HTTP 409'

$secondLogin = Invoke-Cloud POST '/api/cloud/auth/login' @{
    email = $adminEmail
    password = $password
    device_name = 'Revoked Test Device'
    platform = 'Web'
} ''
$deviceResponse = Invoke-Cloud GET '/api/cloud/devices' $null $adminToken
$devices = @($deviceResponse | ForEach-Object { $_ })
$revokedDevice = $devices | Where-Object { $_.name -eq 'Revoked Test Device' } | Select-Object -First 1
Assert-Cloud ($null -ne $revokedDevice) 'second device should appear in device list'
Invoke-Cloud DELETE "/api/cloud/devices/$($revokedDevice.id)" $null $adminToken | Out-Null
$revokedSessionRejected = $false
try { Invoke-Cloud GET '/api/cloud/me' $null $secondLogin.access_token | Out-Null } catch {
    $revokedSessionRejected = $_.Exception.Response.StatusCode.value__ -eq 401
}
Assert-Cloud $revokedSessionRejected 'revoked device session should fail immediately'

$team = Invoke-Cloud POST '/api/cloud/teams' @{ name = "Sculk Test Team $suffix" } $adminToken
$invitation = Invoke-Cloud POST "/api/cloud/teams/$($team.id)/invitations" @{
    email = $memberEmail
    role = 'approver'
} $adminToken
$acceptedTeam = Invoke-Cloud POST '/api/cloud/invitations/accept' @{
    invite_code = $invitation.invite_code
} $memberToken
Assert-Cloud ($acceptedTeam.id -eq $team.id) 'member should join invited team'
$memberResponse = Invoke-Cloud GET "/api/cloud/teams/$($team.id)/members" $null $adminToken
$members = @($memberResponse | ForEach-Object { $_ })
Assert-Cloud ($members.Count -eq 2) 'team should contain two members'

$approval = Invoke-Cloud POST '/api/cloud/approvals' @{
    team_id = $team.id
    title = 'Restart integration test server'
    summary = 'Validate remote approval without changing a real server.'
    risk = 'high'
    payload = @{ server_id = 'integration-test'; action = 'restart' }
} $memberToken
$decidedApproval = Invoke-Cloud POST "/api/cloud/approvals/$($approval.id)/decision" @{
    decision = 'approved'
    comment = 'Integration test passed'
} $adminToken
Assert-Cloud ($decidedApproval.status -eq 'approved') 'admin should approve remote request'

$provider = Invoke-Cloud PUT '/api/cloud/admin/relay-provider' @{
    name = 'Local Integration Upstream'
    base_url = $RelayBaseUrl
    api_key = 'sk-local-integration-test'
    default_model = 'mock-gpt-mini'
    enabled = $true
} $adminToken
Assert-Cloud $provider.configured 'admin provider configuration should persist'

$createdApiToken = Invoke-Cloud POST '/api/cloud/tokens' @{
    label = 'Integration Test Token'
    expires_in_days = 30
} $adminToken
$relayResponse = Invoke-Cloud POST '/api/cloud/v1/chat/completions' @{
    model = 'mock-gpt-mini'
    messages = @(@{ role = 'user'; content = 'Sculk Cloud relay test' })
} $createdApiToken.token
Assert-Cloud ($relayResponse.usage.total_tokens -eq 31) 'relay should return token usage'
$usage = Invoke-Cloud GET '/api/cloud/usage?days=1' $null $adminToken
Assert-Cloud ($usage.requests -eq 1) 'usage should contain one relay request'
Assert-Cloud ($usage.total_tokens -eq 31) 'usage should record 31 tokens'

$capability = Invoke-Cloud GET '/api/cloud/deployments/capability' $null ''
Assert-Cloud ($capability.status -eq 'planned') 'deployment capability should be planned'
$deploymentReserved = $false
try { Invoke-Cloud POST '/api/cloud/deployments' $null $adminToken | Out-Null } catch {
    $deploymentReserved = $_.Exception.Response.StatusCode.value__ -eq 501
}
Assert-Cloud $deploymentReserved 'deployment create should return HTTP 501'

Invoke-Cloud POST '/api/cloud/auth/logout' $null $memberToken | Out-Null
$logoutRejected = $false
try { Invoke-Cloud GET '/api/cloud/me' $null $memberToken | Out-Null } catch {
    $logoutRejected = $_.Exception.Response.StatusCode.value__ -eq 401
}
Assert-Cloud $logoutRejected 'logged-out session should be rejected'

[pscustomobject]@{
    passed = $true
    admin_role = $adminAuth.profile.role
    profile_updated = $profile.nickname
    sync_version = $updatedSync.version
    sync_conflict_409 = $syncConflict
    device_revoked = $revokedSessionRejected
    team_members = $members.Count
    approval_status = $decidedApproval.status
    relay_reply = $relayResponse.choices[0].message.content
    relay_tokens = $usage.total_tokens
    deployment_reserved_501 = $deploymentReserved
    logout_rejected = $logoutRejected
    test_admin_email = $adminEmail
} | ConvertTo-Json -Depth 5
