$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

. (Join-Path $repoRoot "scripts\gstreamer-runtime.ps1")

$manifestPath = Join-Path $repoRoot "THIRD-PARTY.md"
$sectionBegin = "<!-- gstreamer-runtime:begin -->"
$sectionEnd = "<!-- gstreamer-runtime:end -->"

# The runtime's own `License` field is a bare string, not an SPDX expression: this is how
# each one is attributed and which shipped text it points at. `BSD` names no variant and
# the three plugins that declare it do not say which, so both variants ship.
# The LGPL text lives outside LICENSES/ because cargo-tribute deletes any file there that
# is named after an SPDX id it does not itself use (verified: LGPL-3.0.txt is pruned,
# unknown names survive).
$gstreamerLicenseMap = @{
    "LGPL"    = @{ Spdx = "LGPL-2.1-or-later"; Texts = @("licenses-extra/LGPL-2.1-or-later.txt") }
    "MPL"     = @{ Spdx = "MPL-2.0"; Texts = @("LICENSES/MPL-2.0.txt") }
    "MPL-2.0" = @{ Spdx = "MPL-2.0"; Texts = @("LICENSES/MPL-2.0.txt") }
    "MIT/X11" = @{ Spdx = "MIT"; Texts = @("LICENSES/MIT.txt") }
    "BSD"     = @{ Spdx = "BSD-2-Clause OR BSD-3-Clause"; Texts = @("LICENSES/BSD-2-Clause.txt", "LICENSES/BSD-3-Clause.txt") }
}

function Get-GLibVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$GStreamerPrefix
    )

    $versionsFile = Join-Path $GStreamerPrefix "share\versions.txt"
    if (Test-Path -LiteralPath $versionsFile) {
        foreach ($line in Get-Content -LiteralPath $versionsFile) {
            if ($line -match '^glib\s+(\S+)\s*$') {
                return $Matches[1]
            }
        }
    }

    return "version not reported"
}

function Get-PluginGroups {
    param(
        [Parameter(Mandatory = $true)]
        [string]$GStreamerPrefix,

        [Parameter(Mandatory = $true)]
        [object]$Census
    )

    $pluginDir = Join-Path $GStreamerPrefix "lib\gstreamer-1.0"
    if (-not (Test-Path -LiteralPath $pluginDir)) {
        throw "GStreamer plugin directory is missing: $pluginDir"
    }

    # The staged bundle is this directory minus the exclusions, so the notices list it the
    # same way rather than from gst-inspect alone: plugins that register no elements print
    # no section there and would otherwise drop out of the notices entirely.
    $groups = @{}
    $dlls = Get-ChildItem -LiteralPath $pluginDir -Filter "*.dll" -File
    foreach ($file in $dlls) {
        $excluded = $false
        foreach ($excludedDll in $GStreamerExcludedPluginDlls) {
            if ($file.Name -ieq $excludedDll) {
                $excluded = $true
            }
        }
        if ($excluded) {
            continue
        }

        $entry = $Census.ByDll[$file.Name]
        if (-not $entry) {
            $license = "not reported"
            $label = $file.Name
        }
        else {
            $license = $entry.License
            $label = $entry.Name
        }
        if (-not $license) {
            $license = "not reported"
        }

        if (-not $groups.ContainsKey($license)) {
            $groups[$license] = New-Object System.Collections.ArrayList
        }
        [void]$groups[$license].Add($label)
    }

    $result = @()
    foreach ($license in $groups.Keys) {
        $result += [pscustomobject]@{
            License = $license
            Labels  = @($groups[$license] | Sort-Object)
        }
    }

    return $result
}

function New-GStreamerSection {
    param(
        [Parameter(Mandatory = $true)]
        [string]$GStreamerPrefix,

        [Parameter(Mandatory = $true)]
        [object]$Census
    )

    # A stale exclusion list would quietly stop dropping a plugin that went GPL.
    foreach ($excludedDll in $GStreamerExcludedPluginDlls) {
        $entry = $Census.ByDll[$excludedDll]
        if (-not $entry) {
            Write-Warning "Excluded plugin not in this runtime (exclusion is inert): $excludedDll"
            continue
        }
        if ($entry.License -ne "GPL") {
            throw "build-installers.ps1 excludes $excludedDll, but the runtime declares it '$($entry.License)'."
        }
    }

    $groups = @(Get-PluginGroups -GStreamerPrefix $GStreamerPrefix -Census $Census |
        Sort-Object -Property @{ Expression = { $_.Labels.Count }; Descending = $true }, @{ Expression = { $_.License }; Descending = $false })

    $version = $Census.Version
    if (-not $version) {
        $version = "version not reported"
    }
    $glibVersion = Get-GLibVersion -GStreamerPrefix $GStreamerPrefix

    $excludedNames = @()
    foreach ($excludedDll in $GStreamerExcludedPluginDlls) {
        $entry = $Census.ByDll[$excludedDll]
        if ($entry) {
            $excludedNames += $entry.Name
        }
    }

    $lines = @(
        $sectionBegin,
        "## GStreamer runtime (bundled installer)",
        "",
        "The bundled-GStreamer installer carries a GStreamer runtime under ``gstreamer\`` beside",
        "the executable, and the app prefers it over a system-wide install. Versions and plugin",
        "lists below are read from that runtime. Generated by ``generate-attribution.ps1``; do not edit.",
        "",
        "- [GStreamer $version](https://gstreamer.freedesktop.org/) -- ``LGPL-2.1-or-later`` -- [`licenses-extra/LGPL-2.1-or-later.txt`](licenses-extra/LGPL-2.1-or-later.txt)",
        "- [GLib $glibVersion](https://gitlab.gnome.org/GNOME/glib) -- ``LGPL-2.1-or-later`` -- [`licenses-extra/LGPL-2.1-or-later.txt`](licenses-extra/LGPL-2.1-or-later.txt)",
        ""
    )

    if ($excludedNames.Count -gt 0) {
        $quoted = ($excludedNames | ForEach-Object { "``$_``" }) -join ", "
        $lines += "The runtime's GPL plugins ($quoted) are not shipped:"
        $lines += "``build-installers.ps1`` copies every other plugin DLL. Each plugin below ships as"
        $lines += "one ``gst*.dll`` in ``gstreamer\lib\gstreamer-1.0\``, grouped by the license it"
        $lines += "declares to ``gst-inspect-1.0``."
        $lines += ""
    }

    $referencedTexts = @("licenses-extra/LGPL-2.1-or-later.txt")
    foreach ($group in $groups) {
        $count = $group.Labels.Count
        $lines += "### $($group.License) ($count)"
        $lines += ""

        $mapping = $gstreamerLicenseMap[$group.License]
        if ($mapping) {
            $textRefs = @()
            foreach ($text in $mapping.Texts) {
                $textRefs += "[``$text``]($text)"
                $referencedTexts += $text
            }
            $lines += "License: ``$($mapping.Spdx)`` -- $($textRefs -join ", ")"
        }
        elseif ($group.License -eq "not reported") {
            $lines += "No license is reported: these DLLs register no elements, so ``gst-inspect-1.0``"
            $lines += "prints neither their plugin details nor their license. Listed by file name."
        }
        else {
            $lines += "Upstream declares these as ``$($group.License)``, which names no license text;"
            $lines += "listed as declared so the source module can be traced upstream."
        }
        $lines += ""

        foreach ($label in $group.Labels) {
            $lines += "- $label"
        }
        $lines += ""
    }

    while ($lines.Count -gt 0 -and $lines[$lines.Count - 1] -eq "") {
        $lines = $lines[0..($lines.Count - 2)]
    }
    $lines += $sectionEnd

    $missing = @()
    foreach ($text in ($referencedTexts | Sort-Object -Unique)) {
        if (-not (Test-Path -LiteralPath (Join-Path $repoRoot $text))) {
            $missing += $text
        }
    }
    if ($missing.Count -gt 0) {
        throw "Missing license texts referenced by the GStreamer section: $($missing -join ", ")"
    }

    return $lines
}

function Write-ManifestSection {
    param(
        # not Mandatory: PowerShell 5.1 refuses to bind a mandatory [string[]] whose
        # blank separator lines are empty strings
        [string[]]$SectionLines
    )

    $existing = ""
    if (Test-Path -LiteralPath $manifestPath) {
        $existing = [System.IO.File]::ReadAllText($manifestPath)
    }

    # cargo-tribute rewrites the whole manifest, so its output stays untouched here: only
    # everything from our own marker onwards is replaced, which keeps reruns byte-identical.
    $cut = $existing.IndexOf($sectionBegin)
    if ($cut -ge 0) {
        $existing = $existing.Substring(0, $cut)
    }

    $text = $existing.TrimEnd() + "`n`n" + ($SectionLines -join "`n") + "`n"
    [System.IO.File]::WriteAllText($manifestPath, $text, (New-Object System.Text.UTF8Encoding($false)))
}

# LICENSES/ and the crate list in THIRD-PARTY.md are generated from the dependency tree
# rather than kept by hand, so this is what refreshes them along with the section for the
# GStreamer runtime the bundled installer ships: run it after anything changes Cargo.lock
# or that runtime, and commit what it writes.
if (-not (Get-Command cargo-tribute -ErrorAction SilentlyContinue)) {
    Write-Host "cargo-tribute not found. Installing with cargo..."
    cargo install cargo-tribute --locked
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Write-Host "Generating third-party attribution..."
cargo tribute
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Host "Generating GStreamer runtime attribution..."
$gstPrefix = Resolve-GStreamerPrefix -CandidateRoot $null
$census = Get-GStreamerPluginCensus -GStreamerPrefix $gstPrefix
$section = New-GStreamerSection -GStreamerPrefix $gstPrefix -Census $census
Write-ManifestSection -SectionLines $section

$changed = git status --short -- LICENSES THIRD-PARTY.md

if ($changed) {
    Write-Host ""
    Write-Host "Changed, and to be committed:"
    $changed | ForEach-Object { Write-Host (" " + $_) }
}
else {
    Write-Host "Already current; nothing to commit."
}
