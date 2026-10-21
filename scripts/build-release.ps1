<#
.SYNOPSIS
    Build a portable Windows package from any git tag without touching the working tree.

.DESCRIPTION
    Creates a temporary git worktree at the given tag, installs dependencies, runs the
    Tauri release build, and writes a portable ZIP to <repo>/release.
    The current working tree, branch, and uncommitted changes are never modified.

.PARAMETER Tag
    Tag to build, for example v0.1.0 or v0.2.0. The tag must exist locally.

.PARAMETER WithInstaller
    Also build the NSIS installer. Requires the NSIS toolchain and WebView2 offline
    installer to be available (Tauri downloads and caches them on first use).

.PARAMETER OutputDir
    Optional output directory. Defaults to <repo>/release.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 v0.2.0

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 v0.1.0 -WithInstaller
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Tag,

    [switch]$WithInstaller,

    [string]$OutputDir
)

$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][scriptblock]$Action
    )
    Write-Host "==> $Label" -ForegroundColor Cyan
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed with exit code $LASTEXITCODE."
    }
}

function Normalize-Path {
    param([Parameter(Mandatory = $true)][string]$Path)
    $full = [System.IO.Path]::GetFullPath($Path)
    return $full.Replace('/', '\').TrimEnd('\')
}

$rootOutput = & git rev-parse --show-toplevel 2>$null
if ($LASTEXITCODE -ne 0 -or -not $rootOutput) {
    throw "Run this script inside the pic-deduplicate git repository."
}
$repoRoot = Normalize-Path ($rootOutput.Trim())

$commit = & git rev-parse --verify --quiet "refs/tags/$Tag^{commit}"
if ($LASTEXITCODE -ne 0 -or -not $commit) {
    throw "Tag '$Tag' was not found locally. Run 'git fetch --tags' or check the tag name."
}
$commit = $commit.Trim()

$safeTag = $Tag -replace '[^0-9A-Za-z._-]', '-'
if (-not $safeTag) {
    throw "Tag '$Tag' cannot be used as a file name."
}

$buildRoot = Join-Path $repoRoot ".build"
$worktreesRoot = Join-Path $buildRoot "worktrees"
$worktreePath = Normalize-Path (Join-Path $worktreesRoot $safeTag)
$cargoTarget = Normalize-Path (Join-Path $buildRoot "cargo-target")
$stagingDir = Normalize-Path (Join-Path $buildRoot "staging\PicDeduplicate-$safeTag")
$releaseRoot = if ($OutputDir) {
    Normalize-Path ([System.IO.Path]::GetFullPath($OutputDir))
} else {
    Normalize-Path (Join-Path $repoRoot "release")
}

New-Item -ItemType Directory -Force -Path $worktreesRoot, $cargoTarget, (Split-Path $stagingDir -Parent), $releaseRoot | Out-Null

$registered = @(
    & git worktree list --porcelain |
        Where-Object { $_ -like "worktree *" } |
        ForEach-Object { Normalize-Path ($_ -replace '^worktree\s+', '') }
)
if (Test-Path $worktreePath) {
    if ($registered -contains $worktreePath) {
        Invoke-Checked "Remove stale worktree $worktreePath" {
            git worktree remove --force $worktreePath
        }
        git worktree prune
    } else {
        throw "Directory already exists and is not a registered worktree: $worktreePath. Delete it manually and retry."
    }
}

$oldCargoTarget = $env:CARGO_TARGET_DIR
$worktreeAdded = $false

try {
    Invoke-Checked "Create worktree at $Tag ($($commit.Substring(0, 8)))" {
        git worktree add --detach $worktreePath "refs/tags/$Tag"
    }
    $worktreeAdded = $true

    $manifest = Get-Content (Join-Path $worktreePath "package.json") -Raw | ConvertFrom-Json
    Write-Host "==> Building version $($manifest.version) from tag $Tag"
    $tagVersion = $Tag.TrimStart('v', 'V')
    if ($manifest.version -ne $tagVersion) {
        Write-Warning "Tag '$Tag' does not match package.json version $($manifest.version)."
    }

    Push-Location $worktreePath
    try {
        Invoke-Checked "Install frontend dependencies" {
            npm ci --no-audit --no-fund
        }

        $env:CARGO_TARGET_DIR = $cargoTarget
        if ($WithInstaller) {
            Invoke-Checked "Build Tauri app and NSIS installer" {
                npm run tauri build -- --bundles nsis
            }
        } else {
            Invoke-Checked "Build Tauri app (portable)" {
                npm run tauri build -- --no-bundle
            }
        }
    } finally {
        if ($null -eq $oldCargoTarget) {
            Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_TARGET_DIR = $oldCargoTarget
        }
        Pop-Location
    }

    $exe = @(
        (Join-Path $cargoTarget "release\pic-deduplicate.exe"),
        (Join-Path $worktreePath "src-tauri\target\release\pic-deduplicate.exe")
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $exe) {
        throw "Build finished but pic-deduplicate.exe was not found under $cargoTarget\release."
    }

    if (Test-Path $stagingDir) {
        Remove-Item -Recurse -Force $stagingDir
    }
    New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null
    Copy-Item $exe (Join-Path $stagingDir "PicDeduplicate.exe")

    $zipPath = Join-Path $releaseRoot "PicDeduplicate-$safeTag-x64-portable.zip"
    if (Test-Path $zipPath) {
        Remove-Item -Force $zipPath
    }
    Compress-Archive -Path (Join-Path $stagingDir "*") -DestinationPath $zipPath -CompressionLevel Optimal
    Remove-Item -Recurse -Force $stagingDir

    Write-Host ""
    Write-Host "Portable package: $zipPath" -ForegroundColor Green

    if ($WithInstaller) {
        $nsisDirs = @(
            (Join-Path $cargoTarget "release\bundle\nsis"),
            (Join-Path $worktreePath "src-tauri\target\release\bundle\nsis")
        )
        $installer = $nsisDirs |
            Where-Object { Test-Path $_ } |
            ForEach-Object { Get-ChildItem $_ -Filter "*.exe" } |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if (-not $installer) {
            throw "NSIS output was not found under bundle\nsis."
        }
        $installerPath = Join-Path $releaseRoot "PicDeduplicate-$safeTag-x64-setup.exe"
        Copy-Item $installer.FullName $installerPath -Force
        Write-Host "NSIS installer:  $installerPath" -ForegroundColor Green
    }

    Write-Host ""
    Write-Host "The source working tree was not modified." -ForegroundColor DarkGray
} finally {
    if ($worktreeAdded) {
        Push-Location $repoRoot
        try {
            git worktree remove --force $worktreePath 2>$null
            git worktree prune
        } finally {
            Pop-Location
        }
    }
}
