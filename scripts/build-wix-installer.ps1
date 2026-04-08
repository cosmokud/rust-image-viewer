param(
    [string]$ProductVersion,

    [string]$Configuration = "release",
    [string]$WorkspaceRoot,
    [string]$OutputMsi,
    [string]$GStreamerRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Ensure-Wix7 {
    $dotnetTools = Join-Path $env:USERPROFILE ".dotnet\\tools"
    if (Test-Path -LiteralPath $dotnetTools) {
        $env:PATH = "$dotnetTools;$env:PATH"
    }

    $wixCmd = Get-Command "wix" -ErrorAction SilentlyContinue
    if (-not $wixCmd) {
        $hasNugetOrg = dotnet nuget list source | Select-String -SimpleMatch "nuget.org"
        if (-not $hasNugetOrg) {
            dotnet nuget add source https://api.nuget.org/v3/index.json -n nuget.org | Out-Null
        }

        dotnet tool install --global wix --version 7.0.0 | Out-Null
        $env:PATH = "$dotnetTools;$env:PATH"
        $wixCmd = Get-Command "wix" -ErrorAction SilentlyContinue
    }

    if (-not $wixCmd) {
        throw "WiX Toolset v7 CLI was not found after installation attempt."
    }

    $versionText = & wix --version
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to query WiX version."
    }

    if ($versionText -notmatch '^7\.') {
        dotnet tool update --global wix --version 7.0.0 | Out-Null
        $env:PATH = "$dotnetTools;$env:PATH"

        $versionText = & wix --version
        if ($LASTEXITCODE -ne 0 -or $versionText -notmatch '^7\.') {
            throw "WiX v7 is required. Found: $versionText"
        }
    }

    & wix eula accept wix7 --acceptEula yes | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to accept WiX v7 EULA non-interactively."
    }
}

function Normalize-MsiProductVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Version
    )

    $trimmed = $Version.Trim()
    if ($trimmed -notmatch '^(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)(?:[-+].*)?$') {
        throw "Version '$Version' is not a supported semantic version (expected Major.Minor.Patch)."
    }

    $major = [int]$Matches['major']
    $minor = [int]$Matches['minor']
    $patch = [int]$Matches['patch']

    if ($major -gt 255 -or $minor -gt 255 -or $patch -gt 65535) {
        throw "MSI ProductVersion parts are out of range for '$Version' (major<=255, minor<=255, patch<=65535)."
    }

    return "$major.$minor.$patch"
}

function Resolve-ProductVersion {
    param(
        [string]$WorkspacePath,
        [string]$RequestedVersion
    )

    if ($RequestedVersion -and $RequestedVersion.Trim().Length -gt 0) {
        return Normalize-MsiProductVersion -Version $RequestedVersion
    }

    $cargoToml = Join-Path $WorkspacePath "Cargo.toml"
    if (-not (Test-Path -LiteralPath $cargoToml)) {
        throw "Cargo.toml not found at $cargoToml"
    }

    $version = $null
    foreach ($line in Get-Content -LiteralPath $cargoToml) {
        if ($line -match '^\s*version\s*=\s*"([^"]+)"\s*$') {
            $version = $Matches[1]
            break
        }
    }

    if (-not $version) {
        throw "Could not find package version in $cargoToml"
    }

    return Normalize-MsiProductVersion -Version $version
}

function Resolve-GStreamerPrefix {
    param(
        [string]$CandidateRoot
    )

    if ($CandidateRoot) {
        $resolvedCandidate = (Resolve-Path -LiteralPath $CandidateRoot).Path
        if (-not (Test-Path -LiteralPath (Join-Path $resolvedCandidate "bin\\gstreamer-1.0-0.dll"))) {
            throw "Provided GStreamerRoot does not contain bin\\gstreamer-1.0-0.dll: $resolvedCandidate"
        }
        return $resolvedCandidate
    }

    $gstInspectCmd = Get-Command "gst-inspect-1.0.exe" -ErrorAction SilentlyContinue
    if (-not $gstInspectCmd) {
        throw "gst-inspect-1.0.exe was not found on PATH. Run setup-gstreamer first."
    }

    $gstBinDir = Split-Path -Parent $gstInspectCmd.Source
    $gstPrefix = Split-Path -Parent $gstBinDir

    if (-not (Test-Path -LiteralPath (Join-Path $gstPrefix "bin\\gstreamer-1.0-0.dll"))) {
        throw "Detected GStreamer prefix is missing bin\\gstreamer-1.0-0.dll: $gstPrefix"
    }

    return $gstPrefix
}

function Copy-MatchingFiles {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceDir,

        [Parameter(Mandatory = $true)]
        [string]$DestinationDir,

        [Parameter(Mandatory = $true)]
        [string[]]$Patterns,

        [switch]$Required
    )

    if (-not (Test-Path -LiteralPath $SourceDir)) {
        if ($Required) {
            throw "Required source directory is missing: $SourceDir"
        }
        return 0
    }

    New-Item -ItemType Directory -Path $DestinationDir -Force | Out-Null

    $copied = 0
    foreach ($pattern in $Patterns) {
        Get-ChildItem -LiteralPath $SourceDir -Filter $pattern -File -ErrorAction SilentlyContinue |
            ForEach-Object {
                Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $DestinationDir $_.Name) -Force
                $copied += 1
            }
    }

    if ($Required -and $copied -eq 0) {
        throw "No files matched required patterns in $SourceDir"
    }

    return $copied
}

if (-not $WorkspaceRoot) {
    $WorkspaceRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
} else {
    $WorkspaceRoot = (Resolve-Path -LiteralPath $WorkspaceRoot).Path
}

$ProductVersion = Resolve-ProductVersion -WorkspacePath $WorkspaceRoot -RequestedVersion $ProductVersion
Write-Host "Using product version: $ProductVersion"

$exePath = Join-Path $WorkspaceRoot "target\\$Configuration\\rust-image-viewer.exe"
if (-not (Test-Path -LiteralPath $exePath)) {
    throw "Executable not found at $exePath. Build the release executable first."
}

$wixSource = Join-Path $WorkspaceRoot "wix\\main.wxs"

if (-not (Test-Path -LiteralPath $wixSource)) {
    throw "Missing WiX source file: $wixSource"
}

Ensure-Wix7

$stagingDir = Join-Path $WorkspaceRoot "target\\wix\\payload"

if (-not $OutputMsi) {
    $OutputMsi = Join-Path $WorkspaceRoot "target\\wix\\rust-image-viewer-$ProductVersion.msi"
}

if (Test-Path -LiteralPath $stagingDir) {
    Remove-Item -LiteralPath $stagingDir -Recurse -Force
}
New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null

Copy-Item -LiteralPath $exePath -Destination (Join-Path $stagingDir "rust-image-viewer.exe") -Force

$gstPrefix = Resolve-GStreamerPrefix -CandidateRoot $GStreamerRoot
$gstStageRoot = Join-Path $stagingDir "gstreamer"

$binCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "bin") -DestinationDir (Join-Path $gstStageRoot "bin") -Patterns @("*.dll", "gst-plugin-scanner.exe") -Required
$pluginCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "lib\\gstreamer-1.0") -DestinationDir (Join-Path $gstStageRoot "lib\\gstreamer-1.0") -Patterns @("*.dll") -Required
$scannerCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "libexec\\gstreamer-1.0") -DestinationDir (Join-Path $gstStageRoot "libexec\\gstreamer-1.0") -Patterns @("gst-plugin-scanner.exe")

# Include GLib schemas when present to keep plugin initialization robust.
$schemaCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "share\\glib-2.0\\schemas") -DestinationDir (Join-Path $gstStageRoot "share\\glib-2.0\\schemas") -Patterns @("*.compiled", "*.xml")

if ($scannerCount -eq 0 -and -not (Test-Path -LiteralPath (Join-Path $gstStageRoot "bin\\gst-plugin-scanner.exe"))) {
    throw "gst-plugin-scanner.exe is missing from both libexec and bin locations."
}

$msiOutputDir = Split-Path -Parent $OutputMsi
New-Item -ItemType Directory -Path $msiOutputDir -Force | Out-Null

$wixArgs = @(
    "build",
    "--acceptEula",
    "yes",
    "-d",
    "ProductVersion=$ProductVersion",
    "-d",
    "PayloadDir=$stagingDir",
    "-o",
    $OutputMsi,
    $wixSource
)

& wix @wixArgs

if ($LASTEXITCODE -ne 0) {
    throw "wix build failed with exit code $LASTEXITCODE"
}

Write-Host "Built installer: $OutputMsi"
Write-Host "Staged GStreamer files: bin=$binCount plugin=$pluginCount scanner=$scannerCount schemas=$schemaCount"

if ($env:GITHUB_OUTPUT) {
    Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "msi_path=$OutputMsi"
}
