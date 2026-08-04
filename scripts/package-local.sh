#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/package-local.sh [--version <version>] [--output-dir <path>] [--refresh-dependencies] [--skip-dependency-install]

Builds a native Linux local deployment archive below artifacts/generated/local.
All frontend dependencies and Rust build output are created in a temporary
workspace below the package output directory; source build directories are not
modified. The archive contains only the local frontend bundle and backend runtime.
When --version is omitted, the backend Cargo package version is used.
Dependencies are installed in the isolated workspace by default;
--skip-dependency-install copies the existing source dependencies read-only.
EOF
}

die() {
  printf '%s\n' "$*" >&2
  exit 1
}

read_backend_version() {
  local manifest="$1"
  awk '
    /^\[package\][[:space:]]*$/ { in_package=1; next }
    in_package && /^\[/ { exit }
    in_package && /^[[:space:]]*version[[:space:]]*=/ {
      value=$0
      sub(/^[^"]*"/, "", value)
      sub(/".*$/, "", value)
      print value
      exit
    }
  ' "$manifest"
}

validate_version() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  [[ -n "$value" ]] || die 'Version must be a non-empty filename-safe value.'
  [[ "$value" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]] || \
    die 'Version must contain only letters, digits, ., _, +, and -.'
  printf '%s' "$value"
}

if [[ "$(uname -s)" != "Linux" ]]; then
  die 'package-local.sh must run on Linux. Build the Windows archive with scripts/package-local.ps1 on Windows.'
fi

case "$(uname -m)" in
  x86_64|amd64) package_arch='x86_64' ;;
  aarch64|arm64) package_arch='aarch64' ;;
  *) die "Unsupported Linux architecture for local distribution: $(uname -m)" ;;
esac

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(cd -- "$script_dir/.." && pwd -P)"
artifacts_root="$root/artifacts"
backend_source="$root/backend"
frontend_source="$root/frontend"
source_node_modules="$frontend_source/node_modules"
backend_manifest="$backend_source/Cargo.toml"
version_arg=''
output_arg=''
skip_dependency_install=false
refresh_dependencies=false

while (($#)); do
  case "$1" in
    --version|-v)
      (($# >= 2)) || die '--version requires a value.'
      version_arg="$2"
      shift 2
      ;;
    --output-dir)
      (($# >= 2)) || die '--output-dir requires a path.'
      output_arg="$2"
      shift 2
      ;;
    --skip-dependency-install)
      skip_dependency_install=true
      shift
      ;;
    --refresh-dependencies)
      refresh_dependencies=true
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

if [[ "$refresh_dependencies" == true && "$skip_dependency_install" == true ]]; then
  die '--refresh-dependencies and --skip-dependency-install cannot be used together.'
fi

command -v cargo >/dev/null 2>&1 || die 'cargo is required.'
command -v npm >/dev/null 2>&1 || die 'npm is required.'
command -v tar >/dev/null 2>&1 || die 'tar is required.'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required.'
command -v flock >/dev/null 2>&1 || die 'flock is required.'
[[ -f "$backend_manifest" ]] || die "Backend manifest is missing: $backend_manifest"

if [[ -z "$version_arg" ]]; then
  version_arg="$(read_backend_version "$backend_manifest")"
fi
package_version="$(validate_version "$version_arg")"

mkdir -p -- "$artifacts_root"
artifacts_root="$(cd -- "$artifacts_root" && pwd -P)"
if [[ -z "$output_arg" ]]; then
  output_dir="$artifacts_root/generated/local"
elif [[ "$output_arg" = /* ]]; then
  output_dir="$output_arg"
else
  output_dir="$root/$output_arg"
fi
mkdir -p -- "$output_dir"
output_dir="$(cd -- "$output_dir" && pwd -P)"
case "$output_dir/" in
  "$artifacts_root/"*) ;;
  *) die "--output-dir must stay below $artifacts_root" ;;
esac
[[ "$output_dir" != "$artifacts_root" ]] || die '--output-dir must be a child directory of artifacts/.'

remove_managed_path() {
  local base="$1"
  local path="$2"
  [[ "$path" != "$base" ]] || die "Refusing to remove the managed base directory: $path"
  case "$path" in
    "$base"/*) ;;
    *) die "Refusing to remove a path outside its managed directory: $path" ;;
  esac
  if [[ -e "$path" || -L "$path" ]]; then
    rm -rf -- "$path"
  fi
}

remove_previous_archives() {
  local old_path old_name
  while IFS= read -r -d '' old_path; do
    old_name="${old_path##*/}"
    case "$old_name" in
      "${release_base}.tar.gz"|"${release_base}"-*.tar.gz|\
      "${release_base}.tar.gz.sha256"|"${release_base}"-*.tar.gz.sha256|\
      ".${release_base}.staging"|".${release_base}"-*.staging)
        remove_managed_path "$output_dir" "$old_path"
        ;;
    esac
  done < <(find "$output_dir" -mindepth 1 -maxdepth 1 \( -type f -o -type d -o -type l \) -print0)
}

release_base="sculk-catalyst-local-linux-$package_arch"
release_name="${release_base}-${package_version}"
staging_directory="$output_dir/.${release_name}.staging"
package_directory="$staging_directory/$release_name"
build_directory="$staging_directory/.build"
backend_target_directory="$build_directory/cargo-target"
frontend_build_source="$build_directory/frontend"
backend_binary="$backend_target_directory/release/backend"
static_directory="$frontend_build_source/dist-package-local"
archive="$output_dir/${release_name}.tar.gz"
checksum="$archive.sha256"
package_lock_directory="$artifacts_root/generated"
package_lock="$package_lock_directory/.sculk-catalyst-local-package.lock"
mkdir -p -- "$package_lock_directory"
exec 9>"$package_lock"
flock -n 9 || die 'Another local package build is already running for this workspace. Wait for it to finish and retry.'

succeeded=false
cleanup() {
  local status=$?
  trap - EXIT
  remove_managed_path "$output_dir" "$staging_directory"
  if [[ "$succeeded" != true ]]; then
    remove_managed_path "$output_dir" "$archive"
    remove_managed_path "$output_dir" "$checksum"
  fi
  flock -u 9 || true
  exec 9>&-
  exit "$status"
}
trap cleanup EXIT

# Remove stale package-owned output state before starting a new build.
remove_previous_archives
remove_managed_path "$output_dir" "$staging_directory"
remove_managed_path "$output_dir" "$archive"
remove_managed_path "$output_dir" "$checksum"

(
  cd -- "$backend_source"
  CARGO_TARGET_DIR="$backend_target_directory" cargo build --release --locked
)
mkdir -p -- "$frontend_build_source"
shopt -s dotglob nullglob
for frontend_entry in "$frontend_source"/*; do
  frontend_name="${frontend_entry##*/}"
  case "$frontend_name" in
    node_modules|dist|dist-cloud|dist-website|.env*) continue ;;
  esac
  cp -a -- "$frontend_entry" "$frontend_build_source/"
done
shopt -u dotglob nullglob
if [[ "$skip_dependency_install" == true ]]; then
  [[ -d "$source_node_modules" ]] || die "Frontend dependencies are missing: $source_node_modules"
  cp -a -- "$source_node_modules" "$frontend_build_source/"
else
  (
    cd -- "$frontend_build_source"
    npm ci
  )
fi
(
  cd -- "$frontend_build_source"
  VITE_APP_MODE=local npm run build -- --outDir dist-package-local
)

[[ -x "$backend_binary" ]] || die "Native release backend was not produced: $backend_binary"
[[ -f "$static_directory/index.html" ]] || die "Dedicated local frontend bundle was not produced: $static_directory/index.html"

package_backend="$package_directory/backend/target-local/release"
package_static="$package_directory/frontend/dist"
package_scripts="$package_directory/scripts"
mkdir -p -- "$package_backend" "$package_static" "$package_scripts" "$package_directory/backend/data"
install -m 0755 -- "$backend_binary" "$package_backend/backend"
cp -a -- "$static_directory"/. "$package_static/"
install -m 0755 -- "$root/scripts/start-local.sh" "$package_scripts/start-local.sh"
install -m 0755 -- "$root/scripts/stop-local.sh" "$package_scripts/stop-local.sh"
install -m 0644 -- "$root/LICENSE" "$package_directory/LICENSE"
install -m 0644 -- "$root/NOTICE" "$package_directory/NOTICE"
cp -a -- "$root/LICENSES" "$package_directory/LICENSES"

cat > "$package_directory/README.md" <<'EOF'
# Sculk Catalyst V3 本地部署

这是仅供本机使用的部署包，服务默认只监听 `127.0.0.1:8787`。

启动：

```bash
./scripts/start-local.sh
```

停止服务：

```bash
./scripts/stop-local.sh
```

启动后访问 <http://127.0.0.1:8787>。运行状态、服务器文件和配置会写入 `backend/data`；升级时请保留该目录。此包不附带 Codex CLI，请在本机单独安装并登录后，再在工作台设置中选择 `codex`。

需要授予 Codex 完整权限时，请先停止服务，再显式指定同一个原生 CLI：

```bash
./scripts/stop-local.sh
./scripts/start-local.sh --enable-codex-full-access --codex-command "$(command -v codex)"
```
EOF

# These files are only useful to Cloud, Agent, or Website deployments.
remove_managed_path "$output_dir" "$package_static/downloads"
remove_managed_path "$output_dir" "$package_static/website"
if [[ -d "$package_static/assets" ]]; then
  find "$package_static/assets" -maxdepth 1 -type f \( -name 'Cloud*' -o -name 'TerminalSessions*' \) -delete
fi
sed -i -E 's#<meta[[:space:]]+(property="og:image"|name="twitter:image")[^>]*>##g; s#/website/sculk-console-v2\.png##g' "$package_static/index.html"

tar -C "$staging_directory" -czf "$archive" "$release_name"

entries="$(tar -tzf "$archive")"
for required in \
  "$release_name/backend/target-local/release/backend" \
  "$release_name/frontend/dist/index.html" \
  "$release_name/scripts/start-local.sh" \
  "$release_name/scripts/stop-local.sh" \
  "$release_name/README.md"; do
  grep -Fqx -- "$required" <<<"$entries" >/dev/null || die "Archive validation failed; required entry is missing: $required"
done
for forbidden in \
  "$release_name/backend/data/state.json" \
  "$release_name/frontend/dist/downloads/" \
  "$release_name/frontend/dist/website/" \
  "$release_name/frontend/dist-cloud/" \
  "$release_name/frontend/dist-website/" \
  "$release_name/agent/"; do
  if grep -Fq -- "$forbidden" <<<"$entries"; then
    die "Archive validation failed; excluded content was included: $forbidden"
  fi
done
if grep -E "/frontend/dist/assets/(Cloud|TerminalSessions)" <<<"$entries" >/dev/null; then
  die 'Archive validation failed; Cloud UI chunks were included.'
fi

archive_name="${archive##*/}"
hash="$(sha256sum -- "$archive" | awk '{print $1}')"
printf '%s *%s\n' "$hash" "$archive_name" > "$checksum"
(
  cd -- "$output_dir"
  sha256sum --check --status "${checksum##*/}"
)

succeeded=true
printf 'Local Linux distribution archive: %s\nSHA256 file: %s\nSHA256: %s\n' "$archive" "$checksum" "$hash"
