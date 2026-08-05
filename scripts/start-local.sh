#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/start-local.sh [port] [--port <port>]
       [--enable-codex-full-access --codex-command <absolute-path>]

The full-access option is explicit and applies only to the newly started
loopback backend. The command must be the same native Codex CLI path configured
for the Codex CLI agent.
EOF
}

port="${SCULK_PORT:-8787}"
enable_codex_full_access=false
codex_command=''
positional_port_seen=false
while (($#)); do
  case "$1" in
    --port)
      (($# >= 2)) || { printf '%s\n' '--port requires a value.' >&2; exit 1; }
      port="$2"
      shift 2
      ;;
    --enable-codex-full-access)
      enable_codex_full_access=true
      shift
      ;;
    --codex-command)
      (($# >= 2)) || { printf '%s\n' '--codex-command requires a path.' >&2; exit 1; }
      codex_command="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    [0-9]*)
      if [[ "$positional_port_seen" == true ]]; then
        printf '%s\n' 'Only one positional port is allowed.' >&2
        exit 1
      fi
      port="$1"
      positional_port_seen=true
      shift
      ;;
    *)
      printf 'Unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 1
      ;;
  esac
done
if [[ ! "$port" =~ ^[0-9]+$ ]] || ((port < 1 || port > 65535)); then
  printf 'Invalid port: %s\n' "$port" >&2
  exit 1
fi
if [[ "$enable_codex_full_access" == true ]]; then
  if [[ -z "$codex_command" || "$codex_command" != /* ]]; then
    printf '%s\n' '--enable-codex-full-access requires an absolute --codex-command path.' >&2
    exit 1
  fi
  if [[ ! -f "$codex_command" || ! -x "$codex_command" ]]; then
    printf 'Codex CLI is missing or not executable: %s\n' "$codex_command" >&2
    exit 1
  fi
  codex_command="$(readlink -f -- "$codex_command")"
elif [[ -n "$codex_command" ]]; then
  printf '%s\n' '--codex-command can only be used with --enable-codex-full-access.' >&2
  exit 1
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$script_dir/.." && pwd)"
runtime_dir="$root/.runtime"

resolve_local_path() {
  local candidate="$1"
  if [[ "$candidate" = /* ]]; then
    printf '%s' "$candidate"
  else
    printf '%s/%s' "$root" "$candidate"
  fi
}

backend="$(resolve_local_path "${SCULK_BACKEND_BIN:-backend/target-local/release/backend}")"
static_dir="$(resolve_local_path "${SCULK_STATIC_DIR:-frontend/dist}")"
data_dir="$(resolve_local_path "${SCULK_DATA_DIR:-backend/data}")"
pid_file="$runtime_dir/backend.pid"
public_backend_pid_file="$runtime_dir/public-backend.pid"

process_is_running() {
  local pid="$1"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  [[ -d "/proc/$pid" ]] || return 1
  [[ "$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || true)" != Z ]]
}

port_is_listening() {
  local port_hex
  local -a proc_files=(/proc/net/tcp)
  printf -v port_hex '%04X' "$1"
  [[ -r /proc/net/tcp6 ]] && proc_files+=(/proc/net/tcp6)
  awk -v port="$port_hex" '
    NR > 1 {
      split($2, endpoint, ":")
      if (toupper(endpoint[2]) == port && $4 == "0A") {
        found = 1
        exit
      }
    }
    END { exit(found ? 0 : 1) }
  ' "${proc_files[@]}"
}

stop_failed_backend() {
  local pid="$1"
  if process_is_running "$pid"; then
    kill -TERM "$pid" 2>/dev/null || true
    for _ in {1..20}; do
      process_is_running "$pid" || break
      sleep 0.1
    done
    if process_is_running "$pid"; then
      kill -KILL "$pid" 2>/dev/null || true
    fi
  fi
  rm -f -- "$pid_file"
}

if [[ ! -x "$backend" ]]; then
  printf 'Linux release backend is missing or not executable: %s\n' "$backend" >&2
  exit 1
fi
if [[ ! -f "$static_dir/index.html" ]]; then
  printf 'Frontend production bundle is missing: %s\n' "$static_dir/index.html" >&2
  exit 1
fi

mkdir -p -- "$runtime_dir" "$data_dir"
if [[ "${SCULK_PUBLIC_PROXY_MANAGED:-}" != true ]]; then
  rm -f -- "$public_backend_pid_file"
fi
command -v flock >/dev/null 2>&1 || { printf '%s\n' 'flock is required to start the local backend safely.' >&2; exit 1; }
exec 9>"$runtime_dir/backend.start.lock"
if ! flock -n 9; then
  printf '%s\n' 'Another local startup is already running; wait for it to finish and retry.' >&2
  exit 1
fi

if [[ -f "$pid_file" ]]; then
  existing_pid="$(<"$pid_file")"
  if process_is_running "$existing_pid"; then
    actual_executable="$(readlink -f "/proc/$existing_pid/exe" 2>/dev/null || true)"
    expected_executable="$(readlink -f "$backend")"
    if [[ "$actual_executable" == "$expected_executable" ]]; then
      if [[ "$enable_codex_full_access" == true ]]; then
        printf '%s\n' 'Codex full access only applies when starting a new backend; stop the existing backend first.' >&2
        exit 1
      fi
      printf 'Sculk Catalyst is already running (PID %s).\n' "$existing_pid"
      exit 0
    fi
    printf 'PID file points to a different process: %s\n' "$existing_pid" >&2
    exit 1
  fi
  rm -f -- "$pid_file"
fi

if port_is_listening "$port"; then
  printf 'Port %s is already in use by an untracked process.\n' "$port" >&2
  exit 1
fi

export SCULK_BIND_ADDRESS="127.0.0.1:$port"
export SCULK_STATIC_DIR="$static_dir"
export SCULK_DATA_DIR="$data_dir"
export SCULK_STATE_FILE="$data_dir/state.json"
# A local deployment must not accidentally attach to a Cloud database inherited
# from the shell or a parent service. Cloud has its own start/deploy entrypoints.
unset DATABASE_URL REDIS_URL SCULK_MASTER_KEY SCULK_ALLOWED_ORIGINS SCULK_CLOUD_PUBLIC_URL
unset SCULK_POSTGRES_PASSWORD SCULK_REDIS_PASSWORD
for cloud_var in $(compgen -v | grep -E '^SCULK_CLOUD_' || true); do
  unset "$cloud_var"
done
export SCULK_DISABLE_CLOUD=true
unset SCULK_ALLOW_CODEX_FULL SCULK_CODEX_TRUSTED_COMMAND
unset SCULK_PUBLIC_PROXY_MANAGED
if [[ "$enable_codex_full_access" == true ]]; then
  export SCULK_ALLOW_CODEX_FULL=true
  export SCULK_CODEX_TRUSTED_COMMAND="$codex_command"
fi

(
  cd -- "$root/backend"
  nohup "$backend" >"$runtime_dir/backend.log" 2>"$runtime_dir/backend.err.log" </dev/null &
  printf '%s\n' "$!" >"$pid_file"
)
backend_pid="$(<"$pid_file")"

ready=false
for _ in {1..50}; do
  if ! process_is_running "$backend_pid"; then
    break
  fi
  if curl --fail --silent --show-error --max-time 2 "http://127.0.0.1:$port/api/health" >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 0.2
done

if [[ "$ready" != true ]]; then
  stop_failed_backend "$backend_pid"
  printf 'Sculk Catalyst did not become ready; inspect %s\n' "$runtime_dir/backend.err.log" >&2
  exit 1
fi

printf 'Sculk Catalyst: http://127.0.0.1:%s (PID %s)\n' "$port" "$backend_pid"
