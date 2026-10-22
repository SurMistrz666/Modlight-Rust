# Build Modlight for Windows: a release build of the app (its icon, version and
# manifest are embedded by crates/modlight-app/build.rs), DirectML.dll beside it (ONNX
# Runtime's DirectML provider for the background remover), the licences, and a zip of
# that folder. The Windows counterpart of build-app.sh; the script never starts the app.
#
#   app\scripts\build-app.ps1 [-OutDir <dir>]     (default: app\dist)
#
# Needs: the Rust toolchain (cargo on PATH or in %USERPROFILE%\.cargo\bin) and the
# MSVC build tools with a Windows SDK. Nothing outside the output directory and cargo's
# target directory is written. The folder runs as it is (no installer): settings live
# in Documents\Modlight, and "Start with Windows" and the Explorer menu entries point at
# wherever Modlight.exe is when they are switched on.

param(
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'

$appRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $OutDir) {
    $OutDir = Join-Path $appRoot 'dist'
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path

# The version the workspace builds, e.g. 0.7.7.
$versionLine = Select-String -Path (Join-Path $appRoot 'Cargo.toml') -Pattern '^version = "([^"]+)"$' |
    Select-Object -First 1
if (-not $versionLine) {
    throw "build-app: no version in $appRoot\Cargo.toml"
}
$version = $versionLine.Matches[0].Groups[1].Value

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) {
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (-not (Test-Path $cargo)) {
        throw 'build-app: cargo was not found (install Rust from https://rustup.rs)'
    }
}

Write-Host "build-app: cargo build --release ($version)"
# cargo reports its progress on standard error, which Windows PowerShell 5.1 turns into
# error records that 'Stop' would end the script on: judge it by its exit code alone.
$ErrorActionPreference = 'Continue'
& $cargo build --release -p modlight-app --manifest-path (Join-Path $appRoot 'Cargo.toml') 2>&1 |
    ForEach-Object { Write-Host "$_" }
$cargoExit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($cargoExit -ne 0) {
    throw "build-app: cargo build failed ($cargoExit)"
}

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $appRoot 'target' }
$release = Join-Path $targetDir 'release'
$binary = Join-Path $release 'Modlight.exe'
if (-not (Test-Path $binary)) {
    throw "build-app: $binary was not built"
}

# The machine the build is for, in the archive's name.
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'ARM64' { 'arm64' }
    default { 'x64' }
}
$triple = if ($arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }

# DirectML.dll: ort-sys copies it beside the build (copy-dylibs); otherwise it is in the
# folder ort-sys unpacked its verified download into.
$directMl = Join-Path $release 'DirectML.dll'
if (-not (Test-Path $directMl)) {
    $cache = Join-Path $env:LOCALAPPDATA "ort.pyke.io\dfbin\$triple"
    $found = Get-ChildItem -Path $cache -Filter 'DirectML.dll' -Recurse -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $found) {
        throw "build-app: DirectML.dll was not found beside the build or in $cache"
    }
    $directMl = $found.FullName
}

# --- The folder -------------------------------------------------------------------
$folder = Join-Path $OutDir 'Modlight'
if (Test-Path $folder) {
    Remove-Item -Recurse -Force -Path $folder
}
New-Item -ItemType Directory -Path $folder | Out-Null
Copy-Item -Path $binary -Destination $folder
Copy-Item -Path $directMl -Destination $folder
Copy-Item -Path (Join-Path $appRoot 'LICENSE.txt') -Destination $folder
Copy-Item -Path (Join-Path $appRoot 'Licenses') -Destination (Join-Path $folder 'Licenses') -Recurse

# --- The archive ------------------------------------------------------------------
$zip = Join-Path $OutDir "Modlight-$version-windows-$arch.zip"
if (Test-Path $zip) {
    Remove-Item -Force -Path $zip
}
Compress-Archive -Path (Join-Path $folder '*') -DestinationPath $zip

$hash = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
Write-Host "build-app: $folder"
Write-Host "build-app: $zip"
Write-Host "build-app: SHA-256 $hash"
