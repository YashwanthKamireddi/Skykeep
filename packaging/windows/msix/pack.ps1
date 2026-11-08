# Skykeep's MSIX package for the Microsoft Store, from the program the .msi installs.
#
#   pack.ps1 -Payload <folder with skykeep.exe> -Out <file.msix> -Version 0.1.2 `
#            -Name <Package/Identity/Name> -Publisher <Package/Identity/Publisher> `
#            -PublisherDisplayName <Package/Properties/PublisherDisplayName>
#
# The three identity values are Partner Center's (the app → Product identity). The package is
# left unsigned: the Store signs what it publishes. Needs the Windows SDK (makeappx, makepri),
# which GitHub's Windows runners have.
#
# 1. The program, with its bundle mark set back to "unknown": taken from the .msi it says
#    "msi", and a copy that says neither .msi nor .exe is never one the in-app updater replaces
#    (src-tauri/src/updates.rs) — beside the package identity check that already rules it out.
# 2. The manifest (AppxManifest.xml) filled in.
# 3. The Start menu's and the Store's images, drawn from packaging/icons/skykeep-1024.png at
#    each scale and taskbar size Windows asks for, and indexed in resources.pri.
# 4. makeappx pack, which validates the manifest against Windows' schema as it packs.
param(
  [Parameter(Mandatory)] [string] $Payload,
  [Parameter(Mandatory)] [string] $Out,
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $Name,
  [Parameter(Mandatory)] [string] $Publisher,
  [Parameter(Mandatory)] [string] $PublisherDisplayName
)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$icon = Join-Path $here "..\..\icons\skykeep-1024.png" | Resolve-Path

# The newest SDK's tools.
$bin = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64\makeappx.exe" |
  Sort-Object { [version]($_.Directory.Parent.Name) } | Select-Object -Last 1
if (-not $bin) { throw "no Windows SDK makeappx.exe found" }
$makeappx = $bin.FullName
$makepri = Join-Path $bin.DirectoryName "makepri.exe"
"Windows SDK: $($bin.DirectoryName)"

# The Store takes a.b.c.0: the fourth part is its own.
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "version '$Version' is not major.minor.patch" }
$packageVersion = "$Version.0"

$stage = Join-Path ([System.IO.Path]::GetTempPath()) "skykeep-msix-$([guid]::NewGuid())"
New-Item -ItemType Directory $stage | Out-Null
try {
  # 1. The program and what lies beside it.
  Get-ChildItem $Payload -File | Where-Object { $_.Extension -ne ".msi" } | Copy-Item -Destination $stage
  $exe = Join-Path $stage "skykeep.exe"
  if (-not (Test-Path $exe)) { throw "no skykeep.exe in $Payload" }
  $bytes = [System.IO.File]::ReadAllBytes($exe)
  $latin = [System.Text.Encoding]::Latin1
  $text = $latin.GetString($bytes)
  $marks = 0
  foreach ($kind in "MSI", "NSS") {
    $from = "__TAURI_BUNDLE_TYPE_VAR_$kind"
    for ($at = $text.IndexOf($from); $at -ge 0; $at = $text.IndexOf($from, $at + 1)) {
      $latin.GetBytes("__TAURI_BUNDLE_TYPE_VAR_UNK").CopyTo($bytes, $at)
      $marks++
    }
  }
  if ($marks -eq 0 -and -not $text.Contains("__TAURI_BUNDLE_TYPE_VAR_UNK")) { throw "skykeep.exe has no bundle mark" }
  [System.IO.File]::WriteAllBytes($exe, $bytes)
  "program: skykeep.exe, bundle mark set to unknown ($marks replaced)"

  # 2. The manifest.
  $esc = { param($v) [System.Security.SecurityElement]::Escape($v) }
  $manifest = (Get-Content (Join-Path $here "AppxManifest.xml") -Raw).
    Replace("__IDENTITY_NAME__", (& $esc $Name)).
    Replace("__PUBLISHER_DISPLAY_NAME__", (& $esc $PublisherDisplayName)).
    Replace("__PUBLISHER__", (& $esc $Publisher)).
    Replace("__VERSION__", $packageVersion)
  if ($manifest -match "__[A-Z_]+__") { throw "the manifest still has $($Matches[0])" }
  [xml]$manifest | Out-Null
  Set-Content (Join-Path $stage "AppxManifest.xml") $manifest -Encoding utf8NoBOM

  # 3. The images: each icon centred on a transparent canvas, at `share` of its shorter side.
  Add-Type -AssemblyName System.Drawing
  $assets = Join-Path $stage "Assets"
  # Every image of one icon into `dir`: Skykeep's own into Assets, and each app's own into
  # Assets\<App>, named in the manifest by that app's entry.
  function Images([string]$from, [string]$dir) {
    New-Item -ItemType Directory $dir -Force | Out-Null
    $source = [System.Drawing.Image]::FromFile($from)
    function Draw([string]$file, [int]$w, [int]$h, [double]$share) {
      $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
      $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
      $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
      $g.Clear([System.Drawing.Color]::Transparent)
      $side = [math]::Round([math]::Min($w, $h) * $share)
      $g.DrawImage($source, [int](($w - $side) / 2), [int](($h - $side) / 2), [int]$side, [int]$side)
      $g.Dispose()
      $bmp.Save((Join-Path $dir $file), [System.Drawing.Imaging.ImageFormat]::Png)
      $bmp.Dispose()
    }
    $scales = 100, 125, 150, 200, 400
    # name, width, height at scale 100, and the icon's share of the shorter side.
    $tiles = @(
      @("Square44x44Logo", 44, 44, 1.0),
      @("Square71x71Logo", 71, 71, 0.75),
      @("Square150x150Logo", 150, 150, 0.66),
      @("Wide310x150Logo", 310, 150, 0.66),
      @("Square310x310Logo", 310, 310, 0.6),
      @("StoreLogo", 50, 50, 1.0)
    )
    foreach ($t in $tiles) {
      foreach ($s in $scales) {
        Draw "$($t[0]).scale-$s.png" ([math]::Round($t[1] * $s / 100)) ([math]::Round($t[2] * $s / 100)) $t[3]
      }
    }
    # The taskbar's, the Start menu list's and Explorer's: each pixel size, on no plate.
    foreach ($size in 16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256) {
      foreach ($form in "", "_altform-unplated", "_altform-lightunplated") {
        Draw "Square44x44Logo.targetsize-$size$form.png" $size $size 1.0
      }
    }
    $source.Dispose()
  }
  Images $icon $assets
  foreach ($app in "Notes", "Reminders", "Calendar", "Contacts", "Drive", "Photos") {
    $from = Join-Path $here "..\..\icons\apps\$($app.ToLower())-1024.png" | Resolve-Path
    Images $from (Join-Path $assets $app)
  }
  "images: $((Get-ChildItem $assets).Count) files"

  $pri = Join-Path ([System.IO.Path]::GetTempPath()) "skykeep-priconfig-$([guid]::NewGuid()).xml"
  & $makepri createconfig /cf $pri /dq en-US /pv 10.0.0 /o
  if ($LASTEXITCODE) { throw "makepri createconfig failed ($LASTEXITCODE)" }
  & $makepri new /pr $stage /cf $pri /mn (Join-Path $stage "AppxManifest.xml") /of (Join-Path $stage "resources.pri") /o
  if ($LASTEXITCODE) { throw "makepri new failed ($LASTEXITCODE)" }
  Remove-Item $pri

  # 4. The package.
  New-Item -ItemType Directory -Force (Split-Path $Out -Parent) | Out-Null
  & $makeappx pack /d $stage /p $Out /o
  if ($LASTEXITCODE) { throw "makeappx pack failed ($LASTEXITCODE)" }
  "packed: $Out ($([math]::Round((Get-Item $Out).Length / 1MB, 1)) MB), version $packageVersion"
} finally {
  Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}
