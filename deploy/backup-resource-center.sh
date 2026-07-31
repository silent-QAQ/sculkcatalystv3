#!/usr/bin/env bash
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

set -euo pipefail

readonly resource_root="/opt/sculk-resource"
readonly backup_root="${resource_root}/backups"

install -d -m 700 "${backup_root}"
if [[ "$(readlink -f "${backup_root}")" != "/opt/sculk-resource/backups" ]]; then
  echo "Refusing to use an unexpected backup path." >&2
  exit 1
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
tar -C "${resource_root}" -czf "${backup_root}/resource-${stamp}.tgz" data objects
find "${backup_root}" -maxdepth 1 -type f -name 'resource-*.tgz' -mtime +14 -delete
