$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

# Packaging stays with this checkout: the generated packager config and `out-dir`
# name `target` under it, so a machine-local shared target dir is opted out of here.
$env:CARGO_TARGET_DIR = Join-Path $repoRoot "target"

# GStreamer prefix resolution and the GPL plugin exclusion list live here so the
# attribution generator reports exactly what this script stages.
. (Join-Path $repoRoot "scripts\gstreamer-runtime.ps1")

function Ensure-CargoPackager {
    if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
        Write-Host "cargo-packager not found. Installing with cargo..."
        cargo install cargo-packager --locked
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }

    $localAppData = [System.Environment]::GetFolderPath("LocalApplicationData")
    $packagerToolsRoot = Join-Path $localAppData ".cargo-packager"
    try {
        New-Item -ItemType Directory -Force -Path $packagerToolsRoot | Out-Null
    }
    catch {
        Write-Error @"
cargo-packager needs write access to its Windows tool cache:
$packagerToolsRoot

Windows denied access before installer generation could start. Run this script from
a normal, non-sandboxed PowerShell session for your user, or grant your current
user write access to Local AppData.

Original error:
$($_.Exception.Message)
"@
        exit 1
    }

    Write-Host "Using cargo-packager tool cache:"
    Write-Host $packagerToolsRoot
}

function Resolve-PackageInfo {
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or -not $metadata) {
        throw "Failed to read cargo metadata."
    }

    $package = $metadata.packages | Where-Object { $_.name -eq "rust-image-viewer" } | Select-Object -First 1
    if (-not $package) {
        $package = $metadata.packages | Select-Object -First 1
    }

    if (-not $package) {
        throw "No package found in cargo metadata."
    }

    return $package
}

function Build-ReleaseBinary {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,

        [Parameter(Mandatory = $true)]
        [string]$BinaryName
    )

    $binaryPath = Join-Path $RepoRoot ("target\release\{0}.exe" -f $BinaryName)
    Write-Host "Building release binary once for both installer variants..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    if (-not (Test-Path -LiteralPath $binaryPath)) {
        throw @"
Release build completed, but expected binary is missing:
$binaryPath

Expected after running:
cargo build --release
"@
    }

    Write-Host "Using release binary for both installers:"
    Write-Host (" - " + $binaryPath)
}

function Build-LegacyBinary {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,

        [Parameter(Mandatory = $true)]
        [string]$BinaryName
    )

    # The same profile, asked for the instruction set a machine below AVX2 has. `.cargo/config.toml`
    # carries `-C target-cpu=x86-64-v3` for every build in this tree, and `RUSTFLAGS` replaces that
    # list rather than adding to it, so this one variable is the whole of the difference — and a
    # target directory of its own is what keeps the two binaries from overwriting each other, since
    # cargo names the artifact after the profile and not after the flags.
    $binaryPath = Join-Path $RepoRoot ("target\legacy\release\{0}.exe" -f $BinaryName)
    Write-Host "Building the x86-64-v2 binary the legacy artifacts carry..."

    $previousRustFlags = $env:RUSTFLAGS
    $previousTargetDir = $env:CARGO_TARGET_DIR
    try {
        $env:RUSTFLAGS = "-C target-cpu=x86-64-v2"
        $env:CARGO_TARGET_DIR = Join-Path $RepoRoot "target\legacy"
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }
    finally {
        $env:RUSTFLAGS = $previousRustFlags
        $env:CARGO_TARGET_DIR = $previousTargetDir
    }

    if (-not (Test-Path -LiteralPath $binaryPath)) {
        throw @"
Legacy build completed, but expected binary is missing:
$binaryPath
"@
    }

    Write-Host "Using legacy (x86-64-v2) binary for the legacy installers and portable:"
    Write-Host (" - " + $binaryPath)
}

function Copy-MatchingFiles {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceDir,

        [Parameter(Mandatory = $true)]
        [string]$DestinationDir,

        [Parameter(Mandatory = $true)]
        [string[]]$Patterns,

        [string[]]$Exclude = @(),

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
            Where-Object { $Exclude -notcontains $_.Name } |
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

function Assert-GStreamerD3D12Runtime {
    param(
        [Parameter(Mandatory = $true)]
        [string]$GStreamerPrefix
    )

    $requiredFiles = @(
        "bin\gstd3d12-1.0-0.dll",
        "lib\gstreamer-1.0\gstd3d12.dll"
    )

    $missing = @()
    foreach ($relativePath in $requiredFiles) {
        $path = Join-Path $GStreamerPrefix $relativePath
        if (-not (Test-Path -LiteralPath $path)) {
            $missing += $relativePath
        }
    }

    if ($missing.Count -gt 0) {
        throw @"
Detected GStreamer prefix is missing D3D12 runtime files:
$($missing -join "`n")

Install a recent 64-bit MSVC GStreamer runtime with the D3D12 plugin enabled,
then rerun this script. The bundled-GStreamer installer needs these files for
[Performance] enable_d3d12 = true to work on machines without system GStreamer.
"@
    }
}

function Assert-StagedGStreamerBundle {
    param(
        [Parameter(Mandatory = $true)]
        [string]$PluginDir,

        [Parameter(Mandatory = $true)]
        [string]$BinDir
    )

    # What src/video_player.rs builds: playbin/decodebin over the d3d12 decoder, with the
    # pixel/audio converters and sinks behind it. The GPL exclusion must not take any of
    # these with it.
    $requiredFiles = @(
        "gstplayback.dll",
        "gsttypefindfunctions.dll",
        "gstcoreelements.dll",
        "gstapp.dll",
        "gstautodetect.dll",
        "gstaudioconvert.dll",
        "gstaudioresample.dll",
        "gstvideoconvertscale.dll",
        "gstd3d12.dll"
    )

    $problems = @()
    foreach ($name in $requiredFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $PluginDir $name))) {
            $problems += $name
        }
    }
    foreach ($name in $GStreamerExcludedPluginDlls) {
        if (Test-Path -LiteralPath (Join-Path $PluginDir $name)) {
            $problems += "$name (excluded GPL plugin was staged anyway)"
        }
    }
    foreach ($name in $GStreamerExcludedLibraryDlls) {
        if (Test-Path -LiteralPath (Join-Path $BinDir $name)) {
            $problems += "$name (excluded GPL library was staged anyway)"
        }
    }

    if ($problems.Count -gt 0) {
        throw @"
Staged GStreamer bundle does not hold the plugins the player needs:
$($problems -join "`n")

Check the exclusion list in scripts\gstreamer-runtime.ps1 and the installed runtime.
"@
    }
}

function Stage-GStreamerBundle {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BundleRoot
    )

    if (Test-Path -LiteralPath $BundleRoot) {
        Remove-Item -LiteralPath $BundleRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $BundleRoot -Force | Out-Null

    $gstPrefix = Resolve-GStreamerPrefix -CandidateRoot $null
    Assert-GStreamerD3D12Runtime -GStreamerPrefix $gstPrefix
    $gstStageRoot = Join-Path $BundleRoot "gstreamer"

    $pluginSourceDir = Join-Path $gstPrefix "lib\gstreamer-1.0"
    $stagedPluginDir = Join-Path $gstStageRoot "lib\gstreamer-1.0"

    $binSourceDir = Join-Path $gstPrefix "bin"
    $stagedBinDir = Join-Path $gstStageRoot "bin"

    $binCount = Copy-MatchingFiles -SourceDir $binSourceDir -DestinationDir $stagedBinDir -Patterns @("*.dll", "gst-plugin-scanner.exe") -Exclude $GStreamerExcludedLibraryDlls -Required
    # GPL plugins are left out of the copy (list and reasoning in
    # scripts\gstreamer-runtime.ps1): the player needs no encoders, and an LGPL-only
    # runtime is what an MIT-licensed installer can redistribute.
    $pluginCount = Copy-MatchingFiles -SourceDir $pluginSourceDir -DestinationDir $stagedPluginDir -Patterns @("*.dll") -Exclude $GStreamerExcludedPluginDlls -Required
    $scannerCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "libexec\gstreamer-1.0") -DestinationDir (Join-Path $gstStageRoot "libexec\gstreamer-1.0") -Patterns @("gst-plugin-scanner.exe")
    $schemaCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "share\glib-2.0\schemas") -DestinationDir (Join-Path $gstStageRoot "share\glib-2.0\schemas") -Patterns @("*.compiled", "*.xml")

    if ($scannerCount -eq 0 -and -not (Test-Path -LiteralPath (Join-Path $gstStageRoot "bin\gst-plugin-scanner.exe"))) {
        throw "gst-plugin-scanner.exe is missing from both libexec and bin locations."
    }

    $excludedCount = 0
    foreach ($excludedDll in $GStreamerExcludedPluginDlls) {
        if (Test-Path -LiteralPath (Join-Path $pluginSourceDir $excludedDll)) {
            $excludedCount += 1
        }
    }
    $excludedLibraryCount = 0
    foreach ($excludedDll in $GStreamerExcludedLibraryDlls) {
        if (Test-Path -LiteralPath (Join-Path $binSourceDir $excludedDll)) {
            $excludedLibraryCount += 1
        }
    }

    Assert-StagedGStreamerBundle -PluginDir $stagedPluginDir -BinDir $stagedBinDir

    Write-Host "Staged bundled GStreamer runtime:"
    Write-Host (" - Prefix: " + $gstPrefix)
    Write-Host (" - Bin files: " + $binCount + " (" + $excludedLibraryCount + " GPL libraries excluded)")
    Write-Host (" - Plugin files: " + $pluginCount + " (" + $excludedCount + " GPL plugins excluded)")
    Write-Host (" - Scanner files: " + $scannerCount)
    Write-Host (" - Schema files: " + $schemaCount)
}

function Write-PackagerConfig {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ConfigPath,

        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,

        [Parameter(Mandatory = $true)]
        [string]$OutputDir,

        [Parameter(Mandatory = $true)]
        [string]$PackageName,

        [Parameter(Mandatory = $true)]
        [string]$PackageVersion,

        [Parameter(Mandatory = $true)]
        [bool]$IncludeGStreamer,

        # Where the exe this variant installs is, which is the whole of what tells the AVX2
        # artifacts from the legacy ones: both are `rust-image-viewer.exe`, and only the
        # directory differs.
        [Parameter(Mandatory = $true)]
        [string]$BinariesDir
    )

    $repoRootAbs = [System.IO.Path]::GetFullPath($RepoRoot)
    $outputDirAbs = [System.IO.Path]::GetFullPath($OutputDir)
    $binariesDirAbs = [System.IO.Path]::GetFullPath($BinariesDir)
    $licensePathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "LICENSE"))
    $iconPathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "assets\icon.ico"))
    $nsisTemplatePathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "packaging\nsis\installer.nsi"))

    $tomlOutputDir = $outputDirAbs.Replace("\", "/")
    $tomlBinariesDir = $binariesDirAbs.Replace("\", "/")
    $tomlLicensePath = $licensePathAbs.Replace("\", "/")
    $tomlIconPath = $iconPathAbs.Replace("\", "/")
    $tomlNsisTemplatePath = $nsisTemplatePathAbs.Replace("\", "/")

    $resources = @()

    $lines = @(
        "name = ""$PackageName""",
        "version = ""$PackageVersion""",
        "product-name = ""Rust Image Viewer""",
        "identifier = ""dev.rust-image-viewer.app""",
        "publisher = ""Rust Image Viewer Contributors""",
        "description = ""High-performance image and video viewer for Windows 10/11""",
        "license-file = ""$tomlLicensePath""",
        "icons = [""$tomlIconPath""]",
        "formats = [""nsis""]",
        "out-dir = ""$tomlOutputDir""",
        "binaries-dir = ""$tomlBinariesDir""",
        "before-packaging-command = ""echo using the binary already in binaries-dir""",
        ""
    )

    if ($IncludeGStreamer) {
        $gstResourcePathAbs = [System.IO.Path]::GetFullPath((Join-Path $outputDirAbs ".with-gstreamer\gstreamer"))
        $tomlGstResourcePath = $gstResourcePathAbs.Replace("\", "/")
        $resources += "{ src = ""$tomlGstResourcePath"", target = ""gstreamer"" }"
    }

    # Notices installed beside the exe: THIRD-PARTY.md links into LICENSES/,
    # licenses-extra/ and NOTICES/, so all of them travel together. NOTICES/ carries the
    # NOTICE files dependencies ship (moka's Apache-2.0 files), which section 4(d) of that
    # license requires a redistributor to pass on; without it the installed manifest holds
    # a link that resolves to nothing. Skipped when absent, so packaging works on a
    # checkout that has not run generate-attribution.ps1.
    $noticeResources = @(
        @{ Path = (Join-Path $repoRootAbs "THIRD-PARTY.md"); Target = "THIRD-PARTY.md" },
        @{ Path = (Join-Path $repoRootAbs "LICENSE"); Target = "LICENSE" },
        @{ Path = (Join-Path $repoRootAbs "LICENSES"); Target = "LICENSES" },
        @{ Path = (Join-Path $repoRootAbs "licenses-extra"); Target = "licenses-extra" },
        @{ Path = (Join-Path $repoRootAbs "NOTICES"); Target = "NOTICES" }
    )
    foreach ($noticeResource in $noticeResources) {
        if (-not (Test-Path -LiteralPath $noticeResource.Path)) {
            continue
        }
        $resourcePath = $noticeResource.Path.Replace("\", "/")
        $resources += "{ src = ""$resourcePath"", target = ""$($noticeResource.Target)"" }"
    }

    if ($resources.Count -gt 0) {
        $lines += "resources = ["
        foreach ($resource in $resources) {
            $lines += "  $resource,"
        }
        $lines += "]"
        $lines += ""
    }

    $lines += @(
        "[[binaries]]",
        "path = ""rust-image-viewer""",
        "main = true",
        "",
        "[[file-associations]]",
        'extensions = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "psd", "ico", "tiff", "tif"]',
        'description = "Image File"',
        'name = "RustImageViewer.Image"',
        "",
        "[[file-associations]]",
        'extensions = ["mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "3gp", "ogv"]',
        'description = "Video File"',
        'name = "RustImageViewer.Video"',
        ""
    )

    $lines += @(
        "[nsis]",
        "installMode = ""currentUser""",
        "template = ""$tomlNsisTemplatePath""",
        'appdata-paths = ["$APPDATA/rust-image-viewer", "$LOCALAPPDATA/rust-image-viewer"]',
        "preinstall-section = '''",
        '; Keep current-user install path at AppData\Local\rust-image-viewer instead of product display name.',
        'Section PreInstallPathOverride',
        '  !if "${INSTALLMODE}" == "currentUser"',
        '    StrCmp $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}" 0 +2',
        '    StrCpy $INSTDIR "$LOCALAPPDATA\rust-image-viewer"',
        '  !endif',
        'SectionEnd',
        "'''"
    )

    $configDir = Split-Path -Parent $ConfigPath
    New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    Set-Content -LiteralPath $ConfigPath -Value ($lines -join [Environment]::NewLine) -Encoding utf8
}

function Invoke-Packager {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ConfigPath,

        [Parameter(Mandatory = $true)]
        [string]$OutputDir
    )

    cargo packager --config $ConfigPath --formats nsis
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

function Rename-PackagedInstaller {
    param(
        [Parameter(Mandatory = $true)]
        [string]$OutputDir,

        [Parameter(Mandatory = $true)]
        [string]$PackageName,

        [Parameter(Mandatory = $true)]
        [string]$PackageVersion,

        # What this one carries, appended to the name the packager gives every variant. The
        # packager names all four the same, so each run has to be renamed before the next writes
        # over it — and the run that keeps the bare `-setup.exe` name is the one made last.
        [string]$Suffix = ""
    )

    $escapedPackageName = [regex]::Escape($PackageName)
    $escapedVersion = [regex]::Escape($PackageVersion)

    $nsisFile = Get-ChildItem -LiteralPath $OutputDir -File |
        Where-Object { $_.Name -match "^$escapedPackageName`_$escapedVersion`_.*-setup\.exe$" } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $nsisFile -or -not $Suffix) {
        return
    }

    $renamedNsis = $nsisFile.Name -replace "-setup\.exe$", ("-setup{0}.exe" -f $Suffix)
    if ($renamedNsis -ne $nsisFile.Name) {
        Move-Item -LiteralPath $nsisFile.FullName -Destination (Join-Path $OutputDir $renamedNsis) -Force
    }

}

Ensure-CargoPackager

$package = Resolve-PackageInfo
$packageName = $package.name
$packageVersion = $package.version

# Two binaries, one program: the AVX2 build every artifact carries by default, and the
# x86-64-v2 build beside it for hardware below that. Both are built before anything is
# packaged, so the four installers below only compress what is already on disk.
Build-ReleaseBinary -RepoRoot $repoRoot -BinaryName "rust-image-viewer"
Build-LegacyBinary -RepoRoot $repoRoot -BinaryName "rust-image-viewer"

$outputDir = Join-Path $repoRoot "target\packager"
New-Item -ItemType Directory -Path $outputDir -Force | Out-Null

# Clean old artifacts for this version so each run outputs exactly four installers and the
# two portables.
$artifactPrefix = "{0}_{1}_" -f $packageName, $packageVersion
Get-ChildItem -LiteralPath $outputDir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name.StartsWith($artifactPrefix) -and $_.Extension -eq ".exe" } |
    Remove-Item -Force

$configDir = Join-Path $outputDir ".packager-configs"
$noGstreamerConfig = Join-Path $configDir "packager-no-gstreamer.toml"
$withGstreamerConfig = Join-Path $configDir "packager-with-gstreamer.toml"
$legacyNoGstreamerConfig = Join-Path $configDir "packager-no-gstreamer-legacy.toml"
$legacyWithGstreamerConfig = Join-Path $configDir "packager-with-gstreamer-legacy.toml"
$bundleRoot = Join-Path $outputDir ".with-gstreamer"
$releaseBinariesDir = Join-Path $repoRoot "target\release"
$legacyBinariesDir = Join-Path $repoRoot "target\legacy\release"

# Staged once: both bundled installers carry the same runtime, and only their own copy of the
# executable differs.
Stage-GStreamerBundle -BundleRoot $bundleRoot

Write-PackagerConfig -ConfigPath $noGstreamerConfig -RepoRoot $repoRoot -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -BinariesDir $releaseBinariesDir -IncludeGStreamer $false
Write-Host "Building no-GStreamer installer (NSIS)..."
Invoke-Packager -ConfigPath $noGstreamerConfig -OutputDir $outputDir
Rename-PackagedInstaller -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -Suffix "-no-gstreamer"

Write-PackagerConfig -ConfigPath $legacyNoGstreamerConfig -RepoRoot $repoRoot -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -BinariesDir $legacyBinariesDir -IncludeGStreamer $false
Write-Host "Building no-GStreamer legacy installer (NSIS)..."
Invoke-Packager -ConfigPath $legacyNoGstreamerConfig -OutputDir $outputDir
Rename-PackagedInstaller -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -Suffix "-no-gstreamer-legacy"

Write-PackagerConfig -ConfigPath $legacyWithGstreamerConfig -RepoRoot $repoRoot -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -BinariesDir $legacyBinariesDir -IncludeGStreamer $true
Write-Host "Building bundled-GStreamer legacy installer (NSIS)..."
Invoke-Packager -ConfigPath $legacyWithGstreamerConfig -OutputDir $outputDir
Rename-PackagedInstaller -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -Suffix "-legacy"

# Last, because this is the one that keeps the name the packager gives it: every earlier run
# wrote `-setup.exe` and was moved off it before this one starts.
Write-PackagerConfig -ConfigPath $withGstreamerConfig -RepoRoot $repoRoot -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -BinariesDir $releaseBinariesDir -IncludeGStreamer $true
Write-Host "Building bundled-GStreamer installer (NSIS)..."
Invoke-Packager -ConfigPath $withGstreamerConfig -OutputDir $outputDir

# The standalone binaries a release publishes beside the installers. The packager's own names
# say nothing about an instruction set, so both are copied out under the names a release uses.
$portables = @(
    @{ Source = (Join-Path $releaseBinariesDir "rust-image-viewer.exe"); Name = "rust-image-viewer.exe" },
    @{ Source = (Join-Path $legacyBinariesDir "rust-image-viewer.exe"); Name = "rust-image-viewer-legacy.exe" }
)
foreach ($portable in $portables) {
    if (-not (Test-Path -LiteralPath $portable.Source)) {
        throw "Expected binary is missing: $($portable.Source)"
    }
    Copy-Item -LiteralPath $portable.Source -Destination (Join-Path $outputDir $portable.Name) -Force
}

$resolvedOutputDir = [System.IO.Path]::GetFullPath($outputDir)
Write-Host ""
Write-Host "Installers generated in:"
Write-Host $resolvedOutputDir

$installers = Get-ChildItem -LiteralPath $resolvedOutputDir -File |
    Where-Object {
        $_.Extension -eq ".exe" -and
        $_.Name.StartsWith($artifactPrefix)
    } |
    Sort-Object Name

if (-not $installers) {
    Write-Warning "No .exe installers were found in the output directory."
    exit 0
}

$installers | ForEach-Object { Write-Host (" - " + $_.FullName) }
foreach ($portable in $portables) {
    $portablePath = Join-Path $resolvedOutputDir $portable.Name
    if (Test-Path -LiteralPath $portablePath) {
        Write-Host (" - " + $portablePath)
    }
}
