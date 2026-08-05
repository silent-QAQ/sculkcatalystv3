# SPDX-License-Identifier: Apache-2.0

<#
.SYNOPSIS
Builds a Windows x86_64 local deployment ZIP.

.DESCRIPTION
Builds the native release backend and a local-mode frontend bundle in an
isolated temporary workspace below the package output directory. The source
frontend dependencies, source frontend dist directories, and source Rust
target directories are never modified. Cloud, Website, Agent and user-data
artifacts are not copied into the archive.

.PARAMETER Version
Version embedded in the archive name. When omitted, the backend Cargo package
version is used.

.PARAMETER OutputDirectory
Directory below artifacts that receives the ZIP and its SHA-256 sidecar. The
default is artifacts/generated/local.

.PARAMETER RefreshDependencies
Runs npm ci in the isolated frontend workspace. This is the default behavior;
the switch is retained for compatibility with existing automation.

.PARAMETER SkipDependencyInstall
Copies the existing frontend node_modules directory into the isolated
workspace instead of running npm ci. The source directory is read-only.
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [string]$OutputDirectory = '',
    [switch]$RefreshDependencies,
    [switch]$SkipDependencyInstall
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($RefreshDependencies -and $SkipDependencyInstall) {
    throw '-RefreshDependencies and -SkipDependencyInstall cannot be used together.'
}

if ([Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw 'package-local.ps1 must run on Windows. Build the Linux archive with scripts/package-local.sh on Linux.'
}
$nativeArchitecture = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITEW6432', 'Process')
if ([string]::IsNullOrWhiteSpace($nativeArchitecture)) {
    $nativeArchitecture = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITECTURE', 'Process')
}
if ($nativeArchitecture -notin @('AMD64', 'x86_64')) {
    throw "Only Windows x86_64 local distribution archives are supported (detected $nativeArchitecture)."
}

function Get-FullPath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw 'A required path is empty.'
    }
    return [System.IO.Path]::GetFullPath($Path)
}

function Assert-ChildPath([string]$BasePath, [string]$CandidatePath, [string]$Label) {
    $base = Get-FullPath $BasePath
    $candidate = Get-FullPath $CandidatePath
    $prefix = $base.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must stay below $base."
    }
    return $candidate
}

function Remove-ManagedPath([string]$BasePath, [string]$Path) {
    $fullPath = Assert-ChildPath $BasePath $Path 'Managed cleanup path'
    if (Test-Path -LiteralPath $fullPath) {
        Remove-Item -LiteralPath $fullPath -Recurse -Force
    }
}

function Get-ApplicationPath([string[]]$Names) {
    foreach ($name in $Names) {
        # GitHub-hosted Windows runners can expose the Node setup cache and the
        # system Node installation under the same command name. Select exactly
        # one application path instead of coercing the collection to a string.
        $command = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -ne $command) {
            return $command.Source
        }
    }
    throw "Required executable was not found: $($Names -join ', ')"
}

function Invoke-External([string]$FilePath, [string[]]$Arguments, [string]$WorkingDirectory) {
    Push-Location -LiteralPath $WorkingDirectory
    try {
        & $FilePath @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($Arguments -join ' ')"
        }
    } finally {
        Pop-Location
    }
}

function Get-BackendVersion([string]$CargoPath, [string]$ManifestPath) {
    $metadataOutput = & $CargoPath 'metadata' '--locked' '--no-deps' '--format-version' '1' '--manifest-path' $ManifestPath | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to read the backend package version from Cargo metadata (exit code $LASTEXITCODE)."
    }
    try {
        $metadata = $metadataOutput | ConvertFrom-Json
    } catch {
        throw "Cargo metadata did not return valid JSON: $($_.Exception.Message)"
    }
    $backendPackage = @($metadata.packages | Where-Object { $_.name -eq 'backend' }) | Select-Object -First 1
    if ($null -eq $backendPackage -or [string]::IsNullOrWhiteSpace([string]$backendPackage.version)) {
        throw 'The backend package version is missing from Cargo metadata.'
    }
    return [string]$backendPackage.version
}

function Assert-PackageVersion([string]$Candidate) {
    $value = if ($null -eq $Candidate) { '' } else { $Candidate.Trim() }
    if ([string]::IsNullOrWhiteSpace($value) -or $value -notmatch '^[A-Za-z0-9][A-Za-z0-9._+-]*$') {
        throw 'Version must be a non-empty filename-safe value (letters, digits, ., _, +, and -).'
    }
    return $value
}

function Remove-PreviousArchives([string]$Directory, [string]$BaseName, [string]$ArchiveExtension) {
    $escapedBase = [regex]::Escape($BaseName)
    $escapedExtension = [regex]::Escape($ArchiveExtension)
    $archivePattern = '^' + $escapedBase + '(?:-[A-Za-z0-9][A-Za-z0-9._+-]*)?' + $escapedExtension + '(?:\.sha256)?$'
    $stagingPattern = '^\.' + $escapedBase + '(?:-[A-Za-z0-9][A-Za-z0-9._+-]*)?\.staging$'
    Get-ChildItem -LiteralPath $Directory -Force -ErrorAction SilentlyContinue | ForEach-Object {
        if ($_.Name -match $archivePattern -or $_.Name -match $stagingPattern) {
            Remove-ManagedPath $Directory $_.FullName
        }
    }
}

function Enter-PackageLock([string]$Path) {
    try {
        return [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::OpenOrCreate,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
    } catch [System.IO.IOException] {
        throw "Another local package build is already running for this workspace. Wait for it to finish and retry."
    }
}

function Copy-DirectoryContents([string]$Source, [string]$Destination) {
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Get-ChildItem -LiteralPath $Source -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $Destination -Recurse -Force
    }
}

function Copy-FrontendInputs([string]$Source, [string]$Destination) {
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    $excludedNames = @('node_modules', 'dist', 'dist-cloud', 'dist-website')
    Get-ChildItem -LiteralPath $Source -Force |
        Where-Object { $_.Name -notin $excludedNames -and $_.Name -notlike '.env*' } |
        ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $Destination -Recurse -Force }
}

function Write-PackageReadme([string]$Path) {
    $content = @'
# Sculk Catalyst V3 本地部署

这是仅供本机使用的部署包，服务默认只监听 `127.0.0.1:8787`。

在 PowerShell 中启动：

```powershell
.\scripts\start-local.ps1
```

停止服务：

```powershell
.\scripts\stop-local.ps1
```

启动后访问 <http://127.0.0.1:8787>。运行状态、服务器文件和配置会写入 `backend\data`；升级时请保留该目录。此包不附带 Codex CLI，请在本机单独安装并登录后，再在工作台设置中选择 `codex.cmd`。

需要授予 Codex 完整权限时，请先停止服务，再显式指定同一个原生 CLI：

```powershell
.\scripts\stop-local.ps1
$codexCommand = (Get-Command codex.cmd -CommandType Application).Path
.\scripts\start-local.ps1 -EnableCodexFullAccess -CodexCommand $codexCommand
```
'@
    Set-Content -LiteralPath $Path -Value $content -Encoding UTF8
}

$root = Get-FullPath (Split-Path $PSScriptRoot -Parent)
$artifactsRoot = Get-FullPath (Join-Path $root 'artifacts')
$backendSource = Get-FullPath (Join-Path $root 'backend')
$frontendSource = Get-FullPath (Join-Path $root 'frontend')
$sourceNodeModules = Join-Path $frontendSource 'node_modules'
$backendManifest = Get-FullPath (Join-Path $backendSource 'Cargo.toml')
$cargo = Get-ApplicationPath @('cargo.exe', 'cargo')
$npm = Get-ApplicationPath @('npm.cmd', 'npm.exe', 'npm')

$packageVersion = if ([string]::IsNullOrWhiteSpace($Version)) {
    Get-BackendVersion $cargo $backendManifest
} else {
    $Version
}
$packageVersion = Assert-PackageVersion $packageVersion

$requestedOutput = if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    Join-Path $artifactsRoot 'generated\local'
} elseif ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory
} else {
    Join-Path $root $OutputDirectory
}
$outputRoot = Assert-ChildPath $artifactsRoot $requestedOutput 'OutputDirectory'
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
if ((Get-Item -LiteralPath $outputRoot -Force).Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
    throw "OutputDirectory must not be a reparse point: $outputRoot"
}
$packageLockDirectory = Get-FullPath (Join-Path $artifactsRoot 'generated')
New-Item -ItemType Directory -Force -Path $packageLockDirectory | Out-Null
$packageLockPath = Join-Path $packageLockDirectory '.sculk-catalyst-local-package.lock'

$releaseBaseName = 'sculk-catalyst-local-windows-x86_64'
$releaseName = "$releaseBaseName-$packageVersion"
$stagingDirectory = Join-Path $outputRoot ".${releaseName}.staging"
$packageDirectory = Join-Path $stagingDirectory $releaseName
$buildDirectory = Join-Path $stagingDirectory '.build'
$backendTargetDirectory = Join-Path $buildDirectory 'cargo-target'
$frontendBuildSource = Join-Path $buildDirectory 'frontend'
$backendBinary = Join-Path $backendTargetDirectory 'release\backend.exe'
$staticDirectory = Join-Path $frontendBuildSource 'dist-package-local'
$archive = Join-Path $outputRoot "${releaseName}.zip"
$checksum = "$archive.sha256"

$packageLock = Enter-PackageLock $packageLockPath
$succeeded = $false
try {
    # Package outputs and the isolated temporary workspace are disposable and owned by this script.
    Remove-PreviousArchives $outputRoot $releaseBaseName '.zip'
    Remove-ManagedPath $outputRoot $stagingDirectory
    Remove-ManagedPath $outputRoot $archive
    Remove-ManagedPath $outputRoot $checksum

    Copy-FrontendInputs $frontendSource $frontendBuildSource
    if ($SkipDependencyInstall) {
        if (-not (Test-Path -LiteralPath $sourceNodeModules -PathType Container)) {
            throw "Frontend dependencies are missing: $sourceNodeModules"
        }
        Copy-DirectoryContents $sourceNodeModules (Join-Path $frontendBuildSource 'node_modules')
    } else {
        Invoke-External $npm @('ci') $frontendBuildSource
    }

    $previousCargoTarget = [Environment]::GetEnvironmentVariable('CARGO_TARGET_DIR', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $backendTargetDirectory, 'Process')
        Invoke-External $cargo @('build', '--release', '--locked') $backendSource
    } finally {
        [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $previousCargoTarget, 'Process')
    }

    $previousAppMode = [Environment]::GetEnvironmentVariable('VITE_APP_MODE', 'Process')
    try {
        [Environment]::SetEnvironmentVariable('VITE_APP_MODE', 'local', 'Process')
        Invoke-External $npm @('run', 'build', '--', '--outDir', 'dist-package-local') $frontendBuildSource
    } finally {
        [Environment]::SetEnvironmentVariable('VITE_APP_MODE', $previousAppMode, 'Process')
    }

    if (-not (Test-Path -LiteralPath $backendBinary -PathType Leaf)) {
        throw "Native release backend was not produced: $backendBinary"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $staticDirectory 'index.html') -PathType Leaf)) {
        throw "Dedicated local frontend bundle was not produced: $staticDirectory"
    }

    $packageBackend = Join-Path $packageDirectory 'backend\target-local\release'
    $packageFrontend = Join-Path $packageDirectory 'frontend'
    $packageStatic = Join-Path $packageFrontend 'dist'
    $packageScripts = Join-Path $packageDirectory 'scripts'
    New-Item -ItemType Directory -Force -Path $packageBackend, $packageStatic, $packageScripts | Out-Null
    Copy-Item -LiteralPath $backendBinary -Destination (Join-Path $packageBackend 'backend.exe') -Force
    Copy-Item -Path (Join-Path $staticDirectory '*') -Destination $packageStatic -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $root 'scripts\start-local.ps1') -Destination (Join-Path $packageScripts 'start-local.ps1') -Force
    Copy-Item -LiteralPath (Join-Path $root 'scripts\stop-local.ps1') -Destination (Join-Path $packageScripts 'stop-local.ps1') -Force
    Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $packageDirectory 'LICENSE') -Force
    Copy-Item -LiteralPath (Join-Path $root 'NOTICE') -Destination (Join-Path $packageDirectory 'NOTICE') -Force
    Copy-Item -LiteralPath (Join-Path $root 'LICENSES') -Destination (Join-Path $packageDirectory 'LICENSES') -Recurse -Force
    New-Item -ItemType Directory -Force -Path (Join-Path $packageDirectory 'backend\data') | Out-Null
    Write-PackageReadme (Join-Path $packageDirectory 'README.md')

    # These files are only useful to Cloud, Agent, or Website deployments.
    Remove-ManagedPath $outputRoot (Join-Path $packageStatic 'downloads')
    Remove-ManagedPath $outputRoot (Join-Path $packageStatic 'website')
    $assetDirectory = Join-Path $packageStatic 'assets'
    if (Test-Path -LiteralPath $assetDirectory -PathType Container) {
        Get-ChildItem -LiteralPath $assetDirectory -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^(Cloud|TerminalSessions)' } |
            ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }
    }
    $indexPath = Join-Path $packageStatic 'index.html'
    $indexHtml = Get-Content -LiteralPath $indexPath -Raw
    $indexHtml = [regex]::Replace($indexHtml, '\s*<meta\s+(?:property="og:image"|name="twitter:image")[^>]*>', '')
    $indexHtml = $indexHtml.Replace('/website/sculk-console-v2.png', '')
    Set-Content -LiteralPath $indexPath -Value $indexHtml -Encoding UTF8

    Compress-Archive -LiteralPath $packageDirectory -DestinationPath $archive -CompressionLevel Optimal -Force

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($archive)
    try {
        $entries = @($zip.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
        $required = @(
            "$releaseName/backend/target-local/release/backend.exe",
            "$releaseName/frontend/dist/index.html",
            "$releaseName/scripts/start-local.ps1",
            "$releaseName/scripts/stop-local.ps1",
            "$releaseName/README.md"
        )
        foreach ($entry in $required) {
            if ($entries -notcontains $entry) {
                throw "Archive validation failed; required entry is missing: $entry"
            }
        }
        $forbidden = @(
            "$releaseName/backend/data/state.json",
            "$releaseName/frontend/dist/downloads/",
            "$releaseName/frontend/dist/website/",
            "$releaseName/frontend/dist-cloud/",
            "$releaseName/frontend/dist-website/",
            "$releaseName/agent/",
            "$releaseName/frontend/dist/assets/Cloud",
            "$releaseName/frontend/dist/assets/TerminalSessions"
        )
        foreach ($prefix in $forbidden) {
            if ($entries | Where-Object { $_.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase) }) {
                throw "Archive validation failed; excluded content was included: $prefix"
            }
        }
    } finally {
        $zip.Dispose()
    }

    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    $archiveFileName = [System.IO.Path]::GetFileName($archive)
    Set-Content -LiteralPath $checksum -Value "$hash *$archiveFileName" -Encoding ASCII
    $succeeded = $true
    Write-Host "Local Windows distribution archive: $archive"
    Write-Host "SHA256 file: $checksum"
    Write-Host "SHA256: $hash"
} finally {
    try {
        Remove-ManagedPath $outputRoot $stagingDirectory
        if (-not $succeeded) {
            Remove-ManagedPath $outputRoot $archive
            Remove-ManagedPath $outputRoot $checksum
        }
    } finally {
        $packageLock.Dispose()
    }
}
