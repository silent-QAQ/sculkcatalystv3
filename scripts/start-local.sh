#!/usr/bin/env bash
set -euo pipefail

port="${1:-${SCULK_PORT:-8787}}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$script_dir/.." && pwd)"
runtime_dir="$root/.runtime"
backend="${SCULK_BACKEND_BIN:-$root/backend/target-local/release/backend}"
static_dir="${SCULK_STATIC_DIR:-$root/frontend/dist}"
data_dir="${SCULK_DATA_DIR:-$root/backend/data}"
pid_file="$runtime_dir/backend.pid"

if [[ ! -x "$backend" ]]; then
  printf 'Linux release backend is missing or not executable: %s\n' "$backend" >&2
  exit 1
fi
if [[ ! -f "$static_dir/index.html" ]]; then
  printf 'Frontend production bundle is missing: %s\n' "$static_dir/index.html" >&2
  exit 1
fi

mkdir -p -- "$runtime_dir" "$data_dir"
if [[ -f "$pid_file" ]]; then
  existing_pid="$(<"$pid_file")"
  if [[ "$existing_pid" =~ ^[0-9]+$ ]] && kill -0 "$existing_pid" 2>/dev/null; then
    actual_executable="$(readlink -f "/proc/$existing_pid/exe" 2>/dev/null || true)"
    expected_executable="$(readlink -f "$backend")"
    if [[ "$actual_executable" == "$expected_executable" ]]; then
      printf 'Sculk Catalyst is already running (PID %s).\n' "$existing_pid"
      exit 0
    fi
    printf 'PID file points to a different process: %s\n' "$existing_pid" >&2
    exit 1
  fi
  rm -f -- "$pid_file"
fi

export SCULK_BIND_ADDRESS="127.0.0.1:$port"
export SCULK_STATIC_DIR="$static_dir"
export SCULK_DATA_DIR="$data_dir"
export SCULK_STATE_FILE="${SCULK_STATE_FILE:-$data_dir/state.json}"

(
  cd -- "$root/backend"
  nohup "$backend" >"$runtime_dir/backend.log" 2>"$runtime_dir/backend.err.log" </dev/null &
  printf '%s\n' "$!" >"$pid_file"
)
backend_pid="$(<"$pid_file")"

ready=false
for _ in {1..50}; do
  if curl --fail --silent --show-error --max-time 2 "http://127.0.0.1:$port/api/health" >/dev/null 2>&1; then
    ready=true
    break
  fi
  if ! kill -0 "$backend_pid" 2>/dev/null; then
    break
  fi
  sleep 0.2
done

if [[ "$ready" != true ]]; then
  kill -TERM "$backend_pid" 2>/dev/null || true
  sleep 1
  kill -KILL "$backend_pid" 2>/dev/null || true
  rm -f -- "$pid_file"
  printf 'Sculk Catalyst did not become ready; inspect %s\n' "$runtime_dir/backend.err.log" >&2
  exit 1
fi

printf 'Sculk Catalyst: http://127.0.0.1:%s (PID %s)\n' "$port" "$backend_pid"
