# Build Modlight for Windows: a release build of the app (its icon, version and
# manifest are embedded by crates/modlight-app/build.rs), DirectML.dll beside it (ONNX
# Runtime's DirectML provider for the background remover), the licences, and a zip of
# that folder, Modlight-<version>-<build>-windows-<arch>.zip, which is published to the
# server's Downloads when a server and a publish token are configured. The Windows
# counterpart of build-app.sh; the script never starts the app. Windows PowerShell 5.1 and
# PowerShell 7.
#
#   app\scripts\build-app.ps1 [-OutDir <dir>]      build (default output: app\dist)
#   app\scripts\build-app.ps1 -Publish <zip>       publish a zip it made before, again
#
# The build is the commit (git rev-parse --short=8 HEAD, empty outside git) and the time
# it was built (UTC); both go into the app (MODLIGHT_BUILD, MODLIGHT_BUILT, read by
# crates/modlight-core/build.rs) and into the zip's name and its description beside it
# (<zip name>.json), so the app knows it is exactly the build the server lists.
#
# Publishing (app\docs\WINDOWS.md, "Updates and publishing"):
#   MODLIGHT_PUBLISH_SERVER   the server, e.g. https://files.example.com
#   MODLIGHT_PUBLISH_TOKEN    a publish token from the server's Downloads (never printed)
#   or both in %USERPROFILE%\.config\modlight\publish.json ({"server": "...", "token":
#   "..."}), readable only by you (the usual for files in your profile)
#   MODLIGHT_PUBLISH=0        build without publishing
#   MODLIGHT_PUBLISH_NOTE     what changed, shown to people with the update
# A failed publish does not fail the build; it says how to try again.
#
# Needs: the Rust toolchain (cargo on PATH or in %USERPROFILE%\.cargo\bin) and the
# MSVC build tools with a Windows SDK; git for the commit. Nothing outside the output
# directory and cargo's target directory is written. The folder runs as it is (no
# installer): settings live in Documents\Modlight, and "Start with Windows" and the
# Explorer menu entries point at wherever Modlight.exe is when they are switched on.

param(
    [string]$OutDir,
    [string]$Publish
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest's progress display slows Windows PowerShell 5.1's uploads to a crawl.
$ProgressPreference = 'SilentlyContinue'

$appRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

# --- Publishing -------------------------------------------------------------------
# POST /api/downloads/upload with the zip's description, PUT each chunk of at most the
# server's chunkSize (50 MiB), POST .../finish; Authorization: Bearer <publish token>.

$MaxChunk = 52428800

function Get-Sha256([string]$Path) {
    (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Read-Json([string]$Path) {
    [IO.File]::ReadAllText($Path) | ConvertFrom-Json
}

function Write-Utf8([string]$Path, [string]$Text) {
    # UTF-8 without a byte order mark (Set-Content -Encoding UTF8 writes one in 5.1).
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding $false))
}

# Whether only you (and SYSTEM and Administrators, as every file in a profile) can read
# the file: Windows' counterpart of the Mac's mode 600.
function Test-PrivateFile([string]$Path) {
    $me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $allowed = @($me, 'S-1-5-18', 'S-1-5-32-544')
    foreach ($rule in (Get-Acl -LiteralPath $Path).Access) {
        if ($rule.AccessControlType -ne 'Allow') { continue }
        try {
            $sid = $rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
        } catch {
            $sid = $rule.IdentityReference.Value
        }
        if ($allowed -notcontains $sid) { return $false }
    }
    return $true
}

# The server and the token, or $null when none is configured. Throws when what is
# configured cannot be used.
function Get-PublishSettings {
    $server = $env:MODLIGHT_PUBLISH_SERVER
    $token = $env:MODLIGHT_PUBLISH_TOKEN
    $config = Join-Path $HOME '.config\modlight\publish.json'
    if (-not $server -and -not $token -and (Test-Path -LiteralPath $config -PathType Leaf)) {
        if (-not (Test-PrivateFile $config)) {
            throw "$config can be read by other accounts; allow only your own account to read it"
        }
        $settings = Read-Json $config
        $server = [string]$settings.server
        $token = [string]$settings.token
    }
    if (-not $server -or -not $token) { return $null }
    $server = $server.TrimEnd('/')
    if ($server -notmatch '^https://.+' -and
        $server -notmatch '^http://(127\.0\.0\.1|localhost|\[::1\])(:\d+)?$') {
        throw 'the publish server must be https:// (plain http only on this computer)'
    }
    if ($token -notmatch '^[A-Za-z0-9._~+/=-]+$') {
        throw 'the publish token has characters a token never has'
    }
    return @{ Server = $server; Token = $token }
}

# How a failed request reads: the HTTP status (or that nothing answered) and what the
# server said. Never the request's headers.
function Get-Failure($ErrorRecord) {
    $status = 0
    $response = $ErrorRecord.Exception.Response
    if ($response) {
        try { $status = [int]$response.StatusCode } catch { $status = 0 }
    }
    $said = ''
    if ($ErrorRecord.ErrorDetails -and $ErrorRecord.ErrorDetails.Message) {
        $said = $ErrorRecord.ErrorDetails.Message
        if ($said.Length -gt 300) { $said = $said.Substring(0, 300) }
    }
    if ($status -eq 0) {
        $what = "failed ($($ErrorRecord.Exception.Message))"
    } else {
        $what = "answered $status"
    }
    if ($said) { $what = "$what; the server said: $said" }
    return $what
}

# One request to the server's Downloads with the publish token. A redirect is not followed
# (the token would go along) and, like any answer that is not a success, is an error.
function Invoke-Downloads([hashtable]$Settings, [string]$Method, [string]$Path, $Body, [string]$ContentType) {
    $arguments = @{
        Uri = "$($Settings.Server)$Path"
        Method = $Method
        Headers = @{ Authorization = "Bearer $($Settings.Token)" }
        UseBasicParsing = $true
        MaximumRedirection = 0
        TimeoutSec = 600
    }
    if ($null -ne $Body) {
        $arguments.Body = $Body
        $arguments.ContentType = $ContentType
    }
    $response = Invoke-WebRequest @arguments
    $status = [int]$response.StatusCode
    if ($status -lt 200 -or $status -ge 300) {
        throw "the server answered $status"
    }
    return $response
}

function Write-PublishFailure([string]$Why, [string]$Zip) {
    Write-Host "build-app: not published: $Why"
    Write-Host 'build-app: the build itself is fine; to publish it again:'
    Write-Host "    `$env:MODLIGHT_PUBLISH_SERVER = '...'; `$env:MODLIGHT_PUBLISH_TOKEN = '...'"
    Write-Host "    & '$PSCommandPath' -Publish '$Zip'"
    Write-Host '    (or, with %USERPROFILE%\.config\modlight\publish.json in place, only the second line)'
}

# Publish a zip and its description (<zip without .zip>.json): $true when it was published
# or publishing is off or not configured, $false when it failed.
function Publish-Build([string]$Zip) {
    if ($env:MODLIGHT_PUBLISH -eq '0') {
        Write-Host 'build-app: not published (MODLIGHT_PUBLISH=0)'
        return $true
    }
    try {
        $settings = Get-PublishSettings
    } catch {
        Write-PublishFailure $_.Exception.Message $Zip
        return $false
    }
    if (-not $settings) {
        Write-Host 'build-app: not published (no MODLIGHT_PUBLISH_SERVER and MODLIGHT_PUBLISH_TOKEN, no publish.json)'
        return $true
    }
    $descriptionPath = [IO.Path]::ChangeExtension($Zip, '.json')
    if (-not (Test-Path -LiteralPath $Zip -PathType Leaf) -or
        -not (Test-Path -LiteralPath $descriptionPath -PathType Leaf)) {
        Write-PublishFailure "$Zip or its description $descriptionPath is missing" $Zip
        return $false
    }
    $description = Read-Json $descriptionPath
    $size = (Get-Item -LiteralPath $Zip).Length
    if ([long]$description.size -ne $size -or [string]$description.sha256 -ne (Get-Sha256 $Zip)) {
        Write-PublishFailure "$Zip is not the file its description describes" $Zip
        return $false
    }
    $note = if ($env:MODLIGHT_PUBLISH_NOTE) { $env:MODLIGHT_PUBLISH_NOTE } else { '' }
    $body = [ordered]@{
        platform = [string]$description.platform
        arch = [string]$description.arch
        version = [string]$description.version
        build = [string]$description.build
        built = [string]$description.built
        size = [long]$description.size
        sha256 = [string]$description.sha256
        filename = [string]$description.filename
        note = $note
    } | ConvertTo-Json -Compress
    Write-Host "build-app: publishing $(Split-Path -Leaf $Zip) to $($settings.Server)"
    # Windows PowerShell 5.1 does not offer TLS 1.2 by itself everywhere.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    try {
        $started = Invoke-Downloads $settings 'Post' '/api/downloads/upload' ([Text.Encoding]::UTF8.GetBytes($body)) 'application/json; charset=utf-8'
    } catch {
        Write-PublishFailure "starting the upload $(Get-Failure $_)" $Zip
        return $false
    }
    $answer = $started.Content | ConvertFrom-Json
    $upload = [string]$answer.upload
    $chunkSize = [long]$answer.chunkSize
    if ($upload -notmatch '^[A-Za-z0-9_-]+$' -or $chunkSize -le 0) {
        Write-PublishFailure 'the server named no upload or no chunk size' $Zip
        return $false
    }
    if ($chunkSize -gt $MaxChunk) { $chunkSize = $MaxChunk }
    $chunks = [long][Math]::Ceiling($size / [double]$chunkSize)
    $stream = [IO.File]::OpenRead($Zip)
    try {
        for ($n = 0; $n -lt $chunks; $n++) {
            $length = [int][Math]::Min($chunkSize, $size - $n * $chunkSize)
            $buffer = New-Object byte[] $length
            $read = 0
            while ($read -lt $length) {
                $got = $stream.Read($buffer, $read, $length - $read)
                if ($got -le 0) {
                    Write-PublishFailure "$Zip ended before its size" $Zip
                    return $false
                }
                $read += $got
            }
            for ($attempt = 1; ; $attempt++) {
                try {
                    Invoke-Downloads $settings 'Put' "/api/downloads/upload/$upload/$n" $buffer 'application/octet-stream' | Out-Null
                    break
                } catch {
                    if ($attempt -ge 3) {
                        Write-PublishFailure "chunk $($n + 1) of $chunks $(Get-Failure $_)" $Zip
                        return $false
                    }
                    Start-Sleep -Seconds 2
                }
            }
            Write-Host "build-app: sent $($n + 1) of $chunks"
        }
    } finally {
        $stream.Dispose()
    }
    try {
        $finished = Invoke-Downloads $settings 'Post' "/api/downloads/upload/$upload/finish" $null $null
    } catch {
        Write-PublishFailure "finishing the upload $(Get-Failure $_)" $Zip
        return $false
    }
    $published = $finished.Content | ConvertFrom-Json
    Write-Host "build-app: published as $($published.id) ($($published.url))"
    return $true
}

if ($Publish) {
    $zipPath = (Resolve-Path -LiteralPath $Publish).Path
    try {
        $published = Publish-Build $zipPath
    } catch {
        Write-PublishFailure $_.Exception.Message $zipPath
        $published = $false
    }
    if ($published) { exit 0 } else { exit 1 }
}

# --- The build ----------------------------------------------------------------------

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

# The build: the commit and the time, the same in the app and in what is published.
$build = ''
$git = (Get-Command git -ErrorAction SilentlyContinue).Source
if ($git) {
    $ErrorActionPreference = 'Continue'
    $commit = & $git -C $appRoot rev-parse --short=8 HEAD 2>$null
    $gitExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($gitExit -eq 0 -and $commit) { $build = ([string]$commit).Trim() }
}
$built = [DateTime]::UtcNow.ToString("yyyy'-'MM'-'dd'T'HH':'mm':'ss'Z'", [Globalization.CultureInfo]::InvariantCulture)
$buildLabel = if ($build) { $build } else { 'outside git' }

Write-Host "build-app: cargo build --release ($version, build $buildLabel, $built)"
# cargo reports its progress on standard error, which Windows PowerShell 5.1 turns into
# error records that 'Stop' would end the script on: judge it by its exit code alone. The
# build's identity is given to cargo only; this session's environment is put back after.
$savedBuild = $env:MODLIGHT_BUILD
$savedBuilt = $env:MODLIGHT_BUILT
$env:MODLIGHT_BUILD = $build
$env:MODLIGHT_BUILT = $built
$ErrorActionPreference = 'Continue'
try {
    & $cargo build --release -p modlight-app --manifest-path (Join-Path $appRoot 'Cargo.toml') 2>&1 |
        ForEach-Object { Write-Host "$_" }
    $cargoExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = 'Stop'
    $env:MODLIGHT_BUILD = $savedBuild
    $env:MODLIGHT_BUILT = $savedBuilt
}
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

# --- The archive and its description -------------------------------------------------
# The folder's files at the top of the zip, written entry by entry with '/' between names
# (Windows PowerShell 5.1's Compress-Archive writes '\', which other tools read as part of
# a name). The app unpacks this zip to update itself.
$zipName = "Modlight-$version-$build-windows-$arch.zip"
$zip = Join-Path $OutDir $zipName
$descriptionPath = [IO.Path]::ChangeExtension($zip, '.json')
foreach ($old in @($zip, $descriptionPath)) {
    if (Test-Path -LiteralPath $old) {
        Remove-Item -Force -LiteralPath $old
    }
}
Add-Type -AssemblyName System.IO.Compression
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
} catch {
    # PowerShell 7 has the zip types loaded already.
}
$archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
try {
    $root = (Get-Item -LiteralPath $folder).FullName.TrimEnd('\')
    foreach ($file in Get-ChildItem -LiteralPath $folder -Recurse -File) {
        $entry = $file.FullName.Substring($root.Length + 1).Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive, $file.FullName, $entry, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally {
    $archive.Dispose()
}

$size = (Get-Item -LiteralPath $zip).Length
$hash = Get-Sha256 $zip
$description = [ordered]@{
    platform = 'windows'
    arch = $arch
    version = $version
    build = $build
    built = $built
    size = [long]$size
    sha256 = $hash
    filename = $zipName
    note = ''
} | ConvertTo-Json -Compress
Write-Utf8 $descriptionPath $description

Write-Host "build-app: $folder"
Write-Host "build-app: $zip"
Write-Host "build-app: $size bytes, SHA-256 $hash"

# A failed publish says how to try again and leaves the build as it is.
try {
    [void](Publish-Build $zip)
} catch {
    Write-PublishFailure $_.Exception.Message $zip
}
