#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/start-public.sh --confirm-public-admin-console \
       [--port <port>] [--domain <hostname>] [--email <email>]
       [--username <username>] [--caddy-command <path>]
       [--reset-credentials]

Starts the local backend on 127.0.0.1 and exposes it only through an
authenticated Caddy HTTPS reverse proxy. Caddy must be installed separately.
EOF
}

die() {
  printf '%s\n' "$*" >&2
  exit 1
}

validate_domain() {
  local value="$1"
  [[ "$value" =~ ^([A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)+[A-Za-z]{2,63}$ ]] || \
    die 'A public HTTPS deployment requires a DNS hostname such as console.example.com.'
}

validate_email() {
  [[ "$1" =~ ^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$ ]] || \
    die 'A valid ACME contact email is required for automatic HTTPS certificates.'
}

validate_username() {
  [[ "$1" =~ ^[A-Za-z0-9_-]{1,64}$ ]] || \
    die 'Username must contain 1-64 ASCII letters, digits, underscores, or hyphens.'
}

random_password() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
  fi
}

process_is_running() {
  local pid="$1"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  [[ -d "/proc/$pid" ]] || return 1
  [[ "$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || true)" != Z ]]
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(cd -- "$script_dir/.." && pwd -P)"
runtime_dir="$root/.runtime"
public_dir="$root/backend/data/public-proxy"
caddyfile="$public_dir/Caddyfile"
proxy_pid_file="$runtime_dir/public-proxy.pid"
proxy_command_file="$runtime_dir/public-proxy.command"
backend_pid_file="$runtime_dir/backend.pid"
public_backend_pid_file="$runtime_dir/public-backend.pid"
backend="$root/backend/target-local/release/backend"
proxy_log="$runtime_dir/public-proxy.log"
proxy_error_log="$runtime_dir/public-proxy.err.log"

backend_port=8787
domain=''
email=''
username='sculk'
caddy_command="${SCULK_CADDY_COMMAND:-}"
reset_credentials=false
confirm_public_admin_console=false
while (($#)); do
  case "$1" in
    --port)
      (($# >= 2)) || die '--port requires a value.'
      backend_port="$2"
      shift 2
      ;;
    --domain)
      (($# >= 2)) || die '--domain requires a value.'
      domain="$2"
      shift 2
      ;;
    --email)
      (($# >= 2)) || die '--email requires a value.'
      email="$2"
      shift 2
      ;;
    --username)
      (($# >= 2)) || die '--username requires a value.'
      username="$2"
      shift 2
      ;;
    --caddy-command)
      (($# >= 2)) || die '--caddy-command requires an absolute path.'
      caddy_command="$2"
      shift 2
      ;;
    --confirm-public-admin-console)
      confirm_public_admin_console=true
      shift
      ;;
    --reset-credentials)
      reset_credentials=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "Unknown argument: $1"
      ;;
  esac
done

[[ "$confirm_public_admin_console" == true ]] || \
  die 'Pass --confirm-public-admin-console to acknowledge the remotely reachable administrative console.'
[[ "$backend_port" =~ ^[0-9]+$ ]] && (($backend_port >= 1 && $backend_port <= 65535)) || \
  die "Invalid port: $backend_port"

if [[ -z "$caddy_command" ]]; then
  caddy_command="$(command -v caddy || true)"
elif [[ "$caddy_command" != /* ]]; then
  die '--caddy-command must be an absolute path.'
fi
[[ -n "$caddy_command" && -x "$caddy_command" ]] || \
  die 'Caddy was not found. Install Caddy first or pass --caddy-command with its absolute executable path.'
caddy_command="$(readlink -f -- "$caddy_command")"

mkdir -p -- "$runtime_dir" "$public_dir"
chmod 700 -- "$public_dir"

backend_pid=''
if [[ -f "$backend_pid_file" ]]; then
  recorded_backend_pid="$(<"$backend_pid_file")"
  if [[ "$recorded_backend_pid" =~ ^[0-9]+$ ]] && process_is_running "$recorded_backend_pid"; then
    actual_executable="$(readlink -f "/proc/$recorded_backend_pid/exe" 2>/dev/null || true)"
    [[ "$actual_executable" == "$backend" ]] || die "Backend PID file points to a different process: $recorded_backend_pid"
    backend_pid="$recorded_backend_pid"
    if [[ -f "$public_backend_pid_file" ]]; then
      public_backend_pid="$(<"$public_backend_pid_file")"
    else
      public_backend_pid=''
    fi
    [[ "$public_backend_pid" == "$backend_pid" ]] || \
      die 'A local backend is already running outside public-proxy management. Stop it before starting the public HTTPS console.'
  else
    rm -f -- "$backend_pid_file" "$public_backend_pid_file"
  fi
fi

proxy_running=false
if [[ -f "$proxy_pid_file" ]]; then
  recorded_proxy_pid="$(<"$proxy_pid_file")"
  if [[ "$recorded_proxy_pid" =~ ^[0-9]+$ ]] && process_is_running "$recorded_proxy_pid"; then
    [[ -f "$proxy_command_file" ]] || die "Caddy command metadata is missing: $proxy_command_file"
    expected_caddy_command="$(<"$proxy_command_file")"
    [[ "$expected_caddy_command" == "$caddy_command" ]] || \
      die 'Caddy command changed while the public proxy is running. Stop it before using a different Caddy executable.'
    actual_executable="$(readlink -f "/proc/$recorded_proxy_pid/exe" 2>/dev/null || true)"
    [[ "$actual_executable" == "$expected_caddy_command" ]] || die "Caddy PID file points to a different process: $recorded_proxy_pid"
    proxy_running=true
  else
    rm -f -- "$proxy_pid_file" "$proxy_command_file"
  fi
fi
if [[ "$reset_credentials" == true && "$proxy_running" == true ]]; then
  die 'Stop the existing public proxy before resetting credentials.'
fi

created_credentials=false
administrator_password=''
if [[ "$reset_credentials" == true || ! -f "$caddyfile" ]]; then
  if [[ -z "$domain" ]]; then
    read -r -p 'Public DNS hostname (for example console.example.com): ' domain
  fi
  if [[ -z "$email" ]]; then
    read -r -p 'ACME contact email: ' email
  fi
  validate_domain "$domain"
  validate_email "$email"
  validate_username "$username"
  administrator_password="$(random_password)"
  password_hash="$("$caddy_command" hash-password --plaintext "$administrator_password")" || \
    die 'Caddy could not create the administrator password hash.'
  umask 077
  cat > "$caddyfile" <<EOF
# sculk-public-domain: $domain
# sculk-backend-port: $backend_port
{
    email $email
}

$domain {
    basic_auth {
        $username $password_hash
    }
    reverse_proxy 127.0.0.1:$backend_port
}
EOF
  chmod 600 -- "$caddyfile"
  created_credentials=true
else
  domain="$(sed -n 's/^# sculk-public-domain: //p' "$caddyfile" | head -n 1)"
  [[ -n "$domain" ]] || die "The managed Caddyfile does not contain a public DNS hostname: $caddyfile"
  validate_domain "$domain"
  configured_port="$(sed -n 's/^# sculk-backend-port: //p' "$caddyfile" | head -n 1)"
  if [[ -z "$configured_port" ]]; then
    configured_port="$(awk '
      /^[[:space:]]*reverse_proxy[[:space:]]+127\.0\.0\.1:[0-9]+[[:space:]]*$/ {
        value = $3
        sub(/^127\.0\.0\.1:/, "", value)
        print value
        exit
      }
    ' "$caddyfile")"
  fi
  [[ "$configured_port" == "$backend_port" ]] || \
    die 'The existing Caddyfile uses a different backend port. Stop the public proxy and reset credentials before changing it.'
  chmod 600 -- "$caddyfile"
fi

"$caddy_command" validate --config "$caddyfile" --adapter caddyfile >/dev/null || \
  die 'Caddyfile validation failed. Check the configured DNS hostname and Caddy installation.'

if [[ -z "$backend_pid" ]]; then
  SCULK_PUBLIC_PROXY_MANAGED=true "$script_dir/start-local.sh" --port "$backend_port"
  [[ -f "$backend_pid_file" ]] || die 'The loopback backend did not start.'
  backend_pid="$(<"$backend_pid_file")"
  [[ "$backend_pid" =~ ^[0-9]+$ ]] && process_is_running "$backend_pid" || die 'The loopback backend did not start.'
  actual_executable="$(readlink -f "/proc/$backend_pid/exe" 2>/dev/null || true)"
  [[ "$actual_executable" == "$backend" ]] || die "Backend PID file points to a different process: $backend_pid"
  printf '%s\n' "$backend_pid" > "$public_backend_pid_file"
fi

if [[ "$proxy_running" != true ]]; then
  nohup "$caddy_command" run --config "$caddyfile" --adapter caddyfile >"$proxy_log" 2>"$proxy_error_log" </dev/null &
  proxy_pid="$!"
  printf '%s\n' "$caddy_command" > "$proxy_command_file"
  printf '%s\n' "$proxy_pid" > "$proxy_pid_file"
  sleep 1
  if ! process_is_running "$proxy_pid"; then
    rm -f -- "$proxy_pid_file" "$proxy_command_file"
    die "Caddy exited during startup. Inspect $proxy_error_log"
  fi
fi

printf 'Public HTTPS console: https://%s\n' "$domain"
printf '%s\n' 'The backend remains private on 127.0.0.1; do not expose port 8787 in the firewall or router.'
if [[ "$created_credentials" == true ]]; then
  printf 'Administrator username: %s\nAdministrator password: %s\n' "$username" "$administrator_password"
  printf '%s\n' 'Save the password now. It is shown only once and is not stored in plaintext.'
fi
