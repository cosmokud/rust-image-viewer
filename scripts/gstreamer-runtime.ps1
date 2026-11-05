# Shared by build-installers.ps1 (which stages the runtime) and generate-attribution.ps1
# (which reports what it ships): both must agree on where GStreamer lives and on which
# plugins are dropped, so neither keeps its own copy.

# The 1.28 plugin set carries six GPL plugins. A viewer needs none of them --
# src/video_player.rs builds playbin/decodebin plus the d3d12 decoder and never an
# encoder -- and an LGPL-only bundle is what can be redistributed inside an
# MIT-licensed installer, so Stage-GStreamerBundle copies everything except these.
$GStreamerExcludedPluginDlls = @(
    "gsta52dec.dll"   # a52dec   AC-3 decoder
    "gstdtsdec.dll"   # dtsdec   DTS decoder
    "gstdvdread.dll"  # dvdread  DVD source
    "gstresindvd.dll" # resindvd DVD playback
    "gstx264.dll"     # x264     H.264 encoder
    "gstx265.dll"     # x265     H.265 encoder
)

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

# Plugin details only, keyed by plugin DLL: `gst-inspect -a` repeats a plugin's section
# for every element it registers, and every output line is prefixed "<element>: ". Field
# values are column-aligned, so name and value are separated by two or more spaces.
# Plugins that register no elements (typefindfunctions, tracers, ...) print no section
# at all and are simply absent from the map.
function Get-GStreamerPluginCensus {
    param(
        [Parameter(Mandatory = $true)]
        [string]$GStreamerPrefix
    )

    $gstInspect = Join-Path $GStreamerPrefix "bin\gst-inspect-1.0.exe"
    if (-not (Test-Path -LiteralPath $gstInspect)) {
        throw "gst-inspect-1.0.exe is missing from the detected GStreamer prefix: $gstInspect"
    }

    # gst-inspect warns on stderr about plugins it cannot load, and a redirected native
    # stderr becomes a terminating error under $ErrorActionPreference = "Stop".
    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $lines = & $gstInspect -a 2>$null
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $previousPreference

    if ($exitCode -ne 0) {
        throw "gst-inspect-1.0 -a failed with exit code $exitCode (prefix: $GStreamerPrefix)"
    }

    $byDll = @{}
    $version = $null
    $current = $null
    $inDetails = $false

    $flush = {
        if ($current -and $current.ContainsKey("Name")) {
            $dll = ""
            if ($current.ContainsKey("Filename")) {
                $dll = [System.IO.Path]::GetFileName($current["Filename"])
            }
            # "(null)" is gst-inspect's Filename for a plugin linked into the core library
            # rather than a DLL of its own; the core attribution covers it and the bundle
            # has no separate file to list.
            if ($dll -and $dll -ne "(null)") {
                $license = ""
                if ($current.ContainsKey("License")) {
                    $license = $current["License"]
                }
                $module = ""
                if ($current.ContainsKey("Source module")) {
                    $module = $current["Source module"]
                }
                $byDll[$dll] = [pscustomobject]@{
                    Dll     = $dll
                    Name    = $current["Name"]
                    License = $license
                    Module  = $module
                }
            }
        }
    }

    foreach ($line in $lines) {
        $sep = $line.IndexOf(": ")
        if ($sep -lt 0) {
            continue
        }
        $body = $line.Substring($sep + 2)

        if ($body -eq "Plugin Details:") {
            $current = @{}
            $inDetails = $true
            continue
        }
        if (-not $inDetails) {
            continue
        }
        if ($body.Trim().Length -eq 0) {
            & $flush
            $current = $null
            $inDetails = $false
            continue
        }
        if ($body -match '^\s+(Name|Version|License|Filename|Source module)\s{2,}(\S.*?)\s*$') {
            $field = $Matches[1]
            $value = $Matches[2]
            $current[$field] = $value
            if ($field -eq "Version" -and -not $version) {
                $version = $value
            }
        }
    }
    & $flush

    return [pscustomobject]@{
        Version = $version
        ByDll   = $byDll
    }
}
