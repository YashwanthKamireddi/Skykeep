$ErrorActionPreference = 'Stop'

# The release's own per-user installer, checked against the SHA256 published with it
# (SHA256SUMS.txt on the release page).
$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  url64bit       = 'https://github.com/YashwanthKamireddi/Skykeep/releases/download/v0.1.2/Skykeep_0.1.2_x64-setup.exe'
  checksum64     = 'e70c1473a1f2c4ef198725d6e4c68d1c73835e5a4f53f6f120a23401c47ab536'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
  softwareName   = 'Skykeep'
}

Install-ChocolateyPackage @packageArgs
