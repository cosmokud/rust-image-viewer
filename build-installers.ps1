$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

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

function Resolve-GStreamerPrefix {
    param(
        [string]$CandidateRoot
    )

    $roots = @()
    if ($CandidateRoot) {
        $roots += $CandidateRoot
    }
    if ($env:GSTREAMER_1_0_ROOT_X86_64) {
        $roots += $env:GSTREAMER_1_0_ROOT_X86_64
    }
    if ($env:GSTREAMER_1_0_ROOT_MSVC_X86_64) {
        $roots += $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
    }

    foreach ($root in $roots) {
        try {
            $resolved = (Resolve-Path -LiteralPath $root).Path
            if (Test-Path -LiteralPath (Join-Path $resolved "bin\gstreamer-1.0-0.dll")) {
                return $resolved
            }
        }
        catch {
        }
    }

    $gstInspectCmd = Get-Command "gst-inspect-1.0.exe" -ErrorAction SilentlyContinue
    if (-not $gstInspectCmd) {
        throw "gst-inspect-1.0.exe was not found on PATH. Install/setup GStreamer first."
    }

    $gstBinDir = Split-Path -Parent $gstInspectCmd.Source
    $gstPrefix = Split-Path -Parent $gstBinDir
    if (-not (Test-Path -LiteralPath (Join-Path $gstPrefix "bin\gstreamer-1.0-0.dll"))) {
        throw "Detected GStreamer prefix is missing bin\gstreamer-1.0-0.dll: $gstPrefix"
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
    $gstStageRoot = Join-Path $BundleRoot "gstreamer"

    $binCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "bin") -DestinationDir (Join-Path $gstStageRoot "bin") -Patterns @("*.dll", "gst-plugin-scanner.exe") -Required
    $pluginCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "lib\gstreamer-1.0") -DestinationDir (Join-Path $gstStageRoot "lib\gstreamer-1.0") -Patterns @("*.dll") -Required
    $scannerCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "libexec\gstreamer-1.0") -DestinationDir (Join-Path $gstStageRoot "libexec\gstreamer-1.0") -Patterns @("gst-plugin-scanner.exe")
    $schemaCount = Copy-MatchingFiles -SourceDir (Join-Path $gstPrefix "share\glib-2.0\schemas") -DestinationDir (Join-Path $gstStageRoot "share\glib-2.0\schemas") -Patterns @("*.compiled", "*.xml")

    if ($scannerCount -eq 0 -and -not (Test-Path -LiteralPath (Join-Path $gstStageRoot "bin\gst-plugin-scanner.exe"))) {
        throw "gst-plugin-scanner.exe is missing from both libexec and bin locations."
    }

    Write-Host "Staged bundled GStreamer runtime:"
    Write-Host (" - Prefix: " + $gstPrefix)
    Write-Host (" - Bin files: " + $binCount)
    Write-Host (" - Plugin files: " + $pluginCount)
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
        [bool]$IncludeGStreamer
    )

    $repoRootAbs = [System.IO.Path]::GetFullPath($RepoRoot)
    $outputDirAbs = [System.IO.Path]::GetFullPath($OutputDir)
    $releaseDirAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "target\release"))
    $licensePathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "LICENSE"))
    $iconPathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "assets\icon.ico"))
    $nsisTemplatePathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "packaging\nsis\installer.nsi"))
    $wixTemplatePathAbs = [System.IO.Path]::GetFullPath((Join-Path $repoRootAbs "packaging\wix\main.wxs"))

    $tomlOutputDir = $outputDirAbs.Replace("\", "/")
    $tomlReleaseDir = $releaseDirAbs.Replace("\", "/")
    $tomlLicensePath = $licensePathAbs.Replace("\", "/")
    $tomlIconPath = $iconPathAbs.Replace("\", "/")
    $tomlNsisTemplatePath = $nsisTemplatePathAbs.Replace("\", "/")
    $tomlWixTemplatePath = $wixTemplatePathAbs.Replace("\", "/")

    $lines = @(
        "name = ""$PackageName""",
        "version = ""$PackageVersion""",
        "product-name = ""Rust Image Viewer""",
        "identifier = ""dev.rust-image-viewer.app""",
        "publisher = ""Rust Image Viewer Contributors""",
        "description = ""High-performance image and video viewer for Windows 10/11""",
        "license-file = ""$tomlLicensePath""",
        "icons = [""$tomlIconPath""]",
        "formats = [""nsis"", ""wix""]",
        "out-dir = ""$tomlOutputDir""",
        "binaries-dir = ""$tomlReleaseDir""",
        "before-packaging-command = ""cargo build --release""",
        ""
    )

    if ($IncludeGStreamer) {
        $gstResourcePathAbs = [System.IO.Path]::GetFullPath((Join-Path $outputDirAbs ".with-gstreamer\gstreamer"))
        $tomlGstResourcePath = $gstResourcePathAbs.Replace("\", "/")
        $lines += "resources = [{ src = ""$tomlGstResourcePath"", target = ""gstreamer"" }]"
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
        "'''",
        "",
        "[wix]",
        "template = ""$tomlWixTemplatePath""",
        "languages = [""en-US""]",
        ""
    )

    $configDir = Split-Path -Parent $ConfigPath
    New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    Set-Content -LiteralPath $ConfigPath -Value ($lines -join [Environment]::NewLine) -Encoding utf8
}

function Invoke-PackagerFormat {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ConfigPath,

        [Parameter(Mandatory = $true)]
        [string]$Format,

        [Parameter(Mandatory = $true)]
        [string]$OutputDir,

        [Parameter(Mandatory = $true)]
        [string]$PackageName,

        [Parameter(Mandatory = $true)]
        [string]$PackageVersion
    )

    cargo packager --release --config $ConfigPath --formats $Format
    if ($LASTEXITCODE -ne 0) {
        if ($Format -eq "wix") {
            $wixToolsRoot = Join-Path ([System.Environment]::GetFolderPath("LocalApplicationData")) ".cargo-packager\WixTools"
            $lightExe = Join-Path $wixToolsRoot "light.exe"
            $wixUtilExtension = Join-Path $wixToolsRoot "WixUtilExtension.dll"
            $wixUiExtension = Join-Path $wixToolsRoot "WixUIExtension.dll"
            $wixIntermediates = Join-Path $OutputDir ".cargo-packager\wix\x64"
            $localeFile = Join-Path $wixIntermediates "locale.wxl"
            $wixObject = Join-Path $wixIntermediates "main.wixobj"
            $fallbackMsi = Join-Path $wixIntermediates "output-no-validation.msi"

            if (
                (Test-Path -LiteralPath $lightExe) -and
                (Test-Path -LiteralPath $wixUtilExtension) -and
                (Test-Path -LiteralPath $wixUiExtension) -and
                (Test-Path -LiteralPath $localeFile) -and
                (Test-Path -LiteralPath $wixObject)
            ) {
                Write-Warning "cargo-packager WiX step failed. Trying fallback WiX link with -sval (skip ICE validation)."
                if (Test-Path -LiteralPath $fallbackMsi) {
                    Remove-Item -LiteralPath $fallbackMsi -Force
                }

                Push-Location $wixIntermediates
                & $lightExe -sval -ext $wixUtilExtension -ext $wixUiExtension -o $fallbackMsi -cultures:en-us -loc $localeFile "*.wixobj"
                $lightExitCode = $LASTEXITCODE
                Pop-Location

                if ($lightExitCode -eq 0 -and (Test-Path -LiteralPath $fallbackMsi)) {
                    $expectedMsiName = "{0}_{1}_x64_en-US.msi" -f $PackageName, $PackageVersion
                    $expectedMsiPath = Join-Path $OutputDir $expectedMsiName
                    Move-Item -LiteralPath $fallbackMsi -Destination $expectedMsiPath -Force
                    Write-Warning "WiX fallback succeeded and produced: $expectedMsiPath"
                    return
                }
            }
        }

        exit $LASTEXITCODE
    }
}

function Rename-NoGstreamerArtifacts {
    param(
        [Parameter(Mandatory = $true)]
        [string]$OutputDir,

        [Parameter(Mandatory = $true)]
        [string]$PackageName,

        [Parameter(Mandatory = $true)]
        [string]$PackageVersion
    )

    $escapedPackageName = [regex]::Escape($PackageName)
    $escapedVersion = [regex]::Escape($PackageVersion)

    $nsisFile = Get-ChildItem -LiteralPath $OutputDir -File |
        Where-Object { $_.Name -match "^$escapedPackageName`_$escapedVersion`_.*-setup\.exe$" } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($nsisFile) {
        $renamedNsis = $nsisFile.Name -replace "-setup\.exe$", "-setup-no-gstreamer.exe"
        if ($renamedNsis -ne $nsisFile.Name) {
            Move-Item -LiteralPath $nsisFile.FullName -Destination (Join-Path $OutputDir $renamedNsis) -Force
        }
    }

    $msiFile = Get-ChildItem -LiteralPath $OutputDir -File |
        Where-Object { $_.Name -match "^$escapedPackageName`_$escapedVersion`_.*\.msi$" } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($msiFile) {
        $renamedMsi = if ($msiFile.Name -match "_en-US\.msi$") {
            $msiFile.Name -replace "_en-US\.msi$", "_no_gstreamer_en-US.msi"
        } else {
            $msiFile.BaseName + "_no_gstreamer.msi"
        }
        Move-Item -LiteralPath $msiFile.FullName -Destination (Join-Path $OutputDir $renamedMsi) -Force
    }
}

Ensure-CargoPackager

$package = Resolve-PackageInfo
$packageName = $package.name
$packageVersion = $package.version

$outputDir = Join-Path $repoRoot "target\packager"
New-Item -ItemType Directory -Path $outputDir -Force | Out-Null

# Clean old artifacts for this version so each run outputs exactly four installers.
$artifactPrefix = "{0}_{1}_" -f $packageName, $packageVersion
Get-ChildItem -LiteralPath $outputDir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name.StartsWith($artifactPrefix) -and $_.Extension -in ".exe", ".msi" } |
    Remove-Item -Force

$configDir = Join-Path $outputDir ".packager-configs"
$noGstreamerConfig = Join-Path $configDir "packager-no-gstreamer.toml"
$withGstreamerConfig = Join-Path $configDir "packager-with-gstreamer.toml"
$bundleRoot = Join-Path $outputDir ".with-gstreamer"

Write-PackagerConfig -ConfigPath $noGstreamerConfig -RepoRoot $repoRoot -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -IncludeGStreamer $false
Write-Host "Building no-GStreamer installers (NSIS + WiX)..."
Invoke-PackagerFormat -ConfigPath $noGstreamerConfig -Format "nsis" -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion
Invoke-PackagerFormat -ConfigPath $noGstreamerConfig -Format "wix" -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion
Rename-NoGstreamerArtifacts -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion

Stage-GStreamerBundle -BundleRoot $bundleRoot
Write-PackagerConfig -ConfigPath $withGstreamerConfig -RepoRoot $repoRoot -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion -IncludeGStreamer $true
Write-Host "Building bundled-GStreamer installers (NSIS + WiX)..."
Invoke-PackagerFormat -ConfigPath $withGstreamerConfig -Format "nsis" -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion
Invoke-PackagerFormat -ConfigPath $withGstreamerConfig -Format "wix" -OutputDir $outputDir -PackageName $packageName -PackageVersion $packageVersion

$resolvedOutputDir = [System.IO.Path]::GetFullPath($outputDir)
Write-Host ""
Write-Host "Installers generated in:"
Write-Host $resolvedOutputDir

$installers = Get-ChildItem -LiteralPath $resolvedOutputDir -File |
    Where-Object {
        $_.Extension -in ".exe", ".msi" -and
        $_.Name.StartsWith($artifactPrefix)
    } |
    Sort-Object Name

if (-not $installers) {
    Write-Warning "No .exe or .msi installers were found in the output directory."
    exit 0
}

$installers | ForEach-Object { Write-Host (" - " + $_.FullName) }
