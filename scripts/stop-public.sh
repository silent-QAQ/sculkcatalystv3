#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(cd -- "$script_dir/.." && pwd -P)"
runtime_dir="$root/.runtime"
proxy_pid_file="$runtime_dir/public-proxy.pid"
proxy_command_file="$runtime_dir/public-proxy.command"
public_backend_pid_file="$runtime_dir/public-backend.pid"
backend_pid_file="$runtime_dir/backend.pid"
backend="$root/backend/target-local/release/backend"

process_is_running() {
  local pid="$1"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  [[ -d "/proc/$pid" ]] || return 1
  [[ "$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || true)" != Z ]]
}

if [[ -f "$proxy_pid_file" ]]; then
  proxy_pid="$(<"$proxy_pid_file")"
  if [[ "$proxy_pid" =~ ^[0-9]+$ ]] && process_is_running "$proxy_pid"; then
    [[ -f "$proxy_command_file" ]] || {
      printf 'Caddy command metadata is missing: %s\n' "$proxy_command_file" >&2
      exit 1
    }
    expected_caddy_command="$(<"$proxy_command_file")"
    actual_executable="$(readlink -f "/proc/$proxy_pid/exe" 2>/dev/null || true)"
    [[ "$actual_executable" == "$expected_caddy_command" ]] || {
      printf 'Public proxy PID file points to a different process: %s\n' "$proxy_pid" >&2
      exit 1
    }
    kill -TERM "$proxy_pid" 2>/dev/null || true
    for _ in {1..20}; do
      process_is_running "$proxy_pid" || break
      sleep 0.5
    done
    if process_is_running "$proxy_pid"; then
      kill -KILL "$proxy_pid" 2>/dev/null || true
    fi
  fi
  rm -f -- "$proxy_pid_file" "$proxy_command_file"
fi

# Stop only a backend that start-public.sh explicitly marked as its own. A
# regular local backend is left alone so Stop-Public cannot kill an unrelated
# local session.
stop_managed_backend=false
if [[ -f "$public_backend_pid_file" && -f "$backend_pid_file" ]]; then
  public_backend_pid="$(<"$public_backend_pid_file")"
  backend_pid="$(<"$backend_pid_file")"
  if [[ "$public_backend_pid" =~ ^[0-9]+$ && "$backend_pid" == "$public_backend_pid" ]] && process_is_running "$backend_pid"; then
    actual_executable="$(readlink -f "/proc/$backend_pid/exe" 2>/dev/null || true)"
    [[ "$actual_executable" == "$backend" ]] && stop_managed_backend=true
  fi
fi
if [[ "$stop_managed_backend" == true ]]; then
  "$script_dir/stop-local.sh"
fi
rm -f -- "$public_backend_pid_file"
