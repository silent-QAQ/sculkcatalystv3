#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$script_dir/.." && pwd)"
backend="${SCULK_BACKEND_BIN:-$root/backend/target-local/release/backend}"
pid_file="$root/.runtime/backend.pid"

if [[ ! -f "$pid_file" ]]; then
  exit 0
fi
backend_pid="$(<"$pid_file")"
if [[ ! "$backend_pid" =~ ^[0-9]+$ ]]; then
  printf 'Invalid backend PID file: %s\n' "$pid_file" >&2
  exit 1
fi
if ! kill -0 "$backend_pid" 2>/dev/null; then
  rm -f -- "$pid_file"
  exit 0
fi

actual_executable="$(readlink -f "/proc/$backend_pid/exe" 2>/dev/null || true)"
expected_executable="$(readlink -f "$backend" 2>/dev/null || true)"
if [[ -z "$expected_executable" || "$actual_executable" != "$expected_executable" ]]; then
  printf 'PID %s is not the configured Sculk backend.\n' "$backend_pid" >&2
  exit 1
fi

kill -TERM "$backend_pid"
for _ in {1..100}; do
  if ! kill -0 "$backend_pid" 2>/dev/null; then
    rm -f -- "$pid_file"
    printf 'Sculk Catalyst stopped cleanly.\n'
    exit 0
  fi
  sleep 0.5
done

printf 'Graceful shutdown timed out; forcing PID %s to exit.\n' "$backend_pid" >&2
kill -KILL "$backend_pid" 2>/dev/null || true
rm -f -- "$pid_file"
