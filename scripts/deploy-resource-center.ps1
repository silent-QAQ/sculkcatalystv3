[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Za-z0-9._:-]+$')]
    [string]$RemoteHost,

    [ValidatePattern('^[A-Za-z_][A-Za-z0-9_-]*$')]
    [string]$RemoteUser = 'root',

    [switch]$InstallCaddyConfig,
    [switch]$SkipFrontendBuild
)

$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$releaseId = Get-Date -Format 'yyyyMMdd-HHmmss'
$archive = Join-Path $workspace ".runtime\sculk-resource-$releaseId.tar.gz"
$remoteArchive = "/tmp/sculk-resource-$releaseId.tar.gz"
$target = "$RemoteUser@$RemoteHost"

if (-not $SkipFrontendBuild) {
    Push-Location (Join-Path $workspace 'frontend')
    try { pnpm run build } finally { Pop-Location }
}

New-Item -ItemType Directory -Force (Split-Path -Parent $archive) | Out-Null
Push-Location $workspace
try {
    tar -czf $archive `
        backend/Cargo.toml backend/Cargo.lock backend/src backend/migrations `
        backend/resources/skills/develop-minecraft-server-plugin/SKILL.md `
        backend/resources/skills/develop-minecraft-server-plugin/references `
        frontend/dist `
        deploy/sculk-resource.service deploy/Caddyfile.resources `
        deploy/caddy-sculk-resource.conf
    if ($LASTEXITCODE -ne 0) { throw 'Failed to create deployment archive.' }

    scp $archive "${target}:$remoteArchive"
    if ($LASTEXITCODE -ne 0) { throw 'Failed to upload deployment archive.' }

    $installCaddy = if ($InstallCaddyConfig) { '1' } else { '0' }
    $remoteScript = @'
set -euo pipefail
resource_root=/opt/sculk-resource
release_id=__RELEASE_ID__
release_dir=${resource_root}/releases/${release_id}
archive=__REMOTE_ARCHIVE__
previous_target=
caddy_backup=
switched=0

rollback() {
  status=$?
  trap - EXIT
  if [ "${status}" -ne 0 ]; then
    echo "Deployment failed; restoring the previous release." >&2
    if [ "${switched}" = 1 ] && [ -n "${previous_target}" ] && [ -d "${previous_target}" ]; then
      ln -sfn "${previous_target}" "${resource_root}/current"
    fi
    if [ -n "${caddy_backup}" ] && [ -f "${caddy_backup}" ]; then
      install -m 0644 "${caddy_backup}" /etc/caddy/Caddyfile
    fi
    if [ "${switched}" = 1 ]; then
      systemctl daemon-reload || true
      systemctl restart sculk-resource || true
      systemctl restart caddy || true
    fi
  fi
  exit "${status}"
}
trap rollback EXIT

wait_for_url() {
  url=$1
  attempts=30
  while [ "${attempts}" -gt 0 ]; do
    if curl --fail --silent --output /dev/null "${url}"; then return 0; fi
    attempts=$((attempts - 1))
    sleep 1
  done
  curl --fail --silent --show-error --output /dev/null "${url}"
}

test "${resource_root}" = /opt/sculk-resource
mkdir -p "${release_dir}" "${resource_root}/objects" "${resource_root}/data"
tar -xzf "${archive}" -C "${release_dir}"
cd "${release_dir}/backend"
CARGO_TARGET_DIR="${resource_root}/build-target" cargo build --release --locked
mkdir -p "${release_dir}/backend/target/release"
install -m 0755 "${resource_root}/build-target/release/backend" "${release_dir}/backend/target/release/backend"
chown -R sculk-resource:sculk-resource "${release_dir}" "${resource_root}/objects" "${resource_root}/data"

if [ -L "${resource_root}/current" ]; then
  previous_target=$(readlink -f "${resource_root}/current")
elif [ -e "${resource_root}/current" ]; then
  mv "${resource_root}/current" "${resource_root}/releases/previous-${release_id}"
fi
ln -sfn "${release_dir}" "${resource_root}/current"
switched=1
install -m 0644 "${release_dir}/deploy/sculk-resource.service" /etc/systemd/system/sculk-resource.service

if [ "__INSTALL_CADDY__" = 1 ]; then
  set -a
  . "${resource_root}/config/resource.env"
  set +a
  caddy validate --config "${release_dir}/deploy/Caddyfile.resources" --adapter caddyfile
  if [ -f /etc/caddy/Caddyfile ]; then
    caddy_backup="/etc/caddy/Caddyfile.sculk-backup-${release_id}"
    cp -p /etc/caddy/Caddyfile "${caddy_backup}"
  fi
  install -m 0644 "${release_dir}/deploy/Caddyfile.resources" /etc/caddy/Caddyfile
  install -d -m 0755 /etc/systemd/system/caddy.service.d
  install -m 0644 "${release_dir}/deploy/caddy-sculk-resource.conf" /etc/systemd/system/caddy.service.d/sculk-resource.conf
fi

systemctl daemon-reload
systemctl restart sculk-resource
wait_for_url http://127.0.0.1:8789/api/health
if [ "__INSTALL_CADDY__" = 1 ]; then
  systemctl restart caddy
fi

wait_for_url http://127.0.0.1:8789/
rm -f "${archive}"
if [ -n "${caddy_backup}" ]; then rm -f "${caddy_backup}"; fi
trap - EXIT
'@
    $remoteScript = $remoteScript.Replace('__RELEASE_ID__', $releaseId).Replace('__REMOTE_ARCHIVE__', $remoteArchive).Replace('__INSTALL_CADDY__', $installCaddy)
    # Windows PowerShell writes here-strings with CRLF; Bash treats a trailing CR
    # in `trap - EXIT` as part of the signal name.
    $remoteScript = $remoteScript.Replace("`r", '')
    # The PowerShell pipeline itself may reintroduce CRLF while streaming. Strip
    # carriage returns on the remote side before Bash parses the script.
    # Windows PowerShell may also prefix native-pipeline stdin with a UTF-8 BOM.
    # Strip it before `set -euo pipefail`, otherwise Bash treats `set` as an
    # unknown BOM-prefixed command and silently loses the rollback safety mode.
    $remoteScript | ssh $target 'sed ''1s/^\xEF\xBB\xBF//'' | tr -d ''\015'' | bash'
    if ($LASTEXITCODE -ne 0) { throw 'Remote build or service switch failed.' }
} finally {
    Pop-Location
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
}

Write-Host "Resource center release $releaseId deployed to $target" -ForegroundColor Green
